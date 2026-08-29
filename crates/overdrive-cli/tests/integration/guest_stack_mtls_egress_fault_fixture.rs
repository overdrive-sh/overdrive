//! Delta-scoped native nft fault fixture for the S-GTI-05 prerequisite.
//!
//! The later owner step still carries the scenario RED scaffold. These tests
//! close only the fixture contract: preserve the existing table object and
//! every unrelated rule while temporarily substituting the `prerouting`
//! chain, drive the real typed production failure, and restore exact
//! structural state on normal, panic, watchdog-signal, parent-death, and
//! partial-construction exits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_netlink::{NetlinkError, nft};
use overdrive_worker::mtls_intercept::{InterceptError, install_outbound_tproxy};
use overdrive_worker::mtls_intercept_worker::MtlsInterceptInstallError;
use serde_json::{Value, json};
use serial_test::serial;
use tempfile::TempDir;

const TABLE: &str = "overdrive-mtls";
const PREROUTING: &str = "prerouting";
const OUTPUT: &str = "output";
const CONTAMINATED_TABLE_HANDLE: u64 = 74;
const CONTAMINATED_PREROUTING_HANDLES: &[u64] = &[25, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
const CONTAMINATED_OUTPUT_HANDLES: &[u64] = &[26, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24];

#[derive(Clone, Debug, PartialEq, Eq)]
struct PacketPathBaseline {
    /// Normalized nft JSON retains table/chain/rule handles, complete ordered
    /// expression programs, and counters. Only nft's tool-version metainfo is
    /// omitted because it is not kernel ruleset state.
    nft_table: Option<Value>,
    /// Raw GETRULE ownership identity for every chain, ordered exactly as the
    /// kernel returned it. This closes the JSON surface's userdata omission.
    ownership: BTreeMap<String, Vec<nft::RuleInfo>>,
    fib_rules: Value,
    fib_routes: Value,
}

impl PacketPathBaseline {
    fn capture() -> Self {
        let nft_output = Command::new("nft")
            .args(["-a", "-j", "list", "table", "ip", TABLE])
            .output()
            .expect("capture normalized nft table");
        let nft_table = if nft_output.status.success() {
            let mut value: Value =
                serde_json::from_slice(&nft_output.stdout).expect("nft JSON is valid");
            value
                .get_mut("nftables")
                .and_then(Value::as_array_mut)
                .expect("nft JSON has nftables array")
                .retain(|entry| entry.get("metainfo").is_none());
            Some(value)
        } else {
            None
        };

        let mut ownership = BTreeMap::new();
        if let Some(table) = &nft_table {
            for chain in chain_names(table) {
                ownership.insert(
                    chain.clone(),
                    nft::list_rules(TABLE, &chain)
                        .unwrap_or_else(|error| panic!("GETRULE {TABLE}/{chain}: {error}")),
                );
            }
        }

        Self {
            nft_table,
            ownership,
            fib_rules: json_output("ip", &["-j", "rule", "show"]),
            fib_routes: json_output("ip", &["-j", "route", "show", "table", "100"]),
        }
    }
}

fn json_output(program: &str, args: &[&str]) -> Value {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {program} {args:?}: {error}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("decode {program} {args:?} JSON: {error}"))
}

