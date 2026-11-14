//! The placement record a test declares on itself, the wrapper that resolves it
//! and the eager check that reads it (`placement-lint`), through their driving
//! ports: `test-wrapper.sh --check-placement` and an ordinary wrapper run, and
//! `xtask placement-lint`.
//!
//! The record is a three-reader contract: the attribute writes it, the wrapper
//! reads it at run time and `placement-lint` reads it through the wrapper. Two
//! kinds of binary feed the readers: hand-built ELF files that carry exactly the
//! records a scenario needs (and run nothing), and this test binary itself,
//! whose `fixtures` module carries records the real attribute wrote. The
//! `nextest list` of the lint's driven port is a stub `cargo` on `PATH`; every
//! other tool, `cargo metadata` included, is the real one.
//!
//! Contract of the resolver, first match wins: the test's own record; a
//! `module_host_pure` record for exactly its module; the binary's
//! `default_host_pure`; until retirement a table entry; otherwise
//! `test_unclassified`. A binary with a default and a `guest`, `native_launcher`
//! or lane record is a `placement_conflict`.

#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "Contract Shape declarations and protocol names keep their literal spelling"
)]

use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn workspace_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(relative)
}

// ----- fixtures: records the real attribute writes --------------------------

/// Tests that carry a record the real attribute wrote into this binary. Each is
/// an ordinary passing test; the scenarios below ask the readers about them.
pub(super) mod fixtures {
    use overdrive_testing_macros::{overdrive_test, placement};
    use std::fs;

    /// Where a fixture reports the namespace its body ran in, when asked.
    const REPORT_ENV: &str = "OVERDRIVE_PLACEMENT_FIXTURE_REPORT";

    #[overdrive_test(host_pure)]
    #[test]
    #[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
    fn per_test_host_pure_fixture() {}

    #[overdrive_test(host_pure)]
    #[test]
    #[allow(
        clippy::unnecessary_wraps,
        reason = "the fixture proves a host-pure test may return a Result"
    )]
    #[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
    fn host_pure_fixture_may_return_a_result() -> Result<(), String> {
        Ok(())
    }

    #[overdrive_test(host_pure)]
    #[test]
    #[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
    fn host_pure_fixture_reports_its_namespace() {
        if let Some(path) = std::env::var_os(REPORT_ENV) {
            let link =
                fs::read_link("/proc/self/ns/net").expect("this process's network namespace");
            fs::write(path, link.to_string_lossy().as_bytes()).expect("writing the report");
        }
    }

    #[overdrive_test(native_launcher)]
    #[test]
    #[ignore = "pending DELIVER: lane-and-launcher-placement-on-the-test"]
    fn native_launcher_fixture() {
        if let Some(path) = std::env::var_os(REPORT_ENV) {
            let view: Vec<String> = ["net", "mnt", "cgroup"]
                .iter()
                .map(|kind| {
                    fs::read_link(format!("/proc/self/ns/{kind}"))
                        .map_or_else(|_| "?".to_owned(), |link| link.to_string_lossy().into_owned())
                })
                .chain(std::iter::once(
                    fs::read_to_string("/proc/sys/net/vsock/ns_mode")
                        .unwrap_or_default()
                        .trim()
                        .to_owned(),
                ))
                .collect();
            fs::write(path, view.join("\n")).expect("writing the report");
        }
    }

    pub(super) mod module_declared {
        use super::placement;

        placement!(host_pure);

        #[test]
        #[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
        fn first_test_of_the_declared_module() {}

        #[test]
        #[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
        fn second_test_of_the_declared_module() {}
    }
}

/// The fixtures that take a host-pure form: placed by the host-pure slice.
const HOST_PURE_FIXTURE_TESTS: [&str; 5] = [
    "per_test_host_pure_fixture",
    "host_pure_fixture_may_return_a_result",
    "host_pure_fixture_reports_its_namespace",
    "module_declared::first_test_of_the_declared_module",
    "module_declared::second_test_of_the_declared_module",
];

/// The fixture that takes the native-launcher form: placed by the lane and
/// launcher slice, which delivers that form.
const NATIVE_LAUNCHER_FIXTURE_TEST: &str = "native_launcher_fixture";

fn fixture_test(name: &str) -> String {
    format!("acceptance::placement_attribute::fixtures::{name}")
}

// ----- hand-built binaries --------------------------------------------------

