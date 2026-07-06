use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{bail, Result};
use beatbox_engine::BeatboxEngine;
use beatbox_server::{router, AuthMode, ServerConfig, DEFAULT_MAX_STORED_JOBS};
use clap::Parser;
use std::io::Write;

#[derive(Parser)]
#[command(version, about = "Run the beatbox sandbox daemon.")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:7300")]
    addr: SocketAddr,
    #[arg(long, env = "BEATBOX_API_KEY")]
    api_key: Option<String>,
    #[arg(
        long,
        env = "BEATBOX_DB_PATH",
        default_value = ".beatbox/beatbox.sqlite3"
    )]
    db_path: PathBuf,
    #[arg(long, env = "BEATBOX_MAX_STORED_JOBS", default_value_t = DEFAULT_MAX_STORED_JOBS)]
    max_stored_jobs: usize,
    /// Allow starting without BEATBOX_API_KEY. Only use in isolated local tests.
    #[arg(long, alias = "allow-unauthenticated-remote")]
    allow_unauthenticated: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let auth = startup_auth(cli.api_key, cli.allow_unauthenticated, cli.addr)?;
    let engine = BeatboxEngine::new()?;
    prepare_private_db_path(&cli.db_path)?;
    let mut config = ServerConfig::new(engine).with_sqlite_job_store(&cli.db_path)?;
    harden_sqlite_file_permissions(&cli.db_path)?;
    config.max_stored_jobs = cli.max_stored_jobs;
    config.auth = auth;
    let listener = tokio::net::TcpListener::bind(cli.addr).await?;
    let local_addr = listener.local_addr()?;
    println!("beatboxd listening on http://{local_addr}");
    io::stdout().flush()?;
    axum::serve(listener, router(config)).await?;
    Ok(())
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn startup_auth(
    api_key: Option<String>,
    allow_unauthenticated: bool,
    addr: SocketAddr,
) -> Result<AuthMode> {
    if let Some(token) = api_key.and_then(non_empty) {
        return Ok(AuthMode::Required { token });
    }
    if !allow_unauthenticated {
        bail!(
            "refusing to start beatboxd without BEATBOX_API_KEY; loopback is not authentication, so pass --allow-unauthenticated only for isolated local tests"
        );
    }
    if !addr.ip().is_loopback() {
        bail!(
            "refusing to start unauthenticated beatboxd on non-loopback address {addr}; --allow-unauthenticated is only allowed with loopback bind addresses"
        );
    }
    Ok(AuthMode::None)
}

fn prepare_private_db_path(path: &Path) -> Result<()> {
    reject_sqlite_uri_path(path)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir_all(parent).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to create {}: {error}", parent.display()),
            )
        })?;
    }
    reject_symlink_or_non_file(path)?;
    create_private_file_if_missing(path)?;
    harden_file_permissions(path)
}

fn reject_sqlite_uri_path(path: &Path) -> Result<()> {
    if path.to_str().is_some_and(|path| path.starts_with("file:")) {
        bail!(
            "refusing to use {} as a job store: SQLite URI filenames are not accepted; use a literal filesystem path",
            path.display()
        );
    }
    Ok(())
}

fn reject_symlink_or_non_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "refusing to use {} as a job store: path must not be a symlink",
                path.display()
            )
        }
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!(
                "refusing to use {} as a job store: path must be a regular file",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn create_private_file_if_missing(path: &Path) -> Result<()> {
    if path.exists() {
        return reject_symlink_or_non_file(path);
    }
    create_private_file_new(path).or_else(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            reject_symlink_or_non_file(path)
        } else {
            Err(error.into())
        }
    })
}

fn harden_sqlite_file_permissions(path: &Path) -> Result<()> {
    for path in sqlite_file_paths(path) {
        if path.exists() {
            reject_symlink_or_non_file(&path)?;
            harden_file_permissions(&path)?;
        }
    }
    Ok(())
}

