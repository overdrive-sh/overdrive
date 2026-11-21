//! GH #372 — the operator path of `overdrive deploy <SPEC>` for a workload's `[resources]`:
//! `cpu = "500m"` and `memory = "128MiB"` are understood, the removed `cpu_milli` / `memory_bytes`
//! keys and malformed values are refused at the command handler before any server call, and the
//! refusal text names the key or the grammar (US-1, US-2; D-1, D-4; DD-8).
//!
//! Driven through the real command handler `overdrive_cli::commands::deploy::deploy` called as a
//! library function (per `crates/overdrive-cli/CLAUDE.md`: no subprocess). The refusal happens
//! client-side, so no server is composed: the config path names a file that does not exist, which
//! also makes "the spec passed local validation" observable as "the failure is not a spec failure".
//!
//! Each spec shape an operator writes is covered: the `[job]` and `[service]` forms and the flat
//! `id =` form (live: `examples/coinflip.toml` is written that way). Evidence tier: unit / in-memory acceptance (layer 2); sad paths are named
//! examples (Mandate 11). Pending bodies are complete and marked `#[should_panic(expected = ...)]`;
//! DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::path::PathBuf;

use overdrive_cli::commands::deploy::{DeployArgs, deploy};
use overdrive_cli::http_client::CliError;
use overdrive_testing::placement;
use tempfile::TempDir;

placement!(host_pure);

const VM: &str =
    "[vm]\ncommand = \"/bin/true\"\nargs = []\nkernel = \"/kernel\"\nrootfs = \"/rootfs\"\n";

/// The three spec shapes an operator writes, each with the given `[resources]` body.
fn shapes(resources: &str) -> Vec<(&'static str, String)> {
    vec![
        ("[job]", format!("[job]\nid = \"payments\"\n\n{VM}\n[resources]\n{resources}\n")),
        (
            "[service]",
            format!(
                "[service]\nid = \"dev-box\"\nreplicas = 1\n\n[[listener]]\nport = 8080\nprotocol = \"tcp\"\n\n{VM}\n[resources]\n{resources}\n"
            ),
        ),
        ("flat", format!("id = \"payments\"\nreplicas = 1\n\n[resources]\n{resources}\n\n{VM}")),
    ]
}

/// Run `deploy` on `spec_text` with a config path that does not exist.
async fn run_deploy(spec_text: &str) -> Result<(), CliError> {
    let tmp = TempDir::new().expect("tempdir");
    let spec: PathBuf = tmp.path().join("spec.toml");
    std::fs::write(&spec, spec_text).expect("write spec");
    let config_path = tmp.path().join("no-such-config");
    deploy(DeployArgs { spec, config_path }).await.map(|_| ())
}

/// The operator-visible text of a spec refusal: either a typed `InvalidSpec` (field + message) or the
/// parser's own error, whichever the handler surfaces. Anything else is not a spec refusal.
fn spec_refusal(outcome: Result<(), CliError>, what: &str) -> String {
    match outcome {
        Err(e @ (CliError::InvalidSpec { .. } | CliError::ParseError(_))) => e.to_string(),
        Err(other) => panic!(
            "the spec must be refused as a spec error before any server call ({what}); the handler returned {other:?}"
        ),
        Ok(()) => panic!(
            "the spec must be refused as a spec error before any server call ({what}); it succeeded"
        ),
    }
}

/// A spec refusal names the offending key, never points the operator at a removed key, and carries
/// the wording DESIGN pins for the rejection class when it pins one (compared case-insensitively).
fn assert_refusal(shape: &str, what: &str, message: &str, key: &str, keyword: Option<&str>) {
    let message = message.to_lowercase();
    assert!(
        message.contains(key),
        "the refusal must name the offending key {key} ({shape}, {what}): {message}"
    );
    assert!(
        !message.contains("cpu_milli") && !message.contains("memory_bytes"),
        "the refusal must not point the operator at a removed key ({shape}, {what}): {message}"
    );
    if let Some(keyword) = keyword {
        assert!(
            message.contains(&keyword.to_lowercase()),
            "the refusal must carry the class wording {keyword:?} ({shape}, {what}): {message}"
        );
    }
}

// ----- D-1: the removed keys ---------------------------------------------------------------------

/// AC-1.2 / AC-2.2 / D-1: `cpu_milli` and `memory_bytes` are refused at `deploy` for every spec shape, the
/// refusal names the unknown key and the expected `cpu` and `memory`, and nothing reaches the server
/// (the config does not exist, so reaching the server would be a different error).
/// CONTRACT_SHAPE: pure-function (spec text in, typed refusal out).
#[tokio::test]
async fn sad_deploy_refuses_the_removed_keys_naming_the_unknown_key() {
    for (body, unknown) in [
        ("cpu_milli = 500\nmemory_bytes = 134217728", "cpu_milli"),
        ("cpu = \"500m\"\nmemory = \"128MiB\"\ncpu_milli = 500", "cpu_milli"),
        ("cpu = \"500m\"\nmemory_bytes = 134217728", "memory_bytes"),
    ] {
        for (shape, text) in shapes(body) {
            let message =
                spec_refusal(run_deploy(&text).await, &format!("{shape}, removed key {unknown}"));
            assert!(
                message.contains(unknown),
                "the spec must be refused as a spec error before any server call: the refusal must name the unknown key {unknown} ({shape}): {message}"
            );
            assert!(
                message.contains("`cpu`") && message.contains("`memory`"),
                "the spec must be refused as a spec error before any server call: the refusal must name the expected keys cpu and memory ({shape}): {message}"
            );
        }
    }
}