/// One record of the section: `<kind> NUL <test> NUL <argc> NUL <args...>`.
fn record(kind: &str, test: &str, args: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in [kind, test, &args.len().to_string()].into_iter().chain(args.iter().copied()) {
        bytes.extend_from_slice(field.as_bytes());
        bytes.push(0);
    }
    bytes
}

/// A 64-bit little-endian ELF file with no code whose one named section holds
/// `section`: enough for a reader of the section, and it runs nothing.
fn elf_with_placement_section(section: &[u8]) -> Vec<u8> {
    const NAME: &[u8] = b"overdrive_test_placement";
    let mut names = vec![0_u8];
    let section_name = u32::try_from(names.len()).expect("a small offset");
    names.extend_from_slice(NAME);
    names.push(0);
    let names_name = u32::try_from(names.len()).expect("a small offset");
    names.extend_from_slice(b".shstrtab\0");

    let data_offset = 64_u64;
    let names_offset = data_offset + section.len() as u64;
    let table_offset = (names_offset + names.len() as u64).next_multiple_of(8);

    let mut file = vec![0_u8; 64];
    file[..4].copy_from_slice(b"\x7fELF");
    file[4] = 2; // 64-bit
    file[5] = 1; // little endian
    file[6] = 1; // ELF version
    file[16..18].copy_from_slice(&1_u16.to_le_bytes()); // relocatable
    file[18..20].copy_from_slice(&62_u16.to_le_bytes()); // x86-64
    file[20..24].copy_from_slice(&1_u32.to_le_bytes());
    file[40..48].copy_from_slice(&table_offset.to_le_bytes()); // e_shoff
    file[52..54].copy_from_slice(&64_u16.to_le_bytes()); // e_ehsize
    file[58..60].copy_from_slice(&64_u16.to_le_bytes()); // e_shentsize
    file[60..62].copy_from_slice(&3_u16.to_le_bytes()); // e_shnum
    file[62..64].copy_from_slice(&2_u16.to_le_bytes()); // e_shstrndx
    file.extend_from_slice(section);
    file.extend_from_slice(&names);
    file.resize(usize::try_from(table_offset).expect("a small offset"), 0);

    let header = |name: u32, kind: u32, offset: u64, size: u64| {
        let mut entry = Vec::with_capacity(64);
        entry.extend_from_slice(&name.to_le_bytes());
        entry.extend_from_slice(&kind.to_le_bytes());
        entry.extend_from_slice(&0_u64.to_le_bytes()); // flags
        entry.extend_from_slice(&0_u64.to_le_bytes()); // addr
        entry.extend_from_slice(&offset.to_le_bytes());
        entry.extend_from_slice(&size.to_le_bytes());
        entry.extend_from_slice(&0_u32.to_le_bytes()); // link
        entry.extend_from_slice(&0_u32.to_le_bytes()); // info
        entry.extend_from_slice(&1_u64.to_le_bytes()); // align
        entry.extend_from_slice(&0_u64.to_le_bytes()); // entsize
        entry
    };
    file.extend_from_slice(&[0_u8; 64]);
    file.extend(header(section_name, 1, data_offset, section.len() as u64));
    file.extend(header(names_name, 3, names_offset, names.len() as u64));
    file
}

struct Workdir {
    dir: tempfile::TempDir,
}

impl Workdir {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().expect("a scratch directory") }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn binary(&self, name: &str, records: &[Vec<u8>]) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, elf_with_placement_section(&records.concat())).expect("writing a binary");
        path
    }

    fn raw_binary(&self, name: &str, section: &[u8]) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, elf_with_placement_section(section)).expect("writing a binary");
        path
    }
}

/// One listed binary of a `cargo nextest list --message-format json` document.
fn suite(id: &str, path: &Path, tests: &[&str]) -> (String, Value) {
    let cases: serde_json::Map<String, Value> = tests
        .iter()
        .map(|t| {
            ((*t).to_owned(), json!({"ignored": false, "filter-match": {"status": "matches"}}))
        })
        .collect();
    (
        id.to_owned(),
        json!({"binary-id": id, "binary-path": path, "testcases": cases, "status": "listed"}),
    )
}

fn list(suites: Vec<(String, Value)>) -> String {
    let suites: serde_json::Map<String, Value> = suites.into_iter().collect();
    json!({"rust-build-meta": {}, "test-count": 0, "rust-suites": suites}).to_string()
}

// ----- the wrapper's eager check --------------------------------------------