fn sqlite_file_paths(path: &Path) -> [PathBuf; 3] {
    [
        path.to_path_buf(),
        path_with_suffix(path, "-wal"),
        path_with_suffix(path, "-shm"),
    ]
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

#[cfg(unix)]
fn create_private_dir_all(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

#[cfg(not(unix))]
fn create_private_dir_all(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn create_private_file_new(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_private_file_new(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    Ok(())
}

#[cfg(unix)]
fn harden_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn harden_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn addr(value: &str) -> SocketAddr {
        match value.parse() {
            Ok(addr) => addr,
            Err(error) => panic!("test address must parse: {error}"),
        }
    }

    #[test]
    fn startup_auth_trims_and_requires_non_empty_keys() -> Result<()> {
        match startup_auth(Some("  secret  ".to_string()), false, addr("0.0.0.0:7300"))? {
            AuthMode::Required { token } => assert_eq!(token, "secret"),
            AuthMode::None => panic!("expected required auth mode"),
        }
        let error = match startup_auth(Some("   ".to_string()), false, addr("127.0.0.1:7300")) {
            Ok(_) => panic!("empty API key must be treated as missing"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("BEATBOX_API_KEY"));
        Ok(())
    }

    #[test]
    fn startup_auth_allows_unauthenticated_only_on_loopback() -> Result<()> {
        assert!(matches!(
            startup_auth(None, true, addr("127.0.0.1:7300"))?,
            AuthMode::None
        ));
        assert!(matches!(
            startup_auth(None, true, addr("[::1]:7300"))?,
            AuthMode::None
        ));

        for bind in ["0.0.0.0:7300", "[::]:7300", "192.0.2.1:7300"] {
            let error = match startup_auth(None, true, addr(bind)) {
                Ok(_) => panic!("non-loopback unauthenticated bind must be rejected"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("non-loopback address"));
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn db_path_setup_creates_private_parent_and_file() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let root = unique_temp_path();
        let db_path = root.join("nested").join("beatbox.sqlite3");

        prepare_private_db_path(&db_path)?;

        let parent = db_path
            .parent()
            .ok_or_else(|| io::Error::other("db path should have a parent"))?;
        let parent_mode = fs::metadata(parent)?.permissions().mode() & 0o777;
        let file_mode = fs::metadata(&db_path)?.permissions().mode() & 0o777;
        assert_eq!(parent_mode, 0o700);
        assert_eq!(file_mode, 0o600);

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn db_path_setup_rejects_sqlite_uri_without_creating_literal_file() -> Result<()> {
        let _cwd_lock = CWD_LOCK
            .lock()
            .map_err(|_| io::Error::other("cwd test lock was poisoned"))?;
        let root = unique_temp_path();
        fs::create_dir_all(&root)?;
        let original_cwd = std::env::current_dir()?;
        std::env::set_current_dir(&root)?;

        let db_path = Path::new("file:beatbox.sqlite3?mode=memory");
        let result = prepare_private_db_path(db_path);
        std::env::set_current_dir(original_cwd)?;

        let error = match result {
            Ok(()) => panic!("SQLite URI database path must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("SQLite URI"));
        assert!(!root.join(db_path).exists());

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn db_path_setup_rejects_symlink() -> Result<()> {
        use std::os::unix::fs::symlink;

        let root = unique_temp_path();
        fs::create_dir_all(&root)?;
        let real = root.join("real.sqlite3");
        fs::write(&real, b"")?;
        let link = root.join("link.sqlite3");
        symlink(&real, &link)?;

        let error = match prepare_private_db_path(&link) {
            Ok(()) => panic!("symlink database path must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("must not be a symlink"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn sqlite_sidecar_permissions_are_hardened_after_open() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let root = unique_temp_path();
        let db_path = root.join("beatbox.sqlite3");
        prepare_private_db_path(&db_path)?;
        let store = beatbox_server::JobStore::open(&db_path)?;
        store.create(&beatbox_core::ExecuteRequest {
            lane: beatbox_core::Lane::Wasm,
            source: beatbox_core::Source::WasmWat {
                text: "(module (func (export \"run\")))".to_string(),
            },
            entrypoint: None,
            input: serde_json::Value::Null,
            stdin: String::new(),
            policy: beatbox_core::Policy::default(),
            idempotency_key: None,
        })?;
        harden_sqlite_file_permissions(&db_path)?;

        for path in sqlite_file_paths(&db_path) {
            if path.exists() {
                let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{}", path.display());
            }
        }

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[cfg(unix)]
    fn unique_temp_path() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("beatboxd-test-{}-{nanos}", std::process::id()))
            .join(counter.to_string())
    }
}
