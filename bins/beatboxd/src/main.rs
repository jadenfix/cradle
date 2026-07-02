use std::net::SocketAddr;
use std::path::PathBuf;
use std::{fs, io};

use anyhow::Result;
use beatbox_engine::BeatboxEngine;
use beatbox_server::{AuthMode, ServerConfig, router};
use clap::Parser;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
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
    let mut config = ServerConfig::new(engine).with_sqlite_job_store(&cli.db_path)?;
    if let Some(token) = cli.api_key {
        config.auth = AuthMode::Required { token };
    }
    let listener = tokio::net::TcpListener::bind(cli.addr).await?;
    println!("beatboxd listening on http://{}", cli.addr);
    axum::serve(listener, router(config)).await?;
    Ok(())
}
