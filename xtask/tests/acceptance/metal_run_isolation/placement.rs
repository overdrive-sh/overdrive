//! P3/P4 adapter boundary. The fixture executable is an external attempted body,
//! not a guest substitute. No test here credits fabricated GuestKit execution.
use super::support::*;
use serde_json::Value;
use std::fs;
use std::process::Command;

const PURE_BINARY: &str = "overdrive-cli";
const PURE_TEST: &str = "http_client::tests::category_strings_are_leak_free_by_construction";
const NATIVE_BINARY: &str = "overdrive-control-plane";
const NATIVE_TEST: &str = "guest_network::flow_io::accept_path_tier3::a_repair_of_a_held_listener_keeps_its_socket_and_its_port";
const GUEST_BINARY: &str = "overdrive-bpf::integration";
const GUEST_TEST: &str =
    "integration::xdp_pass_test_run::bpf_unit_runs_xdp_pass_triptych_via_bpf_prog_test_run";

#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "Bash parameter expansion in the external body fixture"
)]
fn body(fixture: &Fixture) -> std::path::PathBuf {
    let path = fixture.source.join("attempt-body");
    executable(
        &path,
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\0' "$0" "$@" > "$METAL_FIXTURE_BODY_ARGS"
printf 'PROPTEST_CASES=%s\nPROPTEST_RNG_SEED=%s\nRUST_BACKTRACE=%s\nNEXTEST_ATTEMPT=%s\nNEXTEST_BINARY_ID=%s\nNEXTEST_TEST_NAME=%s\n' "${PROPTEST_CASES:-}" "${PROPTEST_RNG_SEED:-}" "${RUST_BACKTRACE:-}" "${NEXTEST_ATTEMPT:-}" "${NEXTEST_BINARY_ID:-}" "${NEXTEST_TEST_NAME:-}" > "$METAL_FIXTURE_BODY_ENV"
readlink /proc/self/ns/net > "$METAL_FIXTURE_BODY_NETNS"
cat /proc/sys/net/vsock/ns_mode > "$METAL_FIXTURE_BODY_MODE" 2>/dev/null || true
exit 255
"#,
    );
    path
}

fn wrapper(fixture: &Fixture, binary: &str, name: &str) -> Command {
    fixture.bound_context_inputs();
    let context_id = fixture.context_id();
    let tmp = fixture.source.join("target/metal-run-tmp").join(&context_id);
    fs::create_dir_all(&tmp).expect("already-prepared command context input");
    let dispatcher = if binary == NATIVE_BINARY {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| workspace_file("target"), std::path::PathBuf::from);
        let original = target.join("debug/overdrive-fwd-guest-suite");
        let staged = fixture.source.join("target/prepared/overdrive-fwd-guest-suite");
        fs::create_dir_all(staged.parent().expect("prepared executable parent"))
            .expect("native helper input directory");
        fs::copy(&original,&staged).unwrap_or_else(|error|panic!("prepare the existing dispatcher before activating native adapter cases: {}: {error}",original.display()));
        staged
    } else {
        fixture.source.join("target/debug/absent-dispatcher")
    };
    executable(
        &fixture.bin.join("cargo"),
        &format!(
            "#!/usr/bin/env bash\nprintf forbidden > '{}'\nexit 95\n",
            fixture.temp.path().join("on-demand-cargo").display()
        ),
    );
    let mut command = Command::new("timeout");
    command
        .args(["--kill-after=1s", "5s", "bash"])
        .arg(fixture.source.join("infra/metal/test-wrapper.sh"));
    fixture.environment(&mut command);
    command
        .env("OVERDRIVE_METAL_RUN_ID", &context_id)
        .env("OVERDRIVE_METAL_EXECUTION_CLASS", "ordinary")
        .env("OVERDRIVE_METAL_SOURCE_ROOT", &fixture.source)
        .env("OVERDRIVE_METAL_TEST_DISPATCHER", dispatcher)
        .env("NEXTEST_WORKSPACE_ROOT", &fixture.source)
        .env("CARGO_MANIFEST_DIR", &fixture.source)
        .env("CARGO_TARGET_DIR", fixture.source.join("target"))
        .env("TMPDIR", tmp)
        .env("NEXTEST_BINARY_ID", binary)
        .env("NEXTEST_TEST_NAME", name)
        .env("NEXTEST_ATTEMPT", "2")
        .env("PROPTEST_CASES", "7")
        .env("PROPTEST_RNG_SEED", "123456")
        .env("RUST_BACKTRACE", "1")
        .env("METAL_FIXTURE_BODY_ARGS", fixture.temp.path().join("body.args"))
        .env("METAL_FIXTURE_BODY_ENV", fixture.temp.path().join("body.env"))
        .env("METAL_FIXTURE_BODY_NETNS", fixture.temp.path().join("body.netns"))
        .env("METAL_FIXTURE_BODY_MODE", fixture.temp.path().join("body.mode"));
    command
}