/// Runs `test-wrapper.sh --check-placement` on a list, with no Metal run
/// context at all: the lint runs on CI and developer hosts.
fn check_placement(list_json: &str) -> Output {
    let mut child = Command::new("bash")
        .arg(workspace_file("infra/metal/test-wrapper.sh"))
        .arg("--check-placement")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("running the wrapper");
    // The wrapper may end before it reads the list (a wrapper that does not
    // know the flag runs it as a command), so a broken pipe is not a failure of
    // the test: the exit status and output are what is asserted.
    let _ = child.stdin.take().expect("stdin").write_all(list_json.as_bytes());
    child.wait_with_output().expect("the wrapper's output")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn all_output(output: &Output) -> String {
    format!("{}{}", stdout(output), String::from_utf8_lossy(&output.stderr))
}

/// Every kind a test declares on itself is placed, with no table entry and no
/// Metal run context.
/// CONTRACT_SHAPE: pure-function (a list and records in, a report out).
#[test]
#[ignore = "pending DELIVER: lane-and-launcher-placement-on-the-test"]
fn every_placement_a_test_declares_on_itself_is_placed() {
    let work = Workdir::new();
    let binary = work.binary(
        "all-kinds",
        &[
            record("host_pure", "crate::m::pure", &[]),
            record("native_launcher", "crate::m::launcher", &[]),
            record("guest", "crate::m::guest", &["cgroup-default", "", "0"]),
            record("guest", "crate::m::shared", &["cgroup-default", "quint", "1"]),
            record("lane_operator", "crate::m::operator", &["integration::in_guest_lane::x"]),
            record("lane_owner", "crate::m::owner", &["integration::in_guest_lane::y"]),
        ],
    );
    let tests = ["m::pure", "m::launcher", "m::guest", "m::shared", "m::operator", "m::owner"];
    let output = check_placement(&list(vec![suite("some-crate::integration", &binary, &tests)]));
    assert!(
        output.status.success() && stdout(&output).trim().is_empty(),
        "every declared test is placed:\n{}",
        all_output(&output)
    );
}

/// A listed test with no record, no applicable declaration and no table entry
/// is reported with its binary and its name, so the author learns before Metal.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_listed_test_with_no_placement_is_reported_naming_its_binary_and_itself() {
    let work = Workdir::new();
    let binary =
        work.binary("declared", &[record("host_pure", "crate::new_mesh_case::declared", &[])]);
    let tests = ["new_mesh_case::declared", "new_mesh_case::dials_by_name"];
    let output = check_placement(&list(vec![suite("overdrive-cli::integration", &binary, &tests)]));
    assert!(!output.status.success(), "an unplaced test fails the check");
    let lines: Vec<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(lines, ["unplaced overdrive-cli::integration new_mesh_case::dials_by_name"]);
}

/// A module declaration places the tests of exactly its module: a child module
/// does not inherit it, and a sibling is not covered. CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_module_declaration_places_exactly_the_tests_of_that_module() {
    let work = Workdir::new();
    let binary = work.binary("module", &[record("module_host_pure", "crate::declared", &[])]);
    let tests = [
        "declared::first",
        "declared::second",
        "declared::child::inherits_nothing",
        "sibling::third",
    ];
    let output = check_placement(&list(vec![suite("some-crate::lib", &binary, &tests)]));
    let unplaced: BTreeSet<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(
        unplaced,
        BTreeSet::from([
            "unplaced some-crate::lib declared::child::inherits_nothing".to_owned(),
            "unplaced some-crate::lib sibling::third".to_owned(),
        ])
    );
    assert!(!output.status.success());
}

/// A binary default places every test that has no record, with no conflict when
/// the records beside it are host-pure. CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_binary_default_places_every_test_of_a_host_only_binary() {
    let work = Workdir::new();
    let binary = work.binary(
        "default",
        &[record("default_host_pure", "", &[]), record("host_pure", "crate::explicit", &[])],
    );
    let output = check_placement(&list(vec![suite(
        "some-crate::lib",
        &binary,
        &["explicit", "anything::at_all", "deeply::nested::module::test"],
    )]));
    assert!(output.status.success() && stdout(&output).is_empty(), "{}", all_output(&output));
}

