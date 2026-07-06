use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use beatbox_core::{ErrorBody, ExecuteRequest, ExecutionResult, JobRecord, JobStatus};
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone)]
pub struct JobStore {
    conn: Arc<Mutex<Connection>>,
}

pub struct CreatedJob {
    pub job_id: String,
    pub inserted: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CancelOutcome {
    Canceled,
    AlreadyCanceled,
    NotCancelable(JobStatus),
    Missing,
}

impl JobStore {
    pub fn in_memory() -> Result<Self, JobStoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, JobStoreError> {
        let path = path.as_ref();
        prepare_job_store_path(path)?;
        let sqlite_path = sqlite_open_path(path)?;
        prepare_job_store_sidecars(&sqlite_path)?;
        let store = Self::from_connection(Connection::open_with_flags(
            sqlite_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?)?;
        harden_existing_job_store_sidecars(path)?;
        Ok(store)
    }

    fn from_connection(conn: Connection) -> Result<Self, JobStoreError> {
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> Result<(), JobStoreError> {
        let conn = self.lock()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                idempotency_key TEXT,
                request_json TEXT NOT NULL,
                result_json TEXT,
                error_json TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )?;
        ensure_column(&conn, "jobs", "idempotency_key", "TEXT")?;
        conn.execute_batch(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS jobs_idempotency_key_unique
                ON jobs(idempotency_key)
                WHERE idempotency_key IS NOT NULL;
            "#,
        )?;
        fail_incomplete_jobs_on_startup(&conn)?;
        Ok(())
    }

    pub fn create(&self, request: &ExecuteRequest) -> Result<String, JobStoreError> {
        self.create_or_get(request).map(|created| created.job_id)
    }

    pub fn create_or_get(&self, request: &ExecuteRequest) -> Result<CreatedJob, JobStoreError> {
        self.create_or_get_inner(request, None)
    }

    pub fn create_or_get_with_limit(
        &self,
        request: &ExecuteRequest,
        max_jobs: usize,
    ) -> Result<CreatedJob, JobStoreError> {
        self.create_or_get_inner(request, Some(max_jobs))
    }

    fn create_or_get_inner(
        &self,
        request: &ExecuteRequest,
        max_jobs: Option<usize>,
    ) -> Result<CreatedJob, JobStoreError> {
        let idempotency_key = normalized_idempotency_key(request);
        let id = Uuid::new_v4().to_string();
        let now = now();
        let request_json = idempotency_request_json(request)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(key) = idempotency_key.as_deref()
            && let Some((existing, existing_request)) = find_by_idempotency_key(&tx, key)?
        {
            if normalized_request_json(&existing_request)? != request_json {
                return Err(JobStoreError::IdempotencyConflict);
            }
            return Ok(CreatedJob {
                job_id: existing,
                inserted: false,
            });
        }
        if let Some(max_jobs) = max_jobs
            && count_jobs(&tx)? >= max_jobs as u64
        {
            return Err(JobStoreError::CapacityExceeded(max_jobs));
        }
        tx.execute(
            "INSERT INTO jobs (id, status, idempotency_key, request_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                JobStatus::Queued.as_str(),
                idempotency_key,
                request_json,
                now,
                now
            ],
        )?;
        tx.commit()?;
        Ok(CreatedJob {
            job_id: id,
            inserted: true,
        })
    }

    pub fn find_idempotent(
        &self,
        request: &ExecuteRequest,
    ) -> Result<Option<String>, JobStoreError> {
        let Some(key) = normalized_idempotency_key(request) else {
            return Ok(None);
        };
        let request_json = idempotency_request_json(request)?;
        let Some((id, existing_request)) = self.find_by_idempotency_key(&key)? else {
            return Ok(None);
        };
        if normalized_request_json(&existing_request)? != request_json {
            return Err(JobStoreError::IdempotencyConflict);
        }
        Ok(Some(id))
    }