/// CONTRACT_SHAPE: bounded-change (host-pure attempt changes only its output and
/// fixture files, preserving exact argv/env/status with no dispatcher dependency).
#[test]
fn host_pure_attempt_preserves_exact_arguments_environment_and_255_without_a_guest() {
    let fixture = Fixture::new();
    let executable_path = body(&fixture);
    let argv =
        [PURE_TEST, "--exact", "--nocapture", "argument with spaces", "literal;$not-a-shell"];
    let initial_netns = fs::read_link("/proc/self/ns/net").expect("initial namespace");
    let before = snapshot(&fixture.runs);
    let output = wrapper(&fixture, PURE_BINARY, PURE_TEST)
        .arg(&executable_path)
        .args(argv)
        .output()
        .expect("selected host-pure attempt");
    assert_eq!(output.status.code(), Some(255), "{}", String::from_utf8_lossy(&output.stderr));
    let observed_arguments =
        fs::read(fixture.temp.path().join("body.args")).expect("actual attempted argv");
    let expected: Vec<u8> = std::iter::once(executable_path.to_str().expect("path"))
        .chain(argv)
        .flat_map(|arg| arg.bytes().chain(std::iter::once(0)))
        .collect();
    assert_eq!(observed_arguments, expected);
    let environment = text(&fixture.temp.path().join("body.env"));
    for field in
        ["PROPTEST_CASES=7", "PROPTEST_RNG_SEED=123456", "RUST_BACKTRACE=1", "NEXTEST_ATTEMPT=2"]
    {
        assert!(environment.contains(field), "{environment}");
    }
    assert_eq!(
        text(&fixture.temp.path().join("body.netns")).trim(),
        initial_netns.to_str().expect("namespace string")
    );
    assert!(!fixture.source.join("target/debug/absent-dispatcher").exists());
    assert_eq!(snapshot(&fixture.runs), before);
}

/// CONTRACT_SHAPE: bounded-change (outside ordinary Metal context execs the exact
/// original runner chain; placement/dispatcher cannot block non-Metal/exclusive).
#[test]
fn non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain() {
    for context in ["non-metal", "exclusive"] {
        let fixture = Fixture::new();
        let executable_path = body(&fixture);
        let mut command = wrapper(&fixture, "unknown", "unclassified");
        if context == "non-metal" {
            command
                .env_remove("OVERDRIVE_METAL_RUN_ID")
                .env_remove("OVERDRIVE_METAL_EXECUTION_CLASS")
                .env_remove("OVERDRIVE_METAL_SOURCE_ROOT")
                .env_remove("OVERDRIVE_METAL_TEST_DISPATCHER");
        } else {
            command.env("OVERDRIVE_METAL_EXECUTION_CLASS", "exclusive");
        }
        let output = command
            .arg(&executable_path)
            .args(["configured-runner-token", "selected-argument"])
            .output()
            .expect("original exec chain");
        assert_eq!(output.status.code(), Some(255), "{}", String::from_utf8_lossy(&output.stderr));
        let actual = fs::read(fixture.temp.path().join("body.args")).expect("actual exec argv");
        assert!(actual.ends_with(b"configured-runner-token\0selected-argument\0"));
    }
}