/// A binary that carries a default beside any record that is not host-pure is
/// refused: a forgotten declaration in a mixed binary would silently run on the
/// host. A module declaration is not a default. CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_binary_default_beside_a_non_host_pure_record_is_a_conflict_and_a_module_declaration_is_not() {
    let work = Workdir::new();
    for (kind, args) in [
        ("guest", vec!["cgroup-default", "", "0"]),
        ("native_launcher", vec![]),
        ("lane_operator", vec!["integration::in_guest_lane::x"]),
        ("lane_owner", vec!["integration::in_guest_lane::y"]),
    ] {
        let binary = work.binary(
            &format!("conflict-{kind}"),
            &[record("default_host_pure", "", &[]), record(kind, "crate::m::t", &args)],
        );
        let output = check_placement(&list(vec![suite("mixed::integration", &binary, &["m::t"])]));
        assert!(!output.status.success(), "{kind} beside a default is refused");
        assert!(
            stdout(&output).lines().any(|l| l == "conflict mixed::integration"),
            "{kind}: {}",
            all_output(&output)
        );
    }
    let fine = work.binary(
        "module-beside-guest",
        &[
            record("module_host_pure", "crate::pure", &[]),
            record("guest", "crate::g::t", &["core", "", "0"]),
        ],
    );
    let output =
        check_placement(&list(vec![suite("mixed::integration", &fine, &["pure::a", "g::t"])]));
    assert!(
        output.status.success(),
        "a module declaration beside a guest test is no conflict:\n{}",
        all_output(&output)
    );
}

/// A section the reader cannot parse is refused as invalid, never read as "no
/// findings". CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_section_that_cannot_be_read_is_refused_as_an_invalid_placement() {
    let work = Workdir::new();
    let truncated = work.raw_binary("truncated", b"host_pure\0crate::m::t\0");
    let output = check_placement(&list(vec![suite("some-crate::lib", &truncated, &["m::t"])]));
    assert!(!output.status.success(), "an unreadable section never passes");
    assert!(all_output(&output).contains("placement_invalid"), "{}", all_output(&output));
}

/// The record the attribute writes is read by the wrapper's checker: every
/// host-pure fixture of this binary, one of each form, is placed. This is the
/// drift guard between the writer and the readers for the host-pure forms.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn the_records_the_attribute_wrote_into_this_binary_are_read_by_the_checker() {
    let this_binary = std::env::current_exe().expect("this test binary");
    let names: Vec<String> = HOST_PURE_FIXTURE_TESTS.iter().map(|t| fixture_test(t)).collect();
    let tests: Vec<&str> = names.iter().map(String::as_str).collect();
    let output = check_placement(&list(vec![suite("xtask::acceptance", &this_binary, &tests)]));
    assert!(
        output.status.success() && stdout(&output).is_empty(),
        "every host-pure fixture the attribute declared is placed:\n{}",
        all_output(&output)
    );
}

/// The native-launcher record the attribute writes is read by the same checker:
/// the fixture that declares itself a native launcher is placed. The writer-reader
/// drift guard for the form the lane and launcher slice delivers.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: lane-and-launcher-placement-on-the-test"]
fn the_native_launcher_record_the_attribute_wrote_is_read_by_the_checker() {
    let this_binary = std::env::current_exe().expect("this test binary");
    let name = fixture_test(NATIVE_LAUNCHER_FIXTURE_TEST);
    let output =
        check_placement(&list(vec![suite("xtask::acceptance", &this_binary, &[name.as_str()])]));
    assert!(
        output.status.success() && stdout(&output).is_empty(),
        "the native-launcher fixture the attribute declared is placed:\n{}",
        all_output(&output)
    );
}

// ----- an ordinary wrapper run ----------------------------------------------

struct OrdinaryRun {
    source: tempfile::TempDir,
}

const RUN_ID: &str = "0123456789abcdef01234567";

impl OrdinaryRun {
    /// A source tree the wrapper accepts as a Metal checkout, with `table` as
    /// its placement file.
    fn new(table: &Value) -> Self {
        let source = tempfile::tempdir().expect("a scratch source tree");
        let root = fs::canonicalize(source.path()).expect("canonical source root");
        for marker in [".overdrive-metal-checkout.v1", ".overdrive-metal-source"] {
            fs::write(root.join(marker), b"fixture\n").expect("marker file");
        }
        fs::create_dir_all(root.join("infra/metal")).expect("placement directory");
        fs::write(root.join("infra/metal/test-placement.json"), table.to_string()).expect("table");
        fs::create_dir_all(root.join("target/metal-run-tmp").join(RUN_ID)).expect("run tmp");
        Self { source }
    }

    fn root(&self) -> PathBuf {
        fs::canonicalize(self.source.path()).expect("canonical source root")
    }

