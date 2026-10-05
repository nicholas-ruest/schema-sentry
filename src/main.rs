#![forbid(unsafe_code)]

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use schema_sentry::{Report, Severity, compare};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Detect backward-incompatible JSON Schema changes",
    long_about = "Compare an OLD schema with a proposed NEW schema. Exit 0 means backward compatible, exit 2 means breaking changes were found, and exit 1 means the comparison could not run."
)]
struct Args {
    /// Previously published JSON Schema
    old: PathBuf,

    /// Proposed replacement JSON Schema
    new: PathBuf,

    /// Report format
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    /// Print the report without exiting 2 when breaking changes exist
    #[arg(long)]
    allow_breaking: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let old =
        read_schema(&args.old).with_context(|| format!("old schema: {}", args.old.display()))?;
    let new =
        read_schema(&args.new).with_context(|| format!("new schema: {}", args.new.display()))?;
    let report = compare(&old, &new);

    match args.format {
        OutputFormat::Text => print_text(&report),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }

    if !report.compatible && !args.allow_breaking {
        std::process::exit(2);
    }
    Ok(())
}

fn read_schema(path: &Path) -> Result<Value> {
    let input = std::fs::read_to_string(path).context("could not read file")?;
    serde_json::from_str(&input).context("invalid JSON")
}

fn print_text(report: &Report) {
    if report.compatible {
        println!("compatible");
    } else {
        println!(
            "incompatible ({} breaking change(s))",
            report.breaking_changes
        );
    }

    for finding in &report.findings {
        let label = match finding.severity {
            Severity::Breaking => "BREAKING",
            Severity::Warning => "WARNING",
        };
        println!(
            "{label} {} [{}] {}",
            finding.path, finding.rule, finding.message
        );
    }
}
