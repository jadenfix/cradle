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
                request_json TEXT NOT NULL,
                result_json TEXT,
                error_json TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn create(&self, request: &ExecuteRequest) -> Result<String, JobStoreError> {
        let id = Uuid::new_v4().to_string();
        let now = now();
        let request_json = serde_json::to_string(request)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO jobs (id, status, request_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, JobStatus::Queued.as_str(), request_json, now, now],
        )?;
        Ok(id)
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

    pub fn mark_running(&self, id: &str) -> Result<(), JobStoreError> {
        self.set_status(id, JobStatus::Running)
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

    fn set_status(&self, id: &str, status: JobStatus) -> Result<(), JobStoreError> {
        let now = now();
        let conn = self.lock()?;
        conn.execute(
            "UPDATE jobs SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status != ?4",
            params![status.as_str(), now, id, JobStatus::Canceled.as_str()],
        )?;
        Ok(())
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
    #[error("job store connection mutex was poisoned")]
    PoisonedConnection,
    #[error("invalid persisted job status `{0}`")]
    InvalidStatus(String),
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

fn now() -> String {
    Utc::now().to_rfc3339()
}