    fn wrapper(&self, binary_id: &str, test: &str, exe: &Path, args: &[&str]) -> Command {
        let root = self.root();
        let mut command = Command::new("timeout");
        command
            .args(["--kill-after=1s", "20s", "bash"])
            .arg(workspace_file("infra/metal/test-wrapper.sh"))
            .arg(exe)
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("OVERDRIVE_METAL_RUN_ID", RUN_ID)
            .env("OVERDRIVE_METAL_EXECUTION_CLASS", "ordinary")
            .env("OVERDRIVE_METAL_SOURCE_ROOT", &root)
            .env("NEXTEST_WORKSPACE_ROOT", &root)
            .env("TMPDIR", root.join("target/metal-run-tmp").join(RUN_ID))
            .env("NEXTEST_BINARY_ID", binary_id)
            .env("NEXTEST_TEST_NAME", test);
        command
    }
}

fn empty_table() -> Value {
    json!({"format": "overdrive-metal-test-placement-v1", "placements": {}})
}

fn stderr_has(output: &Output, error: &str) -> bool {
    String::from_utf8_lossy(&output.stderr).contains(&format!("error={error}"))
}

/// An unplaced test is refused before its body runs. CONTRACT_SHAPE: bounded-change
/// (the wrapper's exit status and diagnostic; nothing else is written).
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn an_unplaced_test_is_refused_on_a_metal_run_before_its_body_runs() {
    let work = Workdir::new();
    let binary = work.binary("none", &[record("host_pure", "crate::other", &[])]);
    let run = OrdinaryRun::new(&empty_table());
    let output = run
        .wrapper("some-crate::integration", "m::plain_test", &binary, &[])
        .output()
        .expect("wrapper");
    assert_eq!(output.status.code(), Some(78), "{}", all_output(&output));
    assert!(stderr_has(&output, "test_unclassified"), "{}", all_output(&output));
}

/// A binary that mixes a default with a guest record is refused on a Metal run.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_binary_that_mixes_a_default_with_a_guest_record_is_refused_on_a_metal_run() {
    let work = Workdir::new();
    let binary = work.binary(
        "mixed",
        &[record("default_host_pure", "", &[]), record("guest", "crate::m::g", &["core", "", "0"])],
    );
    let run = OrdinaryRun::new(&empty_table());
    let output =
        run.wrapper("mixed::integration", "m::any", &binary, &[]).output().expect("wrapper");
    assert_eq!(output.status.code(), Some(78), "{}", all_output(&output));
    assert!(stderr_has(&output, "placement_conflict"), "{}", all_output(&output));
}

/// An unreadable section is refused on a Metal run as an invalid placement.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn an_unreadable_section_is_refused_on_a_metal_run_as_an_invalid_placement() {
    let work = Workdir::new();
    let binary = work.raw_binary("garbled", b"host_pure\0crate::m::t\0");
    let run = OrdinaryRun::new(&empty_table());
    let output =
        run.wrapper("some-crate::integration", "m::t", &binary, &[]).output().expect("wrapper");
    assert_eq!(output.status.code(), Some(78), "{}", all_output(&output));
    assert!(stderr_has(&output, "placement_invalid"), "{}", all_output(&output));
}

/// The existing run-context checks still come first: a declaration cannot
/// bypass them (true today, kept true). CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_declared_test_with_an_invalid_run_context_is_refused_by_the_run_context_check() {
    let this_binary = std::env::current_exe().expect("this test binary");
    let run = OrdinaryRun::new(&empty_table());
    let mut command = run.wrapper(
        "xtask::acceptance",
        &fixture_test("per_test_host_pure_fixture"),
        &this_binary,
        &["--exact", &fixture_test("per_test_host_pure_fixture")],
    );
    command.env("OVERDRIVE_METAL_RUN_ID", "not-a-run-id");
    let output = command.output().expect("wrapper");
    assert_eq!(output.status.code(), Some(78), "{}", all_output(&output));
    assert!(stderr_has(&output, "invalid_run_context"), "{}", all_output(&output));
}

/// A test that declares itself host-pure runs under the original runner chain:
/// exact argument, no namespace, and the body runs. CONTRACT_SHAPE:
/// bounded-change (one report file the fixture writes).
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_test_that_declares_itself_host_pure_runs_without_a_namespace() {
    let this_binary = std::env::current_exe().expect("this test binary");
    let name = fixture_test("host_pure_fixture_reports_its_namespace");
    let work = Workdir::new();
    let report = work.path("namespace-report");
    let initial = fs::read_link("/proc/self/ns/net").expect("initial namespace");
    let run = OrdinaryRun::new(&empty_table());
    let output = run
        .wrapper("xtask::acceptance", &name, &this_binary, &["--exact", &name])
        .env("OVERDRIVE_PLACEMENT_FIXTURE_REPORT", &report)
        .output()
        .expect("wrapper");
    assert!(output.status.success(), "{}", all_output(&output));
    assert_eq!(
        fs::read_to_string(&report).expect("the fixture's report"),
        initial.to_string_lossy(),
        "a host-pure test sees the namespace the runner started in"
    );
}