/// CONTRACT_SHAPE: bounded-change (malformed ordinary context/placement may emit
/// only typed refusal; no selected body, guest preparation or remote mutation).
#[test]
fn sad_invalid_context_missing_identity_and_missing_guest_dispatcher_never_run_the_body() {
    for state in [
        "short-id",
        "uppercase-id",
        "bad-class",
        "foreign-source",
        "missing-identity",
        "missing-name",
        "guest-not-prepared",
    ] {
        let fixture = Fixture::new();
        let executable_path = body(&fixture);
        let mut command = if state == "guest-not-prepared" {
            wrapper(&fixture, GUEST_BINARY, GUEST_TEST)
        } else {
            wrapper(&fixture, PURE_BINARY, PURE_TEST)
        };
        let (code, field) = match state {
            "short-id" => {
                command.env("OVERDRIVE_METAL_RUN_ID", "0");
                ("invalid_run_context", Some("run_id"))
            }
            "uppercase-id" => {
                command.env("OVERDRIVE_METAL_RUN_ID", "ABCDEF0123456789abcdef01");
                ("invalid_run_context", Some("run_id"))
            }
            "bad-class" => {
                command.env("OVERDRIVE_METAL_EXECUTION_CLASS", "implicit-exclusive");
                ("invalid_run_context", Some("execution_class"))
            }
            "foreign-source" => {
                command.env("OVERDRIVE_METAL_SOURCE_ROOT", fixture.remote.join("foreign"));
                ("invalid_run_context", Some("source_owner"))
            }
            "missing-identity" => {
                command.env("NEXTEST_BINARY_ID", "unknown-binary");
                ("test_unclassified", None)
            }
            "missing-name" => {
                command.env_remove("NEXTEST_TEST_NAME");
                ("test_unclassified", None)
            }
            _ => ("lane_not_prepared", None),
        };
        let output =
            command.arg(&executable_path).arg("--exact").output().expect("refused attempt");
        test_error(&output, code, 78);
        if let Some(field) = field {
            assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("field={field}")));
        }
        assert!(
            !fixture.temp.path().join("body.args").exists(),
            "a refusal must precede selected-body execution"
        );
        assert!(
            !fixture.temp.path().join("on-demand-cargo").exists(),
            "wrapper cannot build missing dispatcher on demand"
        );
    }
}