fn chain_names(table: &Value) -> Vec<String> {
    table["nftables"]
        .as_array()
        .expect("nftables array")
        .iter()
        .filter_map(|entry| entry.get("chain"))
        .filter_map(|chain| chain.get("name"))
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn rule_programs(table: &Value) -> BTreeMap<(String, u64), Value> {
    table["nftables"]
        .as_array()
        .expect("nftables array")
        .iter()
        .filter_map(|entry| entry.get("rule"))
        .map(|rule| {
            let chain = rule["chain"].as_str().expect("rule chain").to_owned();
            let handle = rule["handle"].as_u64().expect("rule handle");
            ((chain, handle), rule["expr"].clone())
        })
        .collect()
}

fn table_handle(table: &Value) -> u64 {
    table["nftables"]
        .as_array()
        .expect("nftables array")
        .iter()
        .find_map(|entry| entry.get("table"))
        .and_then(|table| table.get("handle"))
        .and_then(Value::as_u64)
        .expect("table handle")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConstructionFault {
    None,
    AfterOriginalRename,
}

struct DeltaScopedMalformedPrerouting {
    baseline: PacketPathBaseline,
    watchdog: Option<Child>,
    stop_file: PathBuf,
    _watchdog_dir: TempDir,
}

impl DeltaScopedMalformedPrerouting {
    fn install() -> Self {
        Self::try_install(std::process::id(), ConstructionFault::None)
            .unwrap_or_else(|error| panic!("install delta-scoped INPUT-hook fixture: {error}"))
    }

    fn try_install(parent: u32, fault: ConstructionFault) -> Result<Self, String> {
        let baseline = PacketPathBaseline::capture();
        let watchdog_dir = TempDir::new().map_err(|error| error.to_string())?;
        let dir = watchdog_dir.path();
        let ready_file = dir.join("ready");
        let stop_file = dir.join("stop");
        let script = dir.join("watchdog.sh");
        let saved_chain = format!("prerouting-gti-saved-{parent}");

        mark_if(dir, "table-present", baseline.nft_table.is_some())?;
        let chains = baseline.nft_table.as_ref().map_or_else(Vec::new, chain_names);
        mark_if(dir, "prerouting-present", chains.iter().any(|name| name == PREROUTING))?;
        mark_if(dir, "output-present", chains.iter().any(|name| name == OUTPUT))?;
        mark_if(
            dir,
            "had-fwmark-rule",
            command_stdout("ip", &["rule", "show"])
                .lines()
                .any(|line| line.contains("fwmark 0x1") && line.contains("lookup 100")),
        )?;
        mark_if(
            dir,
            "had-local-route",
            command_stdout("ip", &["route", "show", "table", "100"])
                .lines()
                .any(|line| line.starts_with("local ") && line.contains(" dev lo")),
        )?;
        mark_if(dir, "inject-after-rename", fault == ConstructionFault::AfterOriginalRename)?;

        std::fs::write(&script, watchdog_script()).map_err(|error| error.to_string())?;
        let watchdog = Command::new("sh")
            .arg(&script)
            .arg(parent.to_string())
            .arg(dir)
            .arg(&saved_chain)
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut fixture =
            Self { baseline, watchdog: Some(watchdog), stop_file, _watchdog_dir: watchdog_dir };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready_file.exists() {
            if let Some(status) = fixture
                .watchdog
                .as_mut()
                .expect("watchdog present")
                .try_wait()
                .map_err(|error| error.to_string())?
            {
                fixture.watchdog = None;
                return Err(format!("watchdog exited during construction: {status}"));
            }
            if Instant::now() >= deadline {
                return Err("watchdog did not become ready".to_owned());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(fixture)
    }

    fn finish(mut self) {
        let status = self.stop_and_wait().expect("wait for fixture watchdog");
        assert!(status.success(), "fixture watchdog restoration failed: {status}");
        assert_eq!(
            PacketPathBaseline::capture(),
            self.baseline,
            "normal restoration preserves full programs/counters/handles/userdata and FIB state"
        );
    }

    fn signal_and_wait(&mut self, signal: i32) -> ExitStatus {
        let pid = self.watchdog.as_ref().expect("watchdog present").id();
        // SAFETY: pid is the live child owned by this fixture and signal is a
        // caller-selected POSIX signal constant.
        let rc = unsafe { libc::kill(pid.cast_signed(), signal) };
        assert_eq!(rc, 0, "signal fixture watchdog");
        self.wait().expect("wait for signalled fixture watchdog")
    }

    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.watchdog.take().expect("watchdog present").wait()
    }

    fn stop_and_wait(&mut self) -> std::io::Result<ExitStatus> {
        std::fs::write(&self.stop_file, b"")?;
        self.wait()
    }
}

impl Drop for DeltaScopedMalformedPrerouting {
    fn drop(&mut self) {
        if self.watchdog.is_some() {
            let _ = self.stop_and_wait();
        }
    }
}

fn mark_if(dir: &Path, name: &str, condition: bool) -> Result<(), String> {
    if condition {
        std::fs::write(dir.join(name), b"").map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn command_stdout(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run {program} {args:?}: {error}"));
    assert!(output.status.success(), "{program} {args:?} failed");
    String::from_utf8(output.stdout).expect("command output is UTF-8")
}

fn watchdog_script() -> &'static str {
    r#"#!/bin/sh
set -u
parent="$1"
dir="$2"
saved="$3"

restore() {
  status=0
  if [ -f "$dir/table-created" ]; then
    nft delete table ip overdrive-mtls >/dev/null 2>&1 || status=81
  else
    if [ -f "$dir/malformed-created" ]; then
      nft flush chain ip overdrive-mtls prerouting >/dev/null 2>&1 || status=82
      nft delete chain ip overdrive-mtls prerouting >/dev/null 2>&1 || status=83
    fi
    if [ -f "$dir/original-renamed" ]; then
      nft rename chain ip overdrive-mtls "$saved" prerouting >/dev/null 2>&1 || status=84
    fi
    if [ ! -f "$dir/output-present" ] \
      && nft list chain ip overdrive-mtls output >/dev/null 2>&1; then
      nft flush chain ip overdrive-mtls output >/dev/null 2>&1 || status=85
      nft delete chain ip overdrive-mtls output >/dev/null 2>&1 || status=86
    fi
  fi
  if [ ! -f "$dir/had-fwmark-rule" ]; then
    while ip rule del fwmark 0x1 lookup 100 >/dev/null 2>&1; do :; done
  fi
  if [ ! -f "$dir/had-local-route" ]; then
    ip route del local 0.0.0.0/0 dev lo table 100 >/dev/null 2>&1 || true
  fi
  return "$status"
}

finish() {
  prior=$?
  restore
  restored=$?
  trap - EXIT HUP INT TERM
  if [ "$restored" -ne 0 ]; then
    exit "$restored"
  fi
  exit "$prior"
}

trap finish EXIT
trap 'exit 128' HUP INT TERM

if [ ! -f "$dir/table-present" ]; then
  nft add table ip overdrive-mtls || exit 71
  : >"$dir/table-created"
elif [ -f "$dir/prerouting-present" ]; then
  if nft list chain ip overdrive-mtls "$saved" >/dev/null 2>&1; then
    exit 72
  fi
  nft rename chain ip overdrive-mtls prerouting "$saved" || exit 73
  : >"$dir/original-renamed"
fi

if [ -f "$dir/inject-after-rename" ]; then
  exit 70
fi

nft add chain ip overdrive-mtls prerouting \
  '{ type filter hook input priority mangle; policy accept; }' || exit 74
: >"$dir/malformed-created"
: >"$dir/ready"

while kill -0 "$parent" 2>/dev/null && [ ! -f "$dir/stop" ]; do
  sleep 0.05
done
"#
}

fn assert_typed_input_hook_failure() {
    let fixture = DeltaScopedMalformedPrerouting::install();
    let Err(source) = install_outbound_tproxy("ovd-hv-fffe", 49_151) else {
        panic!("the production IPv4 TPROXY expression must fail on the INPUT-hook substitute");
    };
    let install = MtlsInterceptInstallError::OutboundTproxyInstall(source);
    let source = match install {
        MtlsInterceptInstallError::OutboundTproxyInstall(
            InterceptError::NftRuleInstallFailed { op: "append-egress", source },
        ) => source,
        other => panic!("expected typed append-egress failure, got {other:#?}"),
    };
    let source = match source {
        NetlinkError::Nft { op: "append-rule", source } => source,
        other => panic!("expected typed append-rule failure, got {other:#?}"),
    };
    assert_eq!(source.raw_os_error(), Some(libc::EOPNOTSUPP));
    fixture.finish();
}

/// The real encoded INPUT-hook failure retains its complete typed cause chain,
/// while the delta-scoped substitute restores every unrelated structural fact.
#[test]
#[serial(cgroup)]
fn input_hook_fault_is_typed_and_normal_restoration_is_structurally_exact() {
    assert_typed_input_hook_failure();
}

/// Every trapped fixture exit restores the exact normalized packet-path
/// baseline; no whole-table delete/replay is used when the table pre-exists.
#[test]
#[serial(cgroup)]
fn trapped_fixture_paths_restore_programs_counters_handles_userdata_and_fib() {
    let panic_baseline = PacketPathBaseline::capture();
    let panic_result = std::panic::catch_unwind(|| {
        let _fixture = DeltaScopedMalformedPrerouting::install();
        panic!("intentional fixture-body panic");
    });
    assert!(panic_result.is_err());
    assert_eq!(PacketPathBaseline::capture(), panic_baseline, "panic/drop restoration");

    let signal_baseline = PacketPathBaseline::capture();
    let mut signalled = DeltaScopedMalformedPrerouting::install();
    let status = signalled.signal_and_wait(libc::SIGTERM);
    assert_eq!(status.code(), Some(128), "watchdog signal path exits through its trap");
    assert_eq!(PacketPathBaseline::capture(), signal_baseline, "signal restoration");

    let parent_baseline = PacketPathBaseline::capture();
    let mut parent =
        Command::new("sleep").arg("60").spawn().expect("spawn disposable watchdog parent");
    let mut orphaned =
        DeltaScopedMalformedPrerouting::try_install(parent.id(), ConstructionFault::None)
            .expect("install parent-death fixture");
    parent.kill().expect("terminate disposable parent");
    parent.wait().expect("reap disposable parent");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = orphaned
            .watchdog
            .as_mut()
            .expect("watchdog present")
            .try_wait()
            .expect("poll parent-death watchdog")
        {
            orphaned.watchdog = None;
            assert!(status.success(), "parent-death restoration succeeds: {status}");
            break;
        }
        assert!(Instant::now() < deadline, "parent-death watchdog must exit");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(PacketPathBaseline::capture(), parent_baseline, "parent-death restoration");

    let partial_baseline = PacketPathBaseline::capture();
    let partial = DeltaScopedMalformedPrerouting::try_install(
        std::process::id(),
        ConstructionFault::AfterOriginalRename,
    );
    assert!(partial.is_err(), "the injected construction interruption is observable");
    assert_eq!(PacketPathBaseline::capture(), partial_baseline, "partial construction restoration");
}

fn canonical_exemption_program() -> Value {
    json!([
        {"match": {"op": "==", "left": {"meta": {"key": "mark"}}, "right": 2}},
        {"accept": null}
    ])
}

fn assert_canonical_clean_state(snapshot: &PacketPathBaseline) {
    let table = snapshot.nft_table.as_ref().expect("production nft table exists");
    let programs = rule_programs(table);
    let tag = nft::userdata_exemption();
    for chain in [PREROUTING, OUTPUT] {
        let ownership = snapshot.ownership.get(chain).expect("production chain ownership");
        let exemptions = ownership.iter().filter(|rule| rule.userdata == tag).collect::<Vec<_>>();
        assert_eq!(exemptions.len(), 1, "{chain} has exactly one production-owned exemption");
        let owned = exemptions[0];
        assert_eq!(
            programs.get(&(chain.to_owned(), owned.handle)),
            Some(&canonical_exemption_program()),
            "{chain} owned exemption has the exact production expression program"
        );
        assert_eq!(ownership.len(), 1, "{chain} has no duplicate or foreign residue");
    }
}

/// One-time, audit-pinned repair for contamination authored by the removed
/// whole-table replay fixture. Deletion is fail-closed on the reviewed table
/// handle, exact chain+rule handles, exact expression program, and exemption
/// userdata shape; any unrelated or changed host state aborts before mutation.
#[test]
#[serial(cgroup)]
fn prior_fixture_duplicate_exemptions_are_safely_repaired_once() {
    let before = PacketPathBaseline::capture();
    if before
        .ownership
        .values()
        .all(|rules| rules.len() == 1 && rules[0].userdata == nft::userdata_exemption())
    {
        assert_canonical_clean_state(&before);
        return;
    }

    let table = before.nft_table.as_ref().expect("reviewed contaminated table exists");
    assert_eq!(
        table_handle(table),
        CONTAMINATED_TABLE_HANDLE,
        "refuse to repair a table other than the exact reviewer-audited object"
    );
    let programs = rule_programs(table);
    let expected_program = canonical_exemption_program();
    let expected_tag = nft::userdata_exemption();
    for (chain, handles) in
        [(PREROUTING, CONTAMINATED_PREROUTING_HANDLES), (OUTPUT, CONTAMINATED_OUTPUT_HANDLES)]
    {
        let ownership = before.ownership.get(chain).expect("reviewed chain exists");
        assert_eq!(
            ownership.iter().map(|rule| rule.handle).collect::<Vec<_>>(),
            handles,
            "refuse mutation unless the complete ordered handle set is the reviewer-audited contamination"
        );
        for rule in ownership {
            assert!(
                rule.userdata.is_empty() || rule.userdata == expected_tag,
                "refuse to delete unrelated userdata at {chain}/{}",
                rule.handle
            );
            assert_eq!(
                programs.get(&(chain.to_owned(), rule.handle)),
                Some(&expected_program),
                "refuse to delete a rule whose normalized program is not the audited exemption"
            );
        }
    }

    let fib_before = (before.fib_rules.clone(), before.fib_routes.clone());
    for (chain, handles) in
        [(PREROUTING, CONTAMINATED_PREROUTING_HANDLES), (OUTPUT, CONTAMINATED_OUTPUT_HANDLES)]
    {
        for handle in handles {
            nft::delete_rule(TABLE, chain, *handle)
                .unwrap_or_else(|error| panic!("delete audited {chain}/{handle}: {error}"));
        }
        nft::insert_rule(
            TABLE,
            chain,
            &nft::mark_accept_exemption_exprs(MTLS_LEG_S_DIAL_MARK),
            &expected_tag,
        )
        .unwrap_or_else(|error| panic!("install canonical owned exemption in {chain}: {error}"));
    }

    let after = PacketPathBaseline::capture();
    assert_canonical_clean_state(&after);
    assert_eq!((after.fib_rules, after.fib_routes), fib_before, "repair changes no FIB state");
}
