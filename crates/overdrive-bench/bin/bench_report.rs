//! `bench-report validate|render <record>`: host-independent (DDD-1, DDD-6).
//!
//! Exit statuses: 0 success, 64 usage, 65 invalid record, 70 the record is unreadable.
//! `render` validates the record, then prints the report to stdout.

#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "the CLI reports its refusals on stderr and the report on stdout"
)]

use std::path::PathBuf;
use std::process::ExitCode;

use overdrive_bench::record::validate;
use overdrive_bench::report::render;
use overdrive_bench::{Error, Result};

const USAGE: &str = "bench-report validate <record> | bench-report render <record>";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bench-report: {}", describe(&error));
            ExitCode::from(status(&error))
        }
    }
}

fn run(arguments: &[String]) -> Result<()> {
    let [subcommand, rest @ ..] = arguments else {
        return Err(usage(format!("no subcommand; {USAGE}")));
    };
    if !matches!(subcommand.as_str(), "validate" | "render") {
        return Err(usage(format!("unknown subcommand `{subcommand}`; {USAGE}")));
    }
    let [path] = rest else {
        return Err(usage(format!("`{subcommand}` takes exactly one record path; {USAGE}")));
    };
    let path = PathBuf::from(path);
    let text = std::fs::read_to_string(&path).map_err(|source| Error::Record { path, source })?;
    if subcommand == "validate" {
        return validate(&text);
    }
    print!("{}", render(&text)?);
    Ok(())
}

const fn usage(reason: String) -> Error {
    Error::Usage { reason }
}

const fn status(error: &Error) -> u8 {
    match error {
        Error::Usage { .. } => 64,
        Error::InvalidRecord { .. } => 65,
        _ => 70,
    }
}

/// The error as one line: the `Display` of `Error::Record` already names the path and its cause.
fn describe(error: &Error) -> String {
    error.to_string()
}
