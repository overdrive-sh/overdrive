//! The runner's command line (DDD-9): `run --family <FAMILY> --results-root <ABSOLUTE PATH>`.
//!
//! Cargo appends `--bench` to a bench target's arguments; it is accepted and ignored. Every
//! other deviation is [`Error::Usage`] (exit 64): the closed `metal run --benchmark` command can
//! only produce a well-formed line, so a refusal here is a direct invocation's.

use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

const USAGE: &str = "run --family <FAMILY> --results-root <ABSOLUTE PATH>";
const CARGO_BENCH_FLAG: &str = "--bench";

/// A parsed runner command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub(crate) family: String,
    results_root: PathBuf,
}

impl Args {
    pub fn parse(arguments: &[String]) -> Result<Self> {
        let mut words = arguments.iter().map(String::as_str).filter(|word| *word != CARGO_BENCH_FLAG);
        if words.next() != Some("run") {
            return Err(usage(format!("expected the `run` command; usage: {USAGE}")));
        }
        let mut family = None;
        let mut results_root = None;
        while let Some(flag) = words.next() {
            let slot = match flag {
                "--family" => &mut family,
                "--results-root" => &mut results_root,
                other => return Err(usage(format!("unknown argument `{other}`; usage: {USAGE}"))),
            };
            let value = words
                .next()
                .ok_or_else(|| usage(format!("`{flag}` needs a value; usage: {USAGE}")))?;
            if slot.replace(value).is_some() {
                return Err(usage(format!("`{flag}` is given more than once; usage: {USAGE}")));
            }
        }
        let family =
            family.ok_or_else(|| usage(format!("`--family` is required; usage: {USAGE}")))?;
        let results_root = results_root
            .ok_or_else(|| usage(format!("`--results-root` is required; usage: {USAGE}")))?;
        let results_root = PathBuf::from(results_root);
        if !results_root.is_absolute() {
            return Err(usage(format!("--results-root `{}` is not an absolute path", results_root.display())));
        }
        Ok(Self { family: family.to_owned(), results_root })
    }

    /// The results root, which must lie inside `source_root` (the checkout the run is bound to).
    pub fn results_root_within(&self, source_root: &Path) -> Result<&Path> {
        let escapes = self.results_root.components().any(|part| part == Component::ParentDir);
        if escapes || !self.results_root.starts_with(source_root) {
            return Err(usage(format!(
                "--results-root `{}` is not inside the checkout `{}`",
                self.results_root.display(),
                source_root.display()
            )));
        }
        Ok(&self.results_root)
    }
}

const fn usage(reason: String) -> Error {
    Error::Usage { reason }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn reason(line: &str) -> String {
        match Args::parse(&words(line)) {
            Err(Error::Usage { reason }) => reason,
            other => panic!("expected a usage error, got {other:?}"),
        }
    }

    #[test]
    fn a_well_formed_line_parses() {
        let args = Args::parse(&words("run --family connect-hook --results-root /r/benchmarks/results"))
            .expect("parses");
        assert_eq!(args.family, "connect-hook");
        assert_eq!(args.results_root, PathBuf::from("/r/benchmarks/results"));
    }

    #[test]
    fn the_flag_cargo_injects_is_tolerated_wherever_it_falls() {
        for line in [
            "run --family f --results-root /r --bench",
            "--bench run --family f --results-root /r",
            "run --family f --bench --results-root /r",
        ] {
            assert!(Args::parse(&words(line)).is_ok(), "{line}");
        }
    }

    #[test]
    fn a_relative_results_root_is_a_usage_error() {
        assert!(reason("run --family f --results-root benchmarks/results").contains("not an absolute path"));
    }

    #[test]
    fn a_root_outside_the_checkout_or_climbing_out_of_it_is_a_usage_error() {
        let args = Args::parse(&words("run --family f --results-root /elsewhere/results")).expect("parses");
        assert!(matches!(args.results_root_within(Path::new("/r")), Err(Error::Usage { .. })));
        let args = Args::parse(&words("run --family f --results-root /r/../elsewhere")).expect("parses");
        assert!(matches!(args.results_root_within(Path::new("/r")), Err(Error::Usage { .. })));
        let args = Args::parse(&words("run --family f --results-root /r/benchmarks/results")).expect("parses");
        assert!(args.results_root_within(Path::new("/r")).is_ok());
    }

    #[test]
    fn malformed_lines_are_usage_errors_naming_the_problem() {
        assert!(reason("").contains("`run` command"));
        assert!(reason("go --family f --results-root /r").contains("`run` command"));
        assert!(reason("run --family f").contains("`--results-root` is required"));
        assert!(reason("run --results-root /r").contains("`--family` is required"));
        assert!(reason("run --family").contains("needs a value"));
        assert!(reason("run --family f --family g --results-root /r").contains("more than once"));
        assert!(reason("run --family f --results-root /r --seed 1").contains("unknown argument `--seed`"));
    }
}
