use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use beater_connect::{demo_app, ConnectBundle};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return Ok(());
    };

    match command.as_str() {
        "demo" => {
            let mut out = PathBuf::from(".agent");
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--out" => {
                        let Some(value) = args.next() else {
                            return Err("--out requires a path".to_string());
                        };
                        out = PathBuf::from(value);
                    }
                    other => return Err(format!("unknown demo argument: {other}")),
                }
            }
            write_bundle(&out, &demo_app().emit_bundle())?;
            println!("generated Beater Connect demo bundle at {}", out.display());
            Ok(())
        }
        "print" => {
            let Some(surface) = args.next() else {
                return Err("print requires one surface name".to_string());
            };
            let bundle = demo_app().emit_bundle();
            match surface.as_str() {
                "beater" => print!("{}", bundle.beater_manifest),
                "agent-card" => print!("{}", bundle.agent_card),
                "openapi" => print!("{}", bundle.openapi),
                "mcp" => print!("{}", bundle.mcp_catalog),
                "llms" => print!("{}", bundle.llms),
                "robots" => print!("{}", bundle.robots),
                "sitemap" => print!("{}", bundle.sitemap),
                other => return Err(format!("unknown surface: {other}")),
            }
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn write_bundle(out: &Path, bundle: &ConnectBundle) -> Result<(), String> {
    fs::create_dir_all(out.join(".well-known")).map_err(|error| error.to_string())?;
    fs::create_dir_all(out.join("mcp")).map_err(|error| error.to_string())?;

    write_file(
        &out.join(".well-known").join("beater.json"),
        &bundle.beater_manifest,
    )?;
    write_file(
        &out.join(".well-known").join("agent-card.json"),
        &bundle.agent_card,
    )?;
    write_file(&out.join("openapi.json"), &bundle.openapi)?;
    write_file(&out.join("mcp").join("catalog.json"), &bundle.mcp_catalog)?;
    write_file(&out.join("llms.txt"), &bundle.llms)?;
    write_file(&out.join("robots.txt"), &bundle.robots)?;
    write_file(&out.join("sitemap.xml"), &bundle.sitemap)?;
    Ok(())
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn print_help() {
    println!(
        "beater-connect\n\nUSAGE:\n  beater-connect demo [--out DIR]\n  beater-connect print <beater|agent-card|openapi|mcp|llms|robots|sitemap>\n"
    );
}