    pub fn get(&self, id: &str) -> Result<Option<JobRecord>, JobStoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, status, request_json, result_json, error_json, created_at, updated_at FROM jobs WHERE id = ?1",
            params![id],
            row_to_job,
        )
        .optional()
        .map_err(JobStoreError::from)
    }

    pub fn mark_running(&self, id: &str) -> Result<bool, JobStoreError> {
        let now = now();
        let conn = self.lock()?;
        let rows = conn.execute(
            "UPDATE jobs SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status = ?4",
            params![
                JobStatus::Running.as_str(),
                now,
                id,
                JobStatus::Queued.as_str()
            ],
        )?;
        Ok(rows > 0)
    }

    pub fn complete(&self, id: &str, result: &ExecutionResult) -> Result<(), JobStoreError> {
        let now = now();
        let result_json = serde_json::to_string(result)?;
        let conn = self.lock()?;
        conn.execute(
            "UPDATE jobs SET status = ?1, result_json = ?2, error_json = NULL, updated_at = ?3 WHERE id = ?4 AND status = ?5",
            params![
                JobStatus::Succeeded.as_str(),
                result_json,
                now,
                id,
                JobStatus::Running.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn fail(&self, id: &str, error: &ErrorBody) -> Result<(), JobStoreError> {
        let now = now();
        let error_json = serde_json::to_string(error)?;
        let conn = self.lock()?;
        conn.execute(
            "UPDATE jobs SET status = ?1, error_json = ?2, result_json = NULL, updated_at = ?3 WHERE id = ?4 AND status = ?5",
            params![
                JobStatus::Failed.as_str(),
                error_json,
                now,
                id,
                JobStatus::Running.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<CancelOutcome, JobStoreError> {
        let now = now();
        let conn = self.lock()?;
        let rows = conn.execute(
            "UPDATE jobs SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status IN (?4, ?5)",
            params![
                JobStatus::Canceled.as_str(),
                now,
                id,
                JobStatus::Queued.as_str(),
                JobStatus::Running.as_str()
            ],
        )?;
        if rows > 0 {
            return Ok(CancelOutcome::Canceled);
        }
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        match status.as_deref() {
            None => Ok(CancelOutcome::Missing),
            Some("canceled") => Ok(CancelOutcome::AlreadyCanceled),
            Some(status) => Ok(CancelOutcome::NotCancelable(parse_status(status)?)),
        }
    }

    fn find_by_idempotency_key(
        &self,
        key: &str,
    ) -> Result<Option<(String, String)>, JobStoreError> {
        let conn = self.lock()?;
        find_by_idempotency_key(&conn, key)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, JobStoreError> {
        self.conn
            .lock()
            .map_err(|_| JobStoreError::PoisonedConnection)
    }
}

#[derive(Debug, Error)]
pub enum JobStoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("invalid job store path {path}: {reason}")]
    InvalidPath { path: String, reason: String },
    #[error("job store connection mutex was poisoned")]
    PoisonedConnection,
    #[error("invalid persisted job status `{0}`")]
    InvalidStatus(String),
    #[error("idempotency key was already used for a different request")]
    IdempotencyConflict,
    #[error("maximum stored jobs ({0}) already exist")]
    CapacityExceeded(usize),
}

fn prepare_job_store_path(path: &Path) -> Result<(), JobStoreError> {
    reject_sqlite_uri_path(path)?;
    match job_store_path_status(path)? {
        JobStorePathStatus::RegularFile => {
            harden_job_store_file(path)?;
            Ok(())
        }
        JobStorePathStatus::Missing => create_private_job_store_file(path),
    }
}

fn sqlite_open_path(path: &Path) -> Result<PathBuf, JobStoreError> {
    fs::canonicalize(path).map_err(JobStoreError::Io)
}

fn prepare_job_store_sidecars(path: &Path) -> Result<(), JobStoreError> {
    for path in job_store_sidecar_paths(path) {
        match job_store_path_status(&path)? {
            JobStorePathStatus::RegularFile => harden_job_store_file(&path)?,
            JobStorePathStatus::Missing => {}
        }
    }
    Ok(())
}

fn harden_existing_job_store_sidecars(path: &Path) -> Result<(), JobStoreError> {
    let path = sqlite_open_path(path)?;
    prepare_job_store_sidecars(&path)
}

fn job_store_sidecar_paths(path: &Path) -> [PathBuf; 2] {
    [
        path_with_suffix(path, "-wal"),
        path_with_suffix(path, "-shm"),
    ]
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

enum JobStorePathStatus {
    RegularFile,
    Missing,
}

fn reject_sqlite_uri_path(path: &Path) -> Result<(), JobStoreError> {
    if path.to_str().is_some_and(|path| path.starts_with("file:")) {
        return Err(JobStoreError::InvalidPath {
            path: path.display().to_string(),
            reason: "SQLite URI filenames are not accepted; use a literal filesystem path"
                .to_string(),
        });
    }
    Ok(())
}

fn job_store_path_status(path: &Path) -> Result<JobStorePathStatus, JobStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(JobStoreError::InvalidPath {
            path: path.display().to_string(),
            reason: "path must not be a symlink".to_string(),
        }),
        Ok(metadata) if !metadata.file_type().is_file() => Err(JobStoreError::InvalidPath {
            path: path.display().to_string(),
            reason: "path must be a regular file".to_string(),
        }),
        Ok(_) => Ok(JobStorePathStatus::RegularFile),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(JobStorePathStatus::Missing),
        Err(error) => Err(JobStoreError::Io(error)),
    }
}

fn create_private_job_store_file(path: &Path) -> Result<(), JobStoreError> {
    match create_private_file_new(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            match job_store_path_status(path)? {
                JobStorePathStatus::RegularFile => {
                    harden_job_store_file(path)?;
                    Ok(())
                }
                JobStorePathStatus::Missing => Err(JobStoreError::Io(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("job store path disappeared before open: {}", path.display()),
                ))),
            }
        }
        Err(error) => Err(JobStoreError::Io(error)),
    }
}

#[cfg(unix)]
fn create_private_file_new(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file_new(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(unix)]
fn harden_job_store_file(path: &Path) -> Result<(), JobStoreError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn harden_job_store_file(_path: &Path) -> Result<(), JobStoreError> {
    Ok(())
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    let id: String = row.get(0)?;
    let status: String = row.get(1)?;
    let request_json: String = row.get(2)?;
    let result_json: Option<String> = row.get(3)?;
    let error_json: Option<String> = row.get(4)?;
    let created_at: String = row.get(5)?;
    let updated_at: String = row.get(6)?;

    let request = serde_json::from_str(&request_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let result = result_json
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let error = error_json
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;

    Ok(JobRecord {
        job_id: id,
        status: parse_status(&status).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        request,
        result,
        error,
        created_at,
        updated_at,
    })
}

fn parse_status(value: &str) -> Result<JobStatus, JobStoreError> {
    match value {
        "queued" => Ok(JobStatus::Queued),
        "running" => Ok(JobStatus::Running),
        "succeeded" => Ok(JobStatus::Succeeded),
        "failed" => Ok(JobStatus::Failed),
        "canceled" => Ok(JobStatus::Canceled),
        other => Err(JobStoreError::InvalidStatus(other.to_string())),
    }
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<(), JobStoreError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|existing| existing == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"),
            [],
        )?;
    }
    Ok(())
}

fn find_by_idempotency_key(
    conn: &Connection,
    key: &str,
) -> Result<Option<(String, String)>, JobStoreError> {
    conn.query_row(
        "SELECT id, request_json FROM jobs WHERE idempotency_key = ?1",
        params![key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(JobStoreError::from)
}

fn count_jobs(conn: &Connection) -> Result<u64, JobStoreError> {
    let count = conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get::<_, i64>(0))?;
    Ok(count.max(0) as u64)
}

fn fail_incomplete_jobs_on_startup(conn: &Connection) -> Result<(), JobStoreError> {
    let now = now();
    let error_json = serde_json::to_string(&ErrorBody::new(
        "daemon_restart",
        "job was queued or running when beatboxd started; previous worker is no longer alive",
    ))?;
    conn.execute(
        "UPDATE jobs SET status = ?1, result_json = NULL, error_json = ?2, updated_at = ?3 WHERE status IN (?4, ?5)",
        params![
            JobStatus::Failed.as_str(),
            error_json,
            now,
            JobStatus::Queued.as_str(),
            JobStatus::Running.as_str()
        ],
    )?;
    Ok(())
}

fn normalized_idempotency_key(request: &ExecuteRequest) -> Option<String> {
    request
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
}

fn idempotency_request_json(request: &ExecuteRequest) -> Result<String, JobStoreError> {
    let mut request = request.clone();
    request.idempotency_key = normalized_idempotency_key(&request);
    serde_json::to_string(&request).map_err(JobStoreError::from)
}

fn normalized_request_json(json: &str) -> Result<String, JobStoreError> {
    let mut request: ExecuteRequest = serde_json::from_str(json)?;
    request.idempotency_key = normalized_idempotency_key(&request);
    serde_json::to_string(&request).map_err(JobStoreError::from)
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use beatbox_core::{Lane, Policy, Source};
    use serde_json::json;

    use super::{job_store_sidecar_paths, path_with_suffix, JobStore, JobStoreError};

    fn request() -> beatbox_core::ExecuteRequest {
        beatbox_core::ExecuteRequest {
            lane: Lane::Wasm,
            source: Source::WasmWat {
                text: "(module (func (export \"run\")))".to_string(),
            },
            entrypoint: None,
            input: json!(null),
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        }
    }

    #[test]
    fn canceled_job_does_not_transition_to_running() -> Result<(), Box<dyn std::error::Error>> {
        let store = JobStore::in_memory()?;
        let request = request();
        let id = store.create(&request)?;

        assert!(matches!(store.cancel(&id)?, super::CancelOutcome::Canceled));
        assert!(!store.mark_running(&id)?);

        let job = store
            .get(&id)?
            .ok_or_else(|| std::io::Error::other("job exists"))?;
        assert_eq!(job.status, beatbox_core::JobStatus::Canceled);
        Ok(())
    }

    #[test]
    fn idempotency_key_reuses_persisted_job_after_reopen() -> Result<(), Box<dyn std::error::Error>>
    {
        let db_path =
            std::env::temp_dir().join(format!("beatbox-jobs-{}.sqlite3", uuid::Uuid::new_v4()));
        let mut request = request();
        request.idempotency_key = Some("same-step".to_string());

        let first_id = {
            let store = JobStore::open(&db_path)?;
            store.create(&request)?
        };
        let second_id = {
            let store = JobStore::open(&db_path)?;
            store.create(&request)?
        };

        assert_eq!(first_id, second_id);
        remove_sqlite_files(&db_path);
        Ok(())
    }

    #[test]
    fn incomplete_jobs_fail_after_reopen() -> Result<(), Box<dyn std::error::Error>> {
        let db_path =
            std::env::temp_dir().join(format!("beatbox-jobs-{}.sqlite3", uuid::Uuid::new_v4()));

        let (queued_id, running_id) = {
            let store = JobStore::open(&db_path)?;
            let queued_id = store.create(&request())?;
            let running_id = store.create(&request())?;
            assert!(store.mark_running(&running_id)?);
            (queued_id, running_id)
        };

        let reopened = JobStore::open(&db_path)?;
        for id in [queued_id, running_id] {
            let job = reopened
                .get(&id)?
                .ok_or_else(|| std::io::Error::other("job exists"))?;
            assert_eq!(job.status, beatbox_core::JobStatus::Failed);
            assert_eq!(
                job.error.as_ref().map(|error| error.code.as_str()),
                Some("daemon_restart")
            );
            assert!(job.result.is_none());
        }

        remove_sqlite_files(&db_path);
        Ok(())
    }

    #[test]
    fn job_store_open_rejects_sqlite_uri_paths() -> Result<(), Box<dyn std::error::Error>> {
        match JobStore::open("file:beatbox-jobs?mode=memory") {
            Err(JobStoreError::InvalidPath { reason, .. }) => {
                assert!(reason.contains("URI"), "{reason}");
                Ok(())
            }
            Ok(_) => Err("SQLite URI job store path unexpectedly opened".into()),
            Err(error) => Err(format!("unexpected error: {error}").into()),
        }
    }

    #[test]
    fn job_store_open_rejects_directory_paths() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("beatbox-jobs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let result = JobStore::open(&root);
        std::fs::remove_dir(&root).ok();
        match result {
            Err(JobStoreError::InvalidPath { reason, .. }) => {
                assert!(reason.contains("regular file"), "{reason}");
                Ok(())
            }
            Ok(_) => Err("directory job store path unexpectedly opened".into()),
            Err(error) => Err(format!("unexpected error: {error}").into()),
        }
    }

    #[test]
    #[cfg(unix)]
    fn job_store_open_rejects_symlink_paths() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("beatbox-jobs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let real = root.join("real.sqlite3");
        std::fs::write(&real, b"not sqlite")?;
        let link = root.join("link.sqlite3");
        symlink(&real, &link)?;

        let result = JobStore::open(&link);
        std::fs::remove_file(&link).ok();
        std::fs::remove_file(&real).ok();
        std::fs::remove_dir(&root).ok();
        match result {
            Err(JobStoreError::InvalidPath { reason, .. }) => {
                assert!(reason.contains("symlink"), "{reason}");
                Ok(())
            }
            Ok(_) => Err("symlink job store path unexpectedly opened".into()),
            Err(error) => Err(format!("unexpected error: {error}").into()),
        }
    }

    #[test]
    #[cfg(unix)]
    fn job_store_open_rejects_symlink_sidecars() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("beatbox-jobs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let db_path = root.join("beatbox.sqlite3");
        std::fs::write(&db_path, b"")?;
        let real = root.join("real-wal");
        std::fs::write(&real, b"")?;
        let link = path_with_suffix(&db_path, "-wal");
        symlink(&real, &link)?;

        let result = JobStore::open(&db_path);
        std::fs::remove_file(&link).ok();
        std::fs::remove_file(&real).ok();
        remove_sqlite_files(&db_path);
        std::fs::remove_dir(&root).ok();
        match result {
            Err(JobStoreError::InvalidPath { reason, .. }) => {
                assert!(reason.contains("symlink"), "{reason}");
                Ok(())
            }
            Ok(_) => Err("symlink SQLite sidecar path unexpectedly opened".into()),
            Err(error) => Err(format!("unexpected error: {error}").into()),
        }
    }

    #[test]
    #[cfg(unix)]
    fn job_store_open_creates_private_regular_file() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let db_path =
            std::env::temp_dir().join(format!("beatbox-jobs-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = JobStore::open(&db_path)?;
        drop(store);

        let metadata = std::fs::symlink_metadata(&db_path)?;
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        remove_sqlite_files(&db_path);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn job_store_open_hardens_sqlite_sidecars() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let db_path =
            std::env::temp_dir().join(format!("beatbox-jobs-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = JobStore::open(&db_path)?;
        store.create(&request())?;

        for path in job_store_sidecar_paths(&db_path) {
            if path.exists() {
                let metadata = std::fs::symlink_metadata(&path)?;
                assert!(metadata.file_type().is_file());
                assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
            }
        }

        drop(store);
        remove_sqlite_files(&db_path);
        Ok(())
    }

    #[test]
    fn completed_and_canceled_jobs_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
        let db_path =
            std::env::temp_dir().join(format!("beatbox-jobs-{}.sqlite3", uuid::Uuid::new_v4()));

        let (completed_id, canceled_id) = {
            let store = JobStore::open(&db_path)?;
            let completed_id = store.create(&request())?;
            assert!(store.mark_running(&completed_id)?);
            store.complete(&completed_id, &ok_result(json!(7)))?;
            let canceled_id = store.create(&request())?;
            assert!(matches!(
                store.cancel(&canceled_id)?,
                super::CancelOutcome::Canceled
            ));
            (completed_id, canceled_id)
        };

        let reopened = JobStore::open(&db_path)?;
        let completed = reopened
            .get(&completed_id)?
            .ok_or_else(|| std::io::Error::other("completed job exists"))?;
        let canceled = reopened
            .get(&canceled_id)?
            .ok_or_else(|| std::io::Error::other("canceled job exists"))?;

        assert_eq!(completed.status, beatbox_core::JobStatus::Succeeded);
        assert_eq!(
            completed.result.as_ref().map(|result| &result.value),
            Some(&json!(7))
        );
        assert_eq!(canceled.status, beatbox_core::JobStatus::Canceled);
        assert!(canceled.error.is_none());

        remove_sqlite_files(&db_path);
        Ok(())
    }

    #[test]
    fn terminal_jobs_ignore_late_worker_writes() -> Result<(), Box<dyn std::error::Error>> {
        let store = JobStore::in_memory()?;

        let canceled_id = store.create(&request())?;
        assert!(store.mark_running(&canceled_id)?);
        assert!(matches!(
            store.cancel(&canceled_id)?,
            super::CancelOutcome::Canceled
        ));
        store.complete(&canceled_id, &ok_result(json!(1)))?;
        store.fail(
            &canceled_id,
            &beatbox_core::ErrorBody::new("late_failure", "late failure"),
        )?;
        let canceled = store
            .get(&canceled_id)?
            .ok_or_else(|| std::io::Error::other("canceled job exists"))?;
        assert_eq!(canceled.status, beatbox_core::JobStatus::Canceled);
        assert!(canceled.result.is_none());
        assert!(canceled.error.is_none());

        let failed_id = store.create(&request())?;
        assert!(store.mark_running(&failed_id)?);
        store.fail(
            &failed_id,
            &beatbox_core::ErrorBody::new("first_failure", "first failure"),
        )?;
        store.complete(&failed_id, &ok_result(json!(2)))?;
        store.fail(
            &failed_id,
            &beatbox_core::ErrorBody::new("second_failure", "second failure"),
        )?;
        let failed = store
            .get(&failed_id)?
            .ok_or_else(|| std::io::Error::other("failed job exists"))?;
        assert_eq!(failed.status, beatbox_core::JobStatus::Failed);
        assert_eq!(
            failed.error.as_ref().map(|error| error.code.as_str()),
            Some("first_failure")
        );
        assert!(failed.result.is_none());

        Ok(())
    }

    #[test]
    fn terminal_jobs_are_not_cancelable() -> Result<(), Box<dyn std::error::Error>> {
        let store = JobStore::in_memory()?;

        let succeeded_id = store.create(&request())?;
        assert!(store.mark_running(&succeeded_id)?);
        store.complete(&succeeded_id, &ok_result(json!(1)))?;
        match store.cancel(&succeeded_id)? {
            super::CancelOutcome::NotCancelable(beatbox_core::JobStatus::Succeeded) => {}
            other => return Err(format!("unexpected succeeded cancel outcome: {other:?}").into()),
        }

        let failed_id = store.create(&request())?;
        assert!(store.mark_running(&failed_id)?);
        store.fail(
            &failed_id,
            &beatbox_core::ErrorBody::new("failed", "failed"),
        )?;
        match store.cancel(&failed_id)? {
            super::CancelOutcome::NotCancelable(beatbox_core::JobStatus::Failed) => {}
            other => return Err(format!("unexpected failed cancel outcome: {other:?}").into()),
        }

        let canceled_id = store.create(&request())?;
        assert!(matches!(
            store.cancel(&canceled_id)?,
            super::CancelOutcome::Canceled
        ));
        assert!(matches!(
            store.cancel(&canceled_id)?,
            super::CancelOutcome::AlreadyCanceled
        ));

        assert!(matches!(
            store.cancel("missing")?,
            super::CancelOutcome::Missing
        ));
        Ok(())
    }

    #[test]
    fn idempotency_comparison_ignores_key_whitespace_and_persists_normalized_key(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let store = JobStore::in_memory()?;
        let mut first = request();
        first.idempotency_key = Some(" retry-key ".to_string());
        let first_id = store.create(&first)?;

        let mut second = request();
        second.idempotency_key = Some("retry-key".to_string());
        let second_id = store.create(&second)?;

        assert_eq!(first_id, second_id);
        let job = store
            .get(&first_id)?
            .ok_or_else(|| std::io::Error::other("job exists"))?;
        assert_eq!(job.request.idempotency_key.as_deref(), Some("retry-key"));
        Ok(())
    }

    #[test]
    fn idempotency_reuses_legacy_compact_limit_request_json(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let store = JobStore::in_memory()?;
        let mut retry = request();
        retry.idempotency_key = Some("compact-key".to_string());
        retry.policy.limits.wall_ms = 250;

        let compact_request = serde_json::json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": "(module (func (export \"run\")))"},
            "policy": {"limits": {"wall_ms": 250}},
            "idempotency_key": " compact-key "
        });
        {
            let conn = store.lock()?;
            conn.execute(
                "INSERT INTO jobs (id, status, idempotency_key, request_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    "legacy-compact-job",
                    beatbox_core::JobStatus::Queued.as_str(),
                    "compact-key",
                    compact_request.to_string(),
                    "2026-01-01T00:00:00Z",
                    "2026-01-01T00:00:00Z",
                ],
            )?;
        }

        let created = store.create_or_get(&retry)?;
        assert_eq!(created.job_id, "legacy-compact-job");
        assert!(!created.inserted);
        assert_eq!(
            store.find_idempotent(&retry)?.as_deref(),
            Some("legacy-compact-job")
        );
        Ok(())
    }

    #[test]
    fn idempotency_conflict_ignores_only_the_idempotency_key(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let store = JobStore::in_memory()?;
        let mut first = request();
        first.idempotency_key = Some("retry-key".to_string());
        store.create(&first)?;

        let mut second = request();
        second.idempotency_key = Some(" retry-key ".to_string());
        second.input = json!({"n": 1});

        match store.create(&second) {
            Err(JobStoreError::IdempotencyConflict) => Ok(()),
            Ok(_) => Err("conflicting idempotent request unexpectedly reused a job".into()),
            Err(error) => Err(error.into()),
        }
    }

    fn remove_sqlite_files(db_path: &std::path::Path) {
        std::fs::remove_file(db_path).ok();
        std::fs::remove_file(format!("{}-wal", db_path.display())).ok();
        std::fs::remove_file(format!("{}-shm", db_path.display())).ok();
    }

    fn ok_result(value: serde_json::Value) -> beatbox_core::ExecutionResult {
        beatbox_core::ExecutionResult {
            status: beatbox_core::ExecutionStatus::Ok,
            value,
            exit_code: Some(0),
            stdout: String::new(),
            stdout_truncated: false,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            metrics: beatbox_core::Metrics::default(),
            lane: beatbox_core::Lane::Wasm,
            deterministic: true,
            inputs_digest: "test-inputs".to_string(),
            engine_version: "test-engine".to_string(),
            beatbox_version: "test-beatbox".to_string(),
            effective_isolation: beatbox_core::EffectiveIsolation {
                os: "test".to_string(),
                mechanisms: Vec::new(),
                landlock_abi: None,
                downgrades: Vec::new(),
            },
            egress: Vec::new(),
        }
    }
}
