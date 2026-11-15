//! P3/P4 adapter boundary. The fixture executable is an external attempted body,
//! not a guest substitute. No test here credits fabricated GuestKit execution.
use super::support::*;
use overdrive_testing_macros::overdrive_test;
use serde_json::Value;
use std::fs;
use std::process::Command;

use overdrive_testing::placement;

placement!(host_pure);

const PURE_BINARY: &str = "overdrive-cli";
const PURE_TEST: &str = "http_client::tests::category_strings_are_leak_free_by_construction";
const NATIVE_BINARY: &str = "xtask::acceptance";
const NATIVE_TEST: &str = "acceptance::metal_run_isolation::namespace_trace::fixture_exhausts_only_a_real_owned_child_namespace_verification_read";

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

/// The wrapper run for one attempted test. With `placement` the attempt's
/// executable is a copy of `env(1)` carrying that placement section (a fixture
/// body is a script, which carries no record), run with the body script as its
/// first argument; without it the body script is the executable itself.
fn wrapper(fixture: &Fixture, binary: &str, name: &str, placement: Option<Vec<u8>>) -> Command {
    fixture.bound_context_inputs();
    let context_id = fixture.context_id();
    let tmp = fixture.source.join("target/metal-run-tmp").join(&context_id);
    fs::create_dir_all(&tmp).expect("already-prepared command context input");
    let script = body(fixture);
    let attempted: Vec<std::path::PathBuf> = placement.map_or_else(
        || vec![script.clone()],
        |section| {
            let launcher = fixture.source.join("placed-env");
            let original = fs::read("/usr/bin/env").expect("the system env executable");
            fs::write(&launcher, elf_with_appended_placement(&original, &section))
                .expect("placed attempt executable");
            fs::set_permissions(&launcher, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .expect("executable placed attempt");
            vec![launcher, script.clone()]
        },
    );
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
        .env("METAL_FIXTURE_BODY_MODE", fixture.temp.path().join("body.mode"))
        .args(attempted);
    command
}

fn placement_of(kind: &str, test: &str, args: &[&str]) -> Vec<u8> {
    record(kind, &format!("some-crate::{test}"), args)
}

/// CONTRACT_SHAPE: bounded-change (host-pure attempt changes only its output and
/// fixture files, preserving exact argv/env/status with no guest dependency).
#[test]
fn host_pure_attempt_preserves_exact_arguments_environment_and_255_without_a_guest() {
    let fixture = Fixture::new();
    let executable_path = body(&fixture);
    let argv =
        [PURE_TEST, "--exact", "--nocapture", "argument with spaces", "literal;$not-a-shell"];
    let initial_netns = fs::read_link("/proc/self/ns/net").expect("initial namespace");
    let before = snapshot(&fixture.runs);
    let output =
        wrapper(&fixture, PURE_BINARY, PURE_TEST, Some(placement_of("host_pure", PURE_TEST, &[])))
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
    assert_eq!(snapshot(&fixture.runs), before);
}

/// CONTRACT_SHAPE: bounded-change (outside ordinary Metal context execs the exact
/// original runner chain; placement cannot block non-Metal/exclusive).
#[test]
fn non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain() {
    for context in ["non-metal", "exclusive"] {
        let fixture = Fixture::new();
        let mut command = wrapper(&fixture, "unknown", "unclassified", None);
        if context == "non-metal" {
            command
                .env_remove("OVERDRIVE_METAL_RUN_ID")
                .env_remove("OVERDRIVE_METAL_EXECUTION_CLASS")
                .env_remove("OVERDRIVE_METAL_SOURCE_ROOT");
        } else {
            command.env("OVERDRIVE_METAL_EXECUTION_CLASS", "exclusive");
        }
        let output = command
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
fn sad_invalid_context_and_missing_identity_never_run_the_body() {
    for state in
        ["short-id", "uppercase-id", "bad-class", "foreign-source", "unplaced-test", "missing-name"]
    {
        let fixture = Fixture::new();
        let mut command = wrapper(
            &fixture,
            PURE_BINARY,
            PURE_TEST,
            Some(placement_of("host_pure", PURE_TEST, &[])),
        );
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
            "unplaced-test" => {
                command.env("NEXTEST_TEST_NAME", "http_client::tests::no_record_places_this");
                ("test_unclassified", None)
            }
            "missing-name" => {
                command.env_remove("NEXTEST_TEST_NAME");
                ("test_unclassified", None)
            }
            _ => unreachable!("every state is matched above"),
        };
        let output = command.arg("--exact").output().expect("refused attempt");
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
            "wrapper cannot build anything on demand"
        );
    }
}

/// CONTRACT_SHAPE: bounded-change (a placement record that names no known kind or
/// group, or that no record supplies, is refused; source/retained evidence and
/// attempted-body complement unchanged).
#[test]
fn sad_invalid_and_missing_placement_is_fail_closed() {
    for state in ["unknown-kind", "unknown-group", "missing"] {
        let fixture = Fixture::new();
        let owned = format!("some-crate::{PURE_TEST}");
        let placement = match state {
            "unknown-kind" => record("guess_host", &owned, &[]),
            "unknown-group" => record("guest", &owned, &["new-group", "", "0"]),
            _ => record("host_pure", "some-crate::another_module::another_test", &[]),
        };
        let before = snapshot(&fixture.runs);
        let output = wrapper(&fixture, PURE_BINARY, PURE_TEST, Some(placement))
            .output()
            .expect("invalid placement attempt");
        test_error(
            &output,
            match state {
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
#[overdrive_test(native_launcher)]
fn native_launcher_gets_a_local_vsock_namespace_without_changing_the_initial_mode() {
    let fixture = Fixture::new_accessible();
    let initial_namespace = fs::read_link("/proc/self/ns/net").expect("initial namespace");
    let initial_modes = ["/proc/sys/net/vsock/ns_mode", "/proc/sys/net/vsock/child_ns_mode"]
        .map(|path| fs::read(path).expect("Linux 7 vsock modes"));
    let output = wrapper(
        &fixture,
        NATIVE_BINARY,
        NATIVE_TEST,
        Some(placement_of("native_launcher", NATIVE_TEST, &[])),
    )
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
#[overdrive_test(native_launcher)]
fn sad_namespace_permission_denial_never_falls_back_to_the_host() {
    let fixture = Fixture::new_accessible();
    let inner = wrapper(
        &fixture,
        NATIVE_BINARY,
        NATIVE_TEST,
        Some(placement_of("native_launcher", NATIVE_TEST, &[])),
    );
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

fn test_function_exists(row: &Value) -> bool {
    let source = row["source"].as_str().expect("source");
    let path = source.split(':').next().expect("source path");
    let leaf = row["test_name"].as_str().expect("name").rsplit("::").next().expect("leaf");
    fs::read_to_string(workspace_file(path)).is_ok_and(|text| text.contains(&format!("fn {leaf}(")))
}

/// Whether the test function carries `#[overdrive_test(guest = "<group>")]`
/// (with or without further arguments such as `profile = "..."`) for exactly
/// the reviewed guest group: a test placed on itself has no table row, and the
/// declaration must name the same group the reviewed row did.
fn declares_the_reviewed_guest_group_on_the_test(row: &Value) -> bool {
    let source = row["source"].as_str().expect("source");
    let path = source.split(':').next().expect("source path");
    let leaf = row["test_name"].as_str().expect("name").rsplit("::").next().expect("leaf");
    let group = row["context"].as_str().expect("guest group");
    let closed = format!("#[overdrive_test(guest = \"{group}\")]");
    let with_arguments = format!("#[overdrive_test(guest = \"{group}\",");
    let Ok(text) = fs::read_to_string(workspace_file(path)) else { return false };
    let Some(function_at) = text.find(&format!("fn {leaf}(")) else { return false };
    let function_line_at = text[..function_at].rfind('\n').map_or(0, |newline| newline + 1);
    text[..function_line_at]
        .lines()
        .rev()
        .take_while(|line| in_attribute_block(line))
        .any(|line| line.trim() == closed || line.trim().starts_with(&with_arguments))
}

/// Whether `line`, read upward from a function, still belongs to the attributes
/// and comments directly above it, however many lines rustfmt gave a multi-line
/// attribute: the walk ends at a blank line or at the previous item's end.
fn in_attribute_block(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("//")
        || !(line.is_empty() || line.ends_with(';') || line.ends_with('{') || line.ends_with('}'))
}

/// Whether a lane or native-launcher test carries the declaration that places
/// it for exactly the reviewed row: `#[overdrive_test(lane_owner = "<fixture>", host = <script>)]`
/// or `lane_operator` for the reviewed fixture, `#[overdrive_test(native_launcher)]`
/// for a native launcher. A test placed on itself has no table row.
fn declares_the_reviewed_lane_or_launcher_on_the_test(row: &Value) -> bool {
    let declaration = match row["placement"].as_str().expect("kind") {
        // An owner lane names its host script after the fixture
        // (`, host = <path>)`), so the declaration is matched up to it.
        "guest-lane-owner" => format!(
            "#[overdrive_test(lane_owner = \"integration::in_guest_lane::{}\", host =",
            row["context"].as_str().expect("fixture")
        ),
        "guest-lane-operator" => format!(
            "#[overdrive_test(lane_operator = \"integration::in_guest_lane::{}\")]",
            row["context"].as_str().expect("fixture")
        ),
        "native-launcher" => "#[overdrive_test(native_launcher)]".to_owned(),
        _ => return false,
    };
    declares_on_the_test(row, &declaration)
}

/// Whether the test function of `row` carries `declaration` among the
/// attributes directly above it, however rustfmt wrapped it.
fn declares_on_the_test(row: &Value, declaration: &str) -> bool {
    let source = row["source"].as_str().expect("source");
    let path = source.split(':').next().expect("source path");
    let leaf = row["test_name"].as_str().expect("name").rsplit("::").next().expect("leaf");
    let Ok(text) = fs::read_to_string(workspace_file(path)) else { return false };
    let Some(function_at) = text.find(&format!("fn {leaf}(")) else { return false };
    let squeezed = |text: &str| text.split_whitespace().collect::<String>();
    let mut attributes: Vec<&str> = text[..function_at]
        .trim_end_matches([' ', '\t'])
        .lines()
        .rev()
        .take_while(|line| in_attribute_block(line))
        .collect();
    attributes.reverse();
    let above = squeezed(&attributes.join("\n"));
    above.contains(&squeezed(declaration))
}

/// Whether a host-pure test is placed by a declaration in its own source: the
/// attribute on the function, a `placement!(host_pure)` or
/// `overdrive_test_default!(host_pure)` in the file that holds it, or the binary
/// default in the root of the test binary it belongs to.
fn declares_host_pure_in_its_source(row: &Value) -> bool {
    let source = row["source"].as_str().expect("source");
    let path = source.split(':').next().expect("source path");
    let leaf = row["test_name"].as_str().expect("name").rsplit("::").next().expect("leaf");
    let Ok(text) = fs::read_to_string(workspace_file(path)) else { return false };
    if text.contains("placement!(host_pure);")
        || text.contains("overdrive_test_default!(host_pure);")
    {
        return true;
    }
    if let Some(function_at) = text.find(&format!("fn {leaf}(")) {
        let on_the_function = text[..function_at]
            .lines()
            .rev()
            .skip(1)
            .take_while(|line| {
                let line = line.trim_start();
                line.starts_with("#[") || line.starts_with("//")
            })
            .any(|line| line.trim() == "#[overdrive_test(host_pure)]");
        if on_the_function {
            return true;
        }
    }
    binary_default_is_host_pure(row)
}

/// Whether the root of the test binary `row` names declares
/// `overdrive_test_default!(host_pure)`: a library's `src/lib.rs`, a binary's
/// `src/main.rs`, `src/bin/<name>.rs` or `bin/<name>.rs`, or an integration
/// target's `tests/<name>.rs`.
fn binary_default_is_host_pure(row: &Value) -> bool {
    let source = row["source"].as_str().expect("source");
    let path = source.split(':').next().expect("source path");
    let mut parts = path.split('/');
    let first = parts.next().expect("source root");
    let package_dir = if first == "crates" {
        format!("crates/{}", parts.next().expect("crate directory"))
    } else {
        first.to_owned()
    };
    let binary_id = row["binary_id"].as_str().expect("binary identity");
    let roots = match binary_id.split_once("::") {
        None => vec![format!("{package_dir}/src/lib.rs")],
        Some((_, target)) => target.strip_prefix("bin/").map_or_else(
            || vec![format!("{package_dir}/tests/{target}.rs")],
            |name| {
                vec![
                    format!("{package_dir}/src/main.rs"),
                    format!("{package_dir}/src/bin/{name}.rs"),
                    format!("{package_dir}/bin/{name}.rs"),
                ]
            },
        ),
    };
    roots.iter().any(|root| {
        fs::read_to_string(workspace_file(root))
            .is_ok_and(|text| text.contains("overdrive_test_default!(host_pure)"))
    })
}

/// Whether the placement of a reviewed or added row is declared in the source of
/// the test it names: the declaration the row's kind needs, naming exactly the
/// reviewed group or fixture. `kind` is the row's placement kind and `context`
/// its group (guest group) or fixture leaf (lane kinds).
fn declares_the_placement(row: &Value, kind: &str, context: &Value) -> bool {
    match kind {
        "guest-group" => declares_the_reviewed_guest_group_on_the_test(&serde_json::json!({
            "source": row["source"], "test_name": row["test_name"], "context": context,
        })),
        "guest-lane-owner" | "guest-lane-operator" | "native-launcher" => {
            declares_the_reviewed_lane_or_launcher_on_the_test(&serde_json::json!({
                "source": row["source"], "test_name": row["test_name"],
                "placement": kind, "context": context,
            }))
        }
        "host-pure" => declares_host_pure_in_its_source(row),
        _ => false,
    }
}

/// CONTRACT_SHAPE: bounded-change (read-only placement comparison and output only;
/// data extensions are exact identities, never absence-means-host or fixed counts).
///
/// Placement has one source, the declaration in the test's own source, so the
/// reviewed partition and the added acceptance identities are compared against
/// those declarations. A row whose test function no longer exists in the
/// workspace has nothing to place and is skipped; every row whose function exists
/// must declare exactly its reviewed placement.
#[test]
fn declared_placement_preserves_the_reviewed_partition_and_declares_new_acceptance_identities() {
    let design: Value = serde_json::from_str(&text(&workspace_file(
        "docs/feature/metal-run-isolation/design/test-placement-inventory.json",
    )))
    .expect("reviewed inventory");
    for row in design["rows"].as_array().expect("reviewed rows") {
        if !test_function_exists(row) {
            continue;
        }
        let kind = row["placement"].as_str().expect("kind");
        assert!(
            declares_the_placement(row, kind, &row["context"]),
            "changed boundary for {}::{}",
            row["binary_id"],
            row["test_name"]
        );
    }
    let additions: Value = serde_json::from_str(&text(&workspace_file(
        "docs/feature/metal-run-isolation/distill/test-placement-additions.json",
    )))
    .expect("DISTILL placement additions");
    for row in additions["rows"].as_array().expect("new rows") {
        if !test_function_exists(row) {
            continue;
        }
        let descriptor = &row["descriptor"];
        let kind = descriptor["kind"].as_str().expect("kind");
        let context = if kind == "guest-group" { &descriptor["group"] } else { &Value::Null };
        assert!(
            declares_the_placement(row, kind, context),
            "undeclared identity {}::{}",
            row["binary_id"],
            row["test_name"]
        );
    }
}

/// CONTRACT_SHAPE: bounded-change (fresh fixture mount/user namespaces and actual
/// syscall refusals only; physical namespace modes and selected body remain unchanged).
#[test]
#[overdrive_test(native_launcher)]
fn sad_each_later_namespace_operation_keeps_its_exact_step_and_errno() {
    for (step, errno) in [("child_mode", "30"), ("local", "28")] {
        let fixture = Fixture::new_accessible();
        let inaccessible = fixture.temp.path().join("namespace-fixture-input");
        let inner = wrapper(
            &fixture,
            NATIVE_BINARY,
            NATIVE_TEST,
            Some(placement_of("native_launcher", NATIVE_TEST, &[])),
        );
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
    let command = wrapper(
        &fixture,
        NATIVE_BINARY,
        NATIVE_TEST,
        Some(placement_of("native_launcher", NATIVE_TEST, &[])),
    );
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
#[overdrive_test(guest = "core")]
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