// ----- D-4 / AC-1.3 / AC-2.3: malformed values ------------------------------------------------------

/// AC-1.3 / D-4: a malformed or unit-less memory is refused at `deploy` in every spec shape, naming the
/// `memory` key and, where DESIGN pins wording, that wording (the binary units for a decimal unit, "smaller"
/// for a fractional count). One representative per shape; the full class matrix is the parser test.
/// A bare number is never silently read as bytes. CONTRACT_SHAPE: pure-function.
#[tokio::test]
async fn sad_deploy_refuses_malformed_memory_naming_the_expected_form() {
    let by_shape = [
        ("[job]", "memory = \"128MB\"", Some("MiB")),
        ("[service]", "memory = 134217728", None),
        ("flat", "memory = \"1.5MiB\"", Some("smaller")),
    ];
    for (shape, memory, keyword) in by_shape {
        let text = shapes(&format!("cpu = \"500m\"\n{memory}"))
            .into_iter()
            .find_map(|(s, t)| (s == shape).then_some(t))
            .expect("the shape exists");
        let message = spec_refusal(run_deploy(&text).await, &format!("{shape}, {memory}"));
        assert_refusal(shape, memory, &message, "memory", keyword);
    }
}

/// AC-2.3: a malformed cpu is refused at `deploy` in every spec shape, naming the `cpu` key and, where
/// DESIGN pins wording, that wording ("lowercase" for an uppercase milli suffix, "three" for four
/// fractional digits). One representative per shape.
/// CONTRACT_SHAPE: pure-function.
#[tokio::test]
async fn sad_deploy_refuses_malformed_cpu_naming_the_expected_grammar() {
    let by_shape = [
        ("[job]", "cpu = \"500M\"", Some("lowercase")),
        ("[service]", "cpu = \"1.0005\"", Some("three")),
        ("flat", "cpu = \"five hundred\"", None),
    ];
    for (shape, cpu, keyword) in by_shape {
        let text = shapes(&format!("{cpu}\nmemory = \"128MiB\""))
            .into_iter()
            .find_map(|(s, t)| (s == shape).then_some(t))
            .expect("the shape exists");
        let message = spec_refusal(run_deploy(&text).await, &format!("{shape}, {cpu}"));
        assert_refusal(shape, cpu, &message, "cpu", keyword);
    }
}

// ----- DD-8: the whole-MiB admission rule at the handler --------------------------------------------

/// DD-8 / AC-1.3: zero memory and memory that is not a whole number of MiB are refused at `deploy` with the
/// field `memory`, naming the rule ("non-zero", "whole number of MiB") before any server call.
/// `0MiB` is a valid size in the grammar and is refused by admission. CONTRACT_SHAPE: pure-function.
#[tokio::test]
async fn sad_deploy_refuses_zero_and_sub_mib_aligned_memory_naming_the_memory_field() {
    for (memory, needle) in
        [("0MiB", "non-zero"), ("1000B", "whole number of MiB"), ("1536KiB", "whole number of MiB")]
    {
        for (shape, text) in shapes(&format!("cpu = \"500m\"\nmemory = \"{memory}\"")) {
            match run_deploy(&text).await {
                Err(CliError::InvalidSpec { field, message }) => {
                    assert_eq!(
                        field, "memory",
                        "the refusal must name the memory field ({shape}, {memory})"
                    );
                    assert!(
                        message.contains(needle),
                        "the refusal must name the rule {needle} ({shape}, {memory}): {message}"
                    );
                }
                other => panic!(
                    "the refusal must name the memory field: {shape}, {memory} must be an InvalidSpec; the handler returned {other:?}"
                ),
            }
        }
    }
}

// ----- the accepted spellings pass local validation ----------------------------------------------------

/// AC-1.1 / AC-2.1: a spec declaring `cpu = "500m"` and `memory = "128MiB"` (and other accepted spellings, zero
/// CPU, a sub-core quantity) passes the handler's local validation for every spec shape: the failure,
/// if any, is the missing operator config (a transport / config error), never a spec refusal.
/// CONTRACT_SHAPE: pure-function.
#[tokio::test]
async fn a_spec_in_the_new_shape_passes_local_validation() {
    for (cpu, memory) in
        [("500m", "128MiB"), ("0.5", "1024MiB"), ("2", "1GiB"), ("0", "1MiB"), ("1.5", "1536MiB")]
    {
        for (shape, text) in shapes(&format!("cpu = \"{cpu}\"\nmemory = \"{memory}\"")) {
            let outcome = run_deploy(&text).await;
            assert!(
                !matches!(outcome, Err(CliError::InvalidSpec { .. } | CliError::ParseError(_))),
                "an accepted spec must pass local validation ({shape}, {cpu} / {memory}): {outcome:?}"
            );
        }
    }
}
