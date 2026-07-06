use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{bail, Context, Result};
use beatbox_engine::BeatboxEngine;
use beatbox_server::{router, AuthMode, ServerConfig};
use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Run the beatbox sandbox daemon.")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:7300")]
    addr: SocketAddr,
    #[arg(long, env = "BEATBOX_API_KEY")]
    api_key: Option<String>,
    /// Read the API key from a file instead of --api-key/BEATBOX_API_KEY so the
    /// secret never appears in `ps`/`/proc/*/cmdline` or shell history. Takes
    /// precedence over --api-key/BEATBOX_API_KEY when both are set.
    #[arg(long, env = "BEATBOX_API_KEY_FILE")]
    api_key_file: Option<PathBuf>,
    #[arg(
        long,
        env = "BEATBOX_DB_PATH",
        default_value = ".beatbox/beatbox.sqlite3"
    )]
    db_path: PathBuf,
    /// Allow binding a non-loopback address without BEATBOX_API_KEY.
    #[arg(long)]
    allow_unauthenticated_remote: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let api_key = resolve_api_key(cli.api_key, cli.api_key_file)?;
    if !cli.addr.ip().is_loopback() && api_key.is_none() && !cli.allow_unauthenticated_remote {
        bail!(
            "refusing to bind {} without BEATBOX_API_KEY; pass --allow-unauthenticated-remote only for isolated test networks",
            cli.addr
        );
    }
    let engine = BeatboxEngine::new()?;
    if let Some(parent) = cli.db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to create {}: {error}", parent.display()),
            )
        })?;
    }
    // Single-instance guard: startup recovery fails every non-terminal job, so a
    // second daemon sharing this database would reconcile the first daemon's
    // in-flight jobs out from under it. Hold an exclusive advisory lock on a
    // sidecar file (released by the OS on exit, even on crash) for the process
    // lifetime and refuse to start if another daemon holds it.
    let _db_lock = acquire_db_lock(&cli.db_path)?;
    let mut config = ServerConfig::new(engine).with_sqlite_job_store(&cli.db_path)?;
    if let Some(token) = api_key {
        config.auth = AuthMode::required(token)?;
    }
    let listener = tokio::net::TcpListener::bind(cli.addr).await?;
    println!("beatboxd listening on http://{}", cli.addr);
    axum::serve(listener, router(config))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Resolve on Ctrl-C or SIGTERM (the signal a deploy/orchestrator sends on
/// restart) so axum stops accepting new connections and drains in-flight
/// requests. Jobs still running in detached workers are reconciled to a
/// terminal state by JobStore startup recovery on the next boot.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received; draining in-flight requests");
}

/// Resolve the API key, preferring the file when given. File precedence (rather
/// than erroring on both) avoids a footgun: clap's `env` fallback populates the
/// inline key from BEATBOX_API_KEY whether or not the user passed --api-key, so
/// treating "both present" as an error would spuriously reject the common case
/// of a leftover env var plus an explicit --api-key-file.
fn resolve_api_key(inline: Option<String>, file: Option<PathBuf>) -> Result<Option<String>> {
    match file {
        Some(path) => {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read API key file {}", path.display()))?;
            Ok(non_empty(contents))
        }
        None => Ok(inline.and_then(non_empty)),
    }
}

/// Take an exclusive advisory lock on `<db_path>.lock`, returning the held file
/// (kept alive for the process lifetime). Errors if another process holds it.
fn acquire_db_lock(db_path: &Path) -> Result<fs::File> {
    let lock_path = PathBuf::from(format!("{}.lock", db_path.display()));
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;
    match lock_file.try_lock() {
        Ok(()) => Ok(lock_file),
        Err(fs::TryLockError::WouldBlock) => bail!(
            "another beatboxd already holds {}; only one daemon may use a job database at a time",
            db_path.display()
        ),
        Err(fs::TryLockError::Error(error)) => {
            Err(anyhow::Error::new(error)
                .context(format!("failed to lock {}", lock_path.display())))
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}