/// A test that declares itself a native launcher keeps the full node view: its
/// own network (vsock local mode), mount and cgroup namespaces.
/// CONTRACT_SHAPE: bounded-change (one report file the fixture writes).
#[test]
#[ignore = "pending DELIVER: lane-and-launcher-placement-on-the-test"]
fn a_test_that_declares_itself_a_native_launcher_runs_in_the_full_node_view() {
    let this_binary = std::env::current_exe().expect("this test binary");
    let name = fixture_test(NATIVE_LAUNCHER_FIXTURE_TEST);
    let work = Workdir::new();
    let report = work.path("view-report");
    let initial: Vec<String> = ["net", "mnt", "cgroup"]
        .iter()
        .map(|kind| {
            fs::read_link(format!("/proc/self/ns/{kind}"))
                .expect("namespace")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let run = OrdinaryRun::new(&empty_table());
    let output = run
        .wrapper("xtask::acceptance", &name, &this_binary, &["--exact", &name])
        .env("OVERDRIVE_PLACEMENT_FIXTURE_REPORT", &report)
        .output()
        .expect("wrapper");
    assert!(output.status.success(), "{}", all_output(&output));
    let seen = fs::read_to_string(&report).expect("the fixture's report");
    let lines: Vec<&str> = seen.lines().collect();
    assert_eq!(lines.len(), 4, "{seen}");
    for (kind, (inside, outside)) in
        ["network", "mount", "cgroup"].iter().zip(lines.iter().zip(&initial))
    {
        assert_ne!(inside, outside, "the {kind} namespace is private to a native launcher");
    }
    assert_eq!(lines[3], "local", "the network namespace is in vsock local mode");
}

/// While the table coexists a record wins over a table entry for the same test:
/// the entry says guest and would need the dispatcher, which is absent, so only
/// the record can place this run. CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn a_record_wins_over_a_table_entry_for_the_same_test() {
    let this_binary = std::env::current_exe().expect("this test binary");
    let name = fixture_test("per_test_host_pure_fixture");
    let table = json!({
        "format": "overdrive-metal-test-placement-v1",
        "placements": {"xtask::acceptance": {(name.clone()): {"kind": "guest-group", "group": "core"}}},
    });
    let run = OrdinaryRun::new(&table);
    let output = run
        .wrapper("xtask::acceptance", &name, &this_binary, &["--exact", &name])
        .output()
        .expect("wrapper");
    assert!(output.status.success(), "{}", all_output(&output));
}

// ----- placement-lint -------------------------------------------------------

struct LintRun {
    work: Workdir,
}

impl LintRun {
    fn new() -> Self {
        Self { work: Workdir::new() }
    }

    /// Runs `xtask placement-lint` from the repository root with a stub `cargo`
    /// in front of the real one: `nextest list` prints `list_json` (or fails with
    /// `fail`), and every other subcommand, `metadata` included, is the real one.
    fn run(&self, list_json: &str, fail: Option<&str>) -> (Output, String) {
        let stub_dir = self.work.path("stub-bin");
        fs::create_dir_all(&stub_dir).expect("stub directory");
        let list_file = self.work.path("list.json");
        fs::write(&list_file, list_json).expect("list file");
        let argv = self.work.path("nextest-list.argv");
        let real = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let script = format!(
            "#!/usr/bin/env bash\nif [ \"${{1:-}}\" = nextest ] && [ \"${{2:-}}\" = list ]; then\n  printf '%s\\n' \"$*\" >> '{argv}'\n  {body}\nfi\nexec '{real}' \"$@\"\n",
            argv = argv.display(),
            real = Path::new(&real).display(),
            body = fail.map_or_else(
                || format!("cat '{}'; exit 0", list_file.display()),
                |message| format!("echo '{message}' >&2; exit 101")
            ),
        );
        let stub = stub_dir.join("cargo");
        fs::write(&stub, script).expect("stub cargo");
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("stub mode");
        let path = std::env::join_paths(
            std::iter::once(stub_dir)
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
        )
        .expect("a PATH");
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("placement-lint")
            .current_dir(workspace_file(""))
            .env("PATH", path)
            .output()
            .expect("running xtask placement-lint");
        let recorded = fs::read_to_string(&argv).unwrap_or_default();
        (output, recorded)
    }
}

/// A fully placed workspace passes. CONTRACT_SHAPE: unbounded-preservation
/// (the lint writes nothing: it reads the list and the binaries).
#[test]
#[ignore = "pending DELIVER: unplaced-test-fails-in-the-default-lane"]
fn placement_lint_passes_when_every_listed_test_is_placed() {
    let lint = LintRun::new();
    let binary = lint.work.binary("a", &[record("host_pure", "crate::m::t", &[])]);
    let (output, _) =
        lint.run(&list(vec![suite("some-crate::integration", &binary, &["m::t"])]), None);
    assert!(output.status.success(), "{}", all_output(&output));
}

/// An unplaced test fails the lint with a line naming the binary and the test,
/// before any guest boots. CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER: unplaced-test-fails-in-the-default-lane"]
fn placement_lint_names_the_binary_and_the_test_of_an_unplaced_integration_test() {
    let lint = LintRun::new();
    let binary = lint.work.binary("a", &[record("host_pure", "crate::m::placed", &[])]);
    let tests = ["m::placed", "new_mesh_case::dials_by_name"];
    let (output, _) =
        lint.run(&list(vec![suite("overdrive-cli::integration", &binary, &tests)]), None);
    assert_eq!(output.status.code(), Some(1), "{}", all_output(&output));
    assert!(
        all_output(&output).lines().any(|l| l
            == "placement-lint: unplaced overdrive-cli::integration new_mesh_case::dials_by_name"),
        "{}",
        all_output(&output)
    );
}

/// A binary that mixes a default with a non-host-pure record fails the lint.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER: unplaced-test-fails-in-the-default-lane"]
fn placement_lint_reports_a_binary_that_mixes_a_default_with_a_guest_test() {
    let lint = LintRun::new();
    let binary = lint.work.binary(
        "a",
        &[record("default_host_pure", "", &[]), record("guest", "crate::m::g", &["core", "", "0"])],
    );
    let (output, _) =
        lint.run(&list(vec![suite("mixed::integration", &binary, &["m::g", "m::h"])]), None);
    assert_eq!(output.status.code(), Some(1), "{}", all_output(&output));
    assert!(
        all_output(&output).lines().any(|l| l == "placement-lint: conflict mixed::integration"),
        "{}",
        all_output(&output)
    );
}

/// A lane test that names a fixture its provider binary does not list fails
/// before any run, naming binary, test and fixture; a fixture that exists does
/// not. CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER: lane-and-launcher-placement-on-the-test"]
fn placement_lint_reports_a_lane_test_whose_fixture_does_not_exist() {
    let lint = LintRun::new();
    let lanes = lint.work.binary(
        "init",
        &[
            record(
                "lane_owner",
                "crate::guest_flow::real_kernel::kept",
                &["integration::in_guest_lane::exists"],
            ),
            record(
                "lane_owner",
                "crate::guest_flow::real_kernel::typo",
                &["integration::in_guest_lane::mispelled"],
            ),
            record("host_pure", "crate::integration::in_guest_lane::exists", &[]),
        ],
    );
    let tests = [
        "guest_flow::real_kernel::kept",
        "guest_flow::real_kernel::typo",
        "integration::in_guest_lane::exists",
    ];
    let (output, _) =
        lint.run(&list(vec![suite("overdrive-init::bin/overdrive-init", &lanes, &tests)]), None);
    assert_eq!(output.status.code(), Some(1), "{}", all_output(&output));
    let text = all_output(&output);
    assert!(
        text.lines().any(|l| l
            == "placement-lint: missing-fixture overdrive-init::bin/overdrive-init guest_flow::real_kernel::typo integration::in_guest_lane::mispelled"),
        "{text}"
    );
    assert!(
        !text.contains("guest_flow::real_kernel::kept"),
        "an existing fixture is not reported:\n{text}"
    );
}

/// A listing that fails is reported with its cause and a non-zero status, never
/// as "no findings". CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER: unplaced-test-fails-in-the-default-lane"]
fn placement_lint_fails_with_the_cause_when_the_test_listing_fails() {
    let lint = LintRun::new();
    let (output, _) =
        lint.run("", Some("error: could not compile `some-crate` (test \"integration\")"));
    assert!(!output.status.success(), "a failed listing is never a pass");
    assert!(
        all_output(&output).contains("could not compile `some-crate`"),
        "{}",
        all_output(&output)
    );
}

/// The listing enables `integration-tests` and, for every member whose manifest
/// declares `kvm-tests`, that member's own feature, so no test behind it escapes
/// the check. The lint's own `cargo metadata` read (the real one, through the
/// stub `cargo`'s pass-through) is the subject under test here: it is the lint's
/// discovery port for the members. It is a read-only query that builds nothing
/// and takes no build lock; the nested `cargo` that design gap D-9 forbids in a
/// test is the build-bearing `cargo nextest list`, which this test stubs.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER: unplaced-test-fails-in-the-default-lane"]
fn placement_lint_lists_with_integration_tests_and_each_members_own_kvm_feature() {
    let lint = LintRun::new();
    let binary = lint.work.binary("a", &[record("host_pure", "crate::m::t", &[])]);
    let (output, argv) =
        lint.run(&list(vec![suite("some-crate::integration", &binary, &["m::t"])]), None);
    assert!(output.status.success(), "{}", all_output(&output));

    let mut declaring = BTreeSet::new();
    for entry in fs::read_dir(workspace_file("crates")).expect("crates directory").flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let manifest = entry.path().join("Cargo.toml");
        let text = fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", manifest.display()));
        let name = text
            .lines()
            .find_map(|l| l.strip_prefix("name").and_then(|r| r.split('"').nth(1)))
            .map(str::to_owned);
        if text.lines().any(|l| l.trim_start().starts_with("kvm-tests") && l.contains('=')) {
            declaring.extend(name);
        }
    }
    assert!(
        declaring.len() >= 5,
        "the workspace has members that declare kvm-tests: {declaring:?}"
    );
    assert!(argv.contains("integration-tests"), "{argv}");
    for member in &declaring {
        assert!(
            argv.contains(&format!("{member}/kvm-tests")),
            "{member}/kvm-tests is enabled:\n{argv}"
        );
    }
}

