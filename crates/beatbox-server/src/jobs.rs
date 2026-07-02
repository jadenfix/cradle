use std::path::Path;
use std::sync::{Arc, Mutex};

use beatbox_core::{ErrorBody, ExecuteRequest, ExecutionResult, JobRecord, JobStatus};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
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

impl JobStore {
    pub fn in_memory() -> Result<Self, JobStoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, JobStoreError> {
        Self::from_connection(Connection::open(path)?)
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
            CREATE INDEX IF NOT EXISTS jobs_status_updated_at ON jobs(status, updated_at);
            "#,
        )?;

        // Recover jobs left non-terminal by a crash or restart. Workers are
        // in-process tokio tasks, so anything still queued/running at startup has
        // no worker and can never progress. Fail them (with a distinct code) so
        // GET /v1/jobs/{id} and idempotent retries observe a terminal state
        // instead of hanging forever on a wedged job.
        let now = now();
        let recovery_error = serde_json::to_string(&ErrorBody::new(
            "daemon_restarted",
            "daemon restarted before this job completed",
        ))?;
        conn.execute(
            "UPDATE jobs SET status = ?1, error_json = ?2, updated_at = ?3 WHERE status IN (?4, ?5)",
            params![
                JobStatus::Failed.as_str(),
                recovery_error,
                now,
                JobStatus::Queued.as_str(),
                JobStatus::Running.as_str()
            ],
        )?;

        // Bound table growth: an insert-only store grows without limit over a
        // long-lived daemon. Evict terminal jobs older than the retention window.
        conn.execute(
            "DELETE FROM jobs WHERE status IN (?1, ?2, ?3) AND updated_at < ?4",
            params![
                JobStatus::Succeeded.as_str(),
                JobStatus::Failed.as_str(),
                JobStatus::Canceled.as_str(),
                retention_cutoff()
            ],
        )?;
        Ok(())
    }

    pub fn create(&self, request: &ExecuteRequest) -> Result<String, JobStoreError> {
        self.create_or_get(request).map(|created| created.job_id)
    }

    pub fn create_or_get(&self, request: &ExecuteRequest) -> Result<CreatedJob, JobStoreError> {
        let idempotency_key = normalized_idempotency_key(request);
        let id = Uuid::new_v4().to_string();
        let now = now();
        let request_json = serde_json::to_string(request)?;
        let conn = self.lock()?;
        if let Some(key) = idempotency_key.as_deref()
            && let Some((existing, existing_request)) = find_by_idempotency_key(&conn, key)?
        {
            if existing_request != request_json {
                return Err(JobStoreError::IdempotencyConflict);
            }
            return Ok(CreatedJob {
                job_id: existing,
                inserted: false,
            });
        }
        conn.execute(
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
        let request_json = serde_json::to_string(request)?;
        let Some((id, existing_request)) = self.find_by_idempotency_key(&key)? else {
            return Ok(None);
        };
        if existing_request != request_json {
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
            "UPDATE jobs SET status = ?1, result_json = ?2, error_json = NULL, updated_at = ?3 WHERE id = ?4 AND status != ?5",
            params![
                JobStatus::Succeeded.as_str(),
                result_json,
                now,
                id,
                JobStatus::Canceled.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn fail(&self, id: &str, error: &ErrorBody) -> Result<(), JobStoreError> {
        let now = now();
        let error_json = serde_json::to_string(error)?;
        let conn = self.lock()?;
        conn.execute(
            "UPDATE jobs SET status = ?1, error_json = ?2, updated_at = ?3 WHERE id = ?4 AND status != ?5",
            params![
                JobStatus::Failed.as_str(),
                error_json,
                now,
                id,
                JobStatus::Canceled.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<bool, JobStoreError> {
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
            return Ok(true);
        }
        let exists: Option<String> = conn
            .query_row("SELECT id FROM jobs WHERE id = ?1", params![id], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(exists.is_some())
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

/// Terminal jobs older than this are pruned at startup to bound table growth.
const TERMINAL_JOB_RETENTION_DAYS: i64 = 7;

#[derive(Debug, Error)]
pub enum JobStoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("job store connection mutex was poisoned")]
    PoisonedConnection,
    #[error("invalid persisted job status `{0}`")]
    InvalidStatus(String),
    #[error("idempotency key was already used for a different request")]
    IdempotencyConflict,
    #[error("job store worker task failed: {0}")]
    Worker(String),
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

fn normalized_idempotency_key(request: &ExecuteRequest) -> Option<String> {
    request
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn retention_cutoff() -> String {
    (Utc::now() - chrono::Duration::days(TERMINAL_JOB_RETENTION_DAYS)).to_rfc3339()
}

#[cfg(test)]
mod tests {
    use beatbox_core::{Lane, Policy, Source};
    use serde_json::json;

    use super::JobStore;

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

        assert!(store.cancel(&id)?);
        assert!(!store.mark_running(&id)?);

        let job = store
            .get(&id)?
            .ok_or_else(|| std::io::Error::other("job exists"))?;
        assert_eq!(job.status, beatbox_core::JobStatus::Canceled);
        Ok(())
    }

    #[test]
    fn reopen_fails_jobs_left_non_terminal_by_restart() -> Result<(), Box<dyn std::error::Error>> {
        let db_path =
            std::env::temp_dir().join(format!("beatbox-recovery-{}.sqlite3", uuid::Uuid::new_v4()));
        let request = request();

        // Simulate a daemon that persisted a queued job then restarted before the
        // worker finished (the job is never marked running/terminal).
        let id = {
            let store = JobStore::open(&db_path)?;
            store.create(&request)?
        };

        // On reopen, startup recovery must move the wedged job to a terminal
        // failed state with the daemon-restart code.
        let reopened = JobStore::open(&db_path)?;
        let job = reopened
            .get(&id)?
            .ok_or_else(|| std::io::Error::other("recovered job should still exist"))?;
        assert_eq!(job.status, beatbox_core::JobStatus::Failed);
        assert_eq!(
            job.error.as_ref().map(|error| error.code.as_str()),
            Some("daemon_restarted")
        );
        std::fs::remove_file(db_path).ok();
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
        std::fs::remove_file(db_path).ok();
        Ok(())
    }
}
