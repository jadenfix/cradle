use std::fs;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use beatbox_client::Client;
use beatbox_core::{Determinism, ExecuteRequest, Lane, Policy, Source};
use beatbox_engine::{BeatboxEngine, MAX_WASM_MODULE_BYTES};
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
    compile_with_limit(&input, &output, MAX_WASM_MODULE_BYTES)
}

fn compile_with_limit(input: &Path, output: &Path, max_bytes: u64) -> Result<()> {
    let source = read_capped_file(input, max_bytes)?;
    let bytes = wat::parse_bytes(&source)
        .map(|cow| cow.into_owned())
        .with_context(|| format!("failed to parse WAT from {}", input.display()))?;
    ensure_source_limit(input, "module", bytes_len_u64(bytes.len()), max_bytes)?;
    fs::write(output, bytes)
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
    let source = if remote.is_some() {
        source_for_remote(&path, wasm_module_byte_limit(policy.limits.memory_bytes))?
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
            client = client.with_api_key_allowing_loopback_http(api_key);
        }
        client.execute(&request).await?
    } else {
        BeatboxEngine::new()?.execute(request)?
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn source_for_remote(path: &Path, max_bytes: u64) -> Result<Source> {
    let bytes = wasm_bytes_from_path(path, max_bytes)?;
    Ok(Source::WasmBytesBase64 {
        bytes: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn wasm_bytes_from_path(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let bytes = read_capped_file(path, max_bytes)?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("wat") {
        let module = wat::parse_bytes(&bytes)
            .map(|cow| cow.into_owned())
            .with_context(|| format!("failed to parse WAT from {}", path.display()))?;
        ensure_source_limit(path, "module", bytes_len_u64(module.len()), max_bytes)?;
        Ok(module)
    } else {
        ensure_source_limit(path, "module", bytes_len_u64(bytes.len()), max_bytes)?;
        Ok(bytes)
    }
}

fn read_capped_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to stat {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("{} source path is not a regular file", path.display());
    }
    ensure_source_limit(path, "source", metadata.len(), max_bytes)?;
    let mut limited = file.take(max_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    ensure_source_limit(path, "source", bytes_len_u64(bytes.len()), max_bytes)?;
    Ok(bytes)
}

fn ensure_source_limit(path: &Path, field: &'static str, actual: u64, limit: u64) -> Result<()> {
    if actual > limit {
        bail!(
            "{} {field} is too large: {actual} bytes exceeds source byte limit {limit} bytes",
            path.display()
        );
    }
    Ok(())
}

fn bytes_len_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

fn wasm_module_byte_limit(policy_memory_bytes: u64) -> u64 {
    policy_memory_bytes.min(MAX_WASM_MODULE_BYTES)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_file_reader_rejects_oversized_source() -> Result<()> {
        let path = temp_path("reader-over-limit.wat");
        fs::write(&path, b"(module)")?;

        let error = match read_capped_file(&path, 1) {
            Ok(_) => bail!("oversized source unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("source is too large"));
        fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn compile_rejects_oversized_source_before_writing_output() -> Result<()> {
        let input = temp_path("compile-over-limit.wat");
        let output = temp_path("compile-over-limit.wasm");
        fs::write(&input, b"(module)")?;
        fs::remove_file(&output).ok();

        let error = match compile_with_limit(&input, &output, 1) {
            Ok(_) => bail!("oversized compile unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("source is too large"));
        assert!(!output.exists());
        fs::remove_file(input).ok();
        fs::remove_file(output).ok();
        Ok(())
    }

    fn temp_path(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!("beatbox-{name}-{}-{unique}", std::process::id()))
    }
}
