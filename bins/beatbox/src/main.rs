use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
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
            policy,
        }) => run(path, input, entrypoint, remote, api_key, policy).await,
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
    policy_items: Vec<String>,
) -> Result<()> {
    let mut policy = Policy::default();
    apply_policy_items(&mut policy, &policy_items)?;
    let input = match input {
        Some(input) => serde_json::from_str(&input).context("--input must be valid JSON")?,
        None => serde_json::Value::Null,
    };
    let request = ExecuteRequest {
        lane: Lane::Wasm,
        source: Source::WasmFile { path },
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