/// CONTRACT_SHAPE: bounded-change (closed exact-identity placement refuses malformed
/// or duplicate data; source/retained evidence and attempted-body complement unchanged).
#[test]
fn sad_invalid_duplicate_and_ambiguous_placement_is_fail_closed() {
    for state in [
        "empty",
        "unknown-kind",
        "unknown-field",
        "unknown-group",
        "duplicate",
        "missing",
        "operator-fixture",
    ] {
        let fixture = Fixture::new();
        let executable_path = body(&fixture);
        let descriptor = match state {
            "empty" => "{}",
            "unknown-kind" => r#"{"kind":"guess-host"}"#,
            "unknown-field" => r#"{"kind":"host-pure","fallback":true}"#,
            "unknown-group" => r#"{"kind":"guest-group","group":"new-group"}"#,
            "operator-fixture" => r#"{"kind":"guest-lane-operator","fixture":"unqualified-leaf"}"#,
            _ => r#"{"kind":"host-pure"}"#,
        };
        let entry = format!("\"{PURE_TEST}\":{descriptor}");
        let entries = match state {
            "duplicate" => format!("{entry},{entry}"),
            "missing" => String::new(),
            _ => entry,
        };
        fs::write(fixture.source.join("infra/metal/test-placement.json"), format!(r#"{{"format":"overdrive-metal-test-placement-v1","placements":{{"{PURE_BINARY}":{{{entries}}}}}}}"#)).expect("placement input");
        let before = snapshot(&fixture.runs);
        let output = wrapper(&fixture, PURE_BINARY, PURE_TEST)
            .arg(executable_path)
            .output()
            .expect("invalid placement attempt");
        test_error(
            &output,
            match state {
                "duplicate" => "placement_ambiguous",
                "missing" => "test_unclassified",
                _ => "placement_invalid",
            },
            78,
        );
        assert!(!fixture.temp.path().join("body.args").exists());
        assert_eq!(snapshot(&fixture.runs), before);
    }
}

/// CONTRACT_SHAPE: bounded-change (launcher anonymous local-vsock namespace and
/// child process only; initial namespace modes and physical kernel complement stay unchanged).
#[test]
fn native_launcher_gets_a_local_vsock_namespace_without_changing_the_initial_mode() {
    let fixture = Fixture::new_accessible();
    let executable_path = body(&fixture);
    let initial_namespace = fs::read_link("/proc/self/ns/net").expect("initial namespace");
    let initial_modes = ["/proc/sys/net/vsock/ns_mode", "/proc/sys/net/vsock/child_ns_mode"]
        .map(|path| fs::read(path).expect("Linux 7 vsock modes"));
    let output = wrapper(&fixture, NATIVE_BINARY, NATIVE_TEST)
        .arg(executable_path)
        .output()
        .expect("native launcher attempt");
    assert_eq!(output.status.code(), Some(255), "{}", String::from_utf8_lossy(&output.stderr));
    assert_ne!(
        text(&fixture.temp.path().join("body.netns")).trim(),
        initial_namespace.to_str().expect("namespace")
    );
    assert_eq!(text(&fixture.temp.path().join("body.mode")).trim(), "local");
    assert_eq!(
        ["/proc/sys/net/vsock/ns_mode", "/proc/sys/net/vsock/child_ns_mode"]
            .map(|path| fs::read(path).expect("initial modes after")),
        initial_modes
    );
}

/// CONTRACT_SHAPE: bounded-change (capability-denied parent namespace produces
/// exact typed failure before a body; no global CID or raw-host fallback).
#[test]
fn sad_namespace_permission_denial_never_falls_back_to_the_host() {
    let fixture = Fixture::new_accessible();
    let executable_path = body(&fixture);
    let mut inner = wrapper(&fixture, NATIVE_BINARY, NATIVE_TEST);
    inner.arg(executable_path);
    let mut command = Command::new("setpriv");
    command
        .args(["--bounding-set=-sys_admin", "--inh-caps=-all", "--ambient-caps=-all"])
        .arg(inner.get_program())
        .args(inner.get_args());
    for (key, value) in inner.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    let output = command.output().expect("capability-denied namespace attempt");
    test_error(&output, "namespace_setup_failed", 70);
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("step=parent") && diagnostic.contains("errno=1"), "{diagnostic}");
    assert!(!fixture.temp.path().join("body.args").exists());
}

const NATIVE_SERVE_CONFORMANCE_BINARY: &str =
    "overdrive-system-conformance::public_ingress_gateway_end_to_end";

fn test_function_exists(row: &Value) -> bool {
    let source = row["source"].as_str().expect("source");
    let path = source.split(':').next().expect("source path");
    let leaf = row["test_name"].as_str().expect("name").rsplit("::").next().expect("leaf");
    fs::read_to_string(workspace_file(path)).is_ok_and(|text| text.contains(&format!("fn {leaf}(")))
}

/// Whether the test function carries `#[overdrive_test(guest = "<group>")]`
/// for exactly the reviewed guest group: a test placed on itself has no table
/// row, and the declaration must name the same group the reviewed row did.
fn declares_the_reviewed_guest_group_on_the_test(row: &Value) -> bool {
    let source = row["source"].as_str().expect("source");
    let path = source.split(':').next().expect("source path");
    let leaf = row["test_name"].as_str().expect("name").rsplit("::").next().expect("leaf");
    let group = row["context"].as_str().expect("guest group");
    let declaration = format!("#[overdrive_testing::overdrive_test(guest = \"{group}\")]");
    let Ok(text) = fs::read_to_string(workspace_file(path)) else { return false };
    let Some(function_at) = text.find(&format!("fn {leaf}(")) else { return false };
    text[..function_at]
        .lines()
        .rev()
        .skip(1)
        .take_while(|line| {
            let line = line.trim_start();
            line.starts_with("#[") || line.starts_with("//")
        })
        .any(|line| line.trim() == declaration)
}

/// CONTRACT_SHAPE: bounded-change (read-only placement comparison and output only;
/// data extensions are exact identities, never absence-means-host or fixed counts).
#[test]
fn runtime_placement_preserves_the_reviewed_partition_and_declares_new_acceptance_identities() {
    let design: Value = serde_json::from_str(&text(&workspace_file(
        "docs/feature/metal-run-isolation/design/test-placement-inventory.json",
    )))
    .expect("reviewed inventory");
    let runtime: Value =
        serde_json::from_str(&text(&workspace_file("infra/metal/test-placement.json")))
            .expect("runtime placement");
    assert_eq!(runtime["format"], "overdrive-metal-test-placement-v1");
    for row in design["rows"].as_array().expect("reviewed rows") {
        let binary = row["binary_id"].as_str().expect("binary identity");
        let name = row["test_name"].as_str().expect("exact identity");
        let actual = &runtime["placements"][binary][name];
        // A reviewed row whose test function was deleted from the workspace has
        // no runtime placement to preserve; a row whose function still exists must.
        if actual.is_null() && !test_function_exists(row) {
            continue;
        }
        // A guest test that declares its placement on itself has no table row.
        if actual.is_null()
            && row["placement"] == "guest-group"
            && declares_the_reviewed_guest_group_on_the_test(row)
        {
            continue;
        }
        // The public-ingress conformance binary boots `serve` in-process on the
        // node's KVM, so it was reclassified from the reviewed guest group to the
        // native launcher (per-run node view). That boundary is asserted exactly.
        if binary == NATIVE_SERVE_CONFORMANCE_BINARY && row["placement"] == "guest-group" {
            assert_eq!(actual["kind"], "native-launcher", "changed boundary for {binary}::{name}");
            continue;
        }
        assert_eq!(actual["kind"], row["placement"], "changed boundary for {binary}::{name}");
        match row["placement"].as_str().expect("kind") {
            "guest-group" => assert_eq!(actual["group"], row["context"]),
            "guest-lane-owner" | "guest-lane-operator" => assert_eq!(
                actual["fixture"],
                format!(
                    "integration::in_guest_lane::{}",
                    row["context"].as_str().expect("fixture")
                )
            ),
            _ => {}
        }
    }
    let additions: Value = serde_json::from_str(&text(&workspace_file(
        "docs/feature/metal-run-isolation/distill/test-placement-additions.json",
    )))
    .expect("DISTILL placement additions");
    for row in additions["rows"].as_array().expect("new rows") {
        assert_eq!(
            runtime["placements"][row["binary_id"].as_str().expect("binary")]
                [row["test_name"].as_str().expect("name")],
            row["descriptor"]
        );
    }
}

/// CONTRACT_SHAPE: bounded-change (fresh fixture mount/user namespaces and actual
/// syscall refusals only; physical namespace modes and selected body remain unchanged).
#[test]
fn sad_each_later_namespace_operation_keeps_its_exact_step_and_errno() {
    for (step, errno) in [("child_mode", "30"), ("local", "28")] {
        let fixture = Fixture::new_accessible();
        let executable_path = body(&fixture);
        let inaccessible = fixture.temp.path().join("namespace-fixture-input");
        let mut inner = wrapper(&fixture, NATIVE_BINARY, NATIVE_TEST);
        inner.arg(&executable_path);
        let before = ["/proc/sys/net/vsock/ns_mode", "/proc/sys/net/vsock/child_ns_mode"]
            .map(|path| fs::read(path).expect("caller namespace observation"));
        let mut command = Command::new("unshare");
        if step == "local" {
            command.args(["--user", "--map-root-user"]);
        } else {
            command.args(["--mount", "--propagation", "private"]);
        }
        let denial = fixture.temp.path().join("namespace-denial");
        executable(&denial, &text(&workspace_file("infra/metal/tests/fixtures/namespace-denial")));
        command
            .arg(denial)
            .arg(step)
            .arg(&inaccessible)
            .arg(inner.get_program())
            .args(inner.get_args());
        for (key, value) in inner.get_envs() {
            if let Some(value) = value {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }
        let output = command.output().expect("real namespace resource refusal");
        test_error(&output, "namespace_setup_failed", 70);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains(&format!("step={step}")), "{diagnostic}");
        assert!(diagnostic.contains(&format!("errno={errno}")), "{diagnostic}");
        assert!(!fixture.temp.path().join("body.args").exists());
        assert_eq!(
            ["/proc/sys/net/vsock/ns_mode", "/proc/sys/net/vsock/child_ns_mode"]
                .map(|path| fs::read(path).expect("unchanged caller namespace observation")),
            before
        );
    }
}

/// CONTRACT_SHAPE: bounded-change (real descriptor exhaustion in the owned
/// verification reader only; no syscall result, namespace mode or guest evidence is faked).
#[test]
#[cfg(target_arch = "x86_64")]
fn sad_actual_verification_read_failure_preserves_its_step_and_errno() {
    let fixture = Fixture::new_accessible();
    let executable_path = body(&fixture);
    let mut command = wrapper(&fixture, NATIVE_BINARY, NATIVE_TEST);
    command.arg(executable_path);
    let output = super::namespace_trace::deny_verification_open(&fixture, &command)
        .expect("real owned descriptor exhaustion at verification");
    test_error(&output, "namespace_setup_failed", 70);
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("step=verify") && diagnostic.contains("errno=24"), "{diagnostic}");
    assert!(!fixture.temp.path().join("body.args").exists());
}

/// CONTRACT_SHAPE: bounded-change (observes only the selected disposable kernel
/// and its actual nextest profile context; a raw-host fallback fails before effects).
#[test]
#[ignore = "external Metal acceptance fixture: deliberate selected body status 255"]
fn selected_guest_body_uses_its_own_kernel_in_existing_profiles() {
    use std::io::Write as _;
    let source = std::env::var("RUST_LOG").expect("external physical host identity input");
    let host = source.strip_prefix("metal_host_boot_id=").expect("host identity prefix");
    let guest = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .expect("actual selected kernel identity");
    assert_ne!(guest.trim(), host, "the existing profile cannot omit mandatory placement");
    assert_eq!(
        std::env::var("NEXTEST_TEST_NAME").expect("actual selected identity"),
        "acceptance::metal_run_isolation::placement::selected_guest_body_uses_its_own_kernel_in_existing_profiles"
    );
    writeln!(
        std::io::stdout(),
        "METAL-ISOLATION-PROFILE guest_boot_id={} profile={}",
        guest.trim(),
        std::env::var("NEXTEST_PROFILE").expect("actual nextest profile metadata")
    )
    .expect("actual profile observation");
    std::io::stdout().flush().expect("profile evidence before selected exit");
    std::process::exit(255);
}