// ----- retirement -----------------------------------------------------------

/// Reads a maintained file. A file a scenario names must exist: a missing or
/// unreadable one is a failure, never a skipped check.
fn read(relative: &str) -> String {
    fs::read_to_string(workspace_file(relative))
        .unwrap_or_else(|error| panic!("{relative} must exist and be readable: {error}"))
}

/// With the table retired nothing reads or ships it: the file, its lookup in the
/// wrapper, the dispatcher binary and its environment variables are gone.
/// CONTRACT_SHAPE: unbounded-preservation (reads the tree, writes nothing).
#[test]
#[ignore = "pending DELIVER: placement-table-and-dispatcher-retired"]
fn the_placement_table_the_lookup_and_the_dispatcher_are_retired() {
    assert!(
        !workspace_file("infra/metal/test-placement.json").exists(),
        "the table file is deleted"
    );
    assert!(
        !workspace_file("crates/overdrive-testing/bin/fwd_guest_suite.rs").exists(),
        "the dispatcher binary is deleted"
    );
    let wrapper = read("infra/metal/test-wrapper.sh");
    for removed in [
        "test-placement",
        "--guest-suite",
        "OVERDRIVE_METAL_TEST_DISPATCHER",
        "overdrive-fwd-guest-suite",
    ] {
        assert!(!wrapper.contains(removed), "the wrapper still names `{removed}`");
    }
}

/// The maintained documents describe the one model: the attribute and the lint,
/// and no longer the table or the explicit batched command.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER: placement-table-and-dispatcher-retired"]
fn the_maintained_documents_describe_the_attribute_and_name_no_removed_path() {
    let rules = read(".claude/rules/testing.md");
    assert!(
        rules.contains("overdrive_test") && rules.contains("placement-lint"),
        "testing.md documents the attribute and the lint"
    );
    for path in [
        ".claude/rules/testing.md",
        "infra/metal/README.md",
        "website/content/docs/how-to/run-verification-on-metal.mdx",
        "tests/conformance/CLAUDE.md",
    ] {
        let text = read(path);
        for removed in [
            "test-placement.json",
            "overdrive-fwd-guest-suite",
            "OVERDRIVE_FWD_GUEST_GROUP",
            "OVERDRIVE_FWD_GUEST_EXACT",
        ] {
            assert!(!text.contains(removed), "{path} still names `{removed}`");
        }
    }
}
