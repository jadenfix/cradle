use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use beatbox_client::Client;
use beatbox_core::{Determinism, ExecuteRequest, Lane, Policy, Source};
use beatbox_engine::BeatboxEngine;
use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Run untrusted code through beatbox sandbox lanes.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Run {
        path: PathBuf,
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        entrypoint: Option<String>,
        #[arg(long)]
        remote: Option<String>,
        #[arg(long, env = "BEATBOX_API_KEY")]
        api_key: Option<String>,
        /// Read the API key from a file instead of --api-key/BEATBOX_API_KEY so
        /// the secret never appears in `ps`/`/proc/*/cmdline` or shell history.
        #[arg(long, env = "BEATBOX_API_KEY_FILE")]
        api_key_file: Option<PathBuf>,
        #[arg(long = "policy")]
        policy: Vec<String>,
    },
    Compile {
        input: PathBuf,
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Run {
            path,
            input,
            entrypoint,
            remote,
            api_key,
            api_key_file,
            policy,
        }) => {
            run(
                path,
                input,
                entrypoint,
                remote,
                api_key,
                api_key_file,
                policy,
            )
            .await
        }
        Some(Command::Compile { input, output }) => compile(input, output),
        None => {
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}

fn compile(input: PathBuf, output: PathBuf) -> Result<()> {
    let bytes = wat::parse_file(&input)
        .with_context(|| format!("failed to parse WAT from {}", input.display()))?;
    fs::write(&output, bytes)
        .with_context(|| format!("failed to write Wasm to {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

async fn run(
    path: PathBuf,
    input: Option<String>,
    entrypoint: Option<String>,
    remote: Option<String>,
    api_key: Option<String>,
    api_key_file: Option<PathBuf>,
    policy_items: Vec<String>,
) -> Result<()> {
    let api_key = resolve_api_key(api_key, api_key_file)?;
    let mut policy = Policy::default();
    apply_policy_items(&mut policy, &policy_items)?;
    let input = match input {
        Some(input) => serde_json::from_str(&input).context("--input must be valid JSON")?,
        None => serde_json::Value::Null,
    };
    let source = if remote.is_some() {
        source_for_remote(&path)?
    } else {
        Source::WasmFile { path }
    };
    let request = ExecuteRequest {
        lane: Lane::Wasm,
        source,
        entrypoint,
        input,
        stdin: String::new(),
        policy,
        idempotency_key: None,
    };

    let result = if let Some(remote) = remote {
        let mut client = Client::new(remote);
        if let Some(api_key) = api_key {
            client = client.with_api_key(api_key);
        }
        client.execute(&request).await?
    } else {
        BeatboxEngine::new()?.execute(request)?
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// Resolve the API key from at most one of the inline flag/env or a file path.
/// Reading from a file keeps the secret out of the process argument list.
fn resolve_api_key(inline: Option<String>, file: Option<PathBuf>) -> Result<Option<String>> {
    match (inline, file) {
        (Some(_), Some(_)) => bail!(
            "pass only one of --api-key/BEATBOX_API_KEY or --api-key-file/BEATBOX_API_KEY_FILE"
        ),
        (Some(value), None) => Ok(non_empty(value)),
        (None, Some(path)) => {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read API key file {}", path.display()))?;
            Ok(non_empty(contents))
        }
        (None, None) => Ok(None),
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn source_for_remote(path: &Path) -> Result<Source> {
    let bytes = wasm_bytes_from_path(path)?;
    Ok(Source::WasmBytesBase64 {
        bytes: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn wasm_bytes_from_path(path: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("wat") {
        wat::parse_bytes(&bytes)
            .map(|cow| cow.into_owned())
            .with_context(|| format!("failed to parse WAT from {}", path.display()))
    } else {
        Ok(bytes)
    }
}

fn apply_policy_items(policy: &mut Policy, items: &[String]) -> Result<()> {
    for item in items {
        let Some((key, value)) = item.split_once('=') else {
            bail!("policy entries must be key=value, got `{item}`");
        };
        match key {
            "fuel" => {
                policy.limits.fuel = Some(value.parse().context("fuel must be an integer")?);
            }
            "wall_ms" | "timeout_ms" => {
                policy.limits.wall_ms = value.parse().context("wall_ms must be an integer")?;
            }
            "memory_bytes" => {
                policy.limits.memory_bytes =
                    value.parse().context("memory_bytes must be an integer")?;
            }
            "output_bytes" => {
                policy.limits.output_bytes =
                    value.parse().context("output_bytes must be an integer")?;
            }
            "deterministic_seed" => {
                let seed = value
                    .parse()
                    .context("deterministic_seed must be an integer")?;
                policy.determinism = Determinism::Seeded { seed, epoch_ms: 0 };
            }
            other => bail!("unknown policy key `{other}`"),
        }
    }
    Ok(())
}
