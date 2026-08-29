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
        Self::capture_table(TABLE)
    }

    fn capture_table(table_name: &str) -> Self {
        let nft_output = Command::new("nft")
            .args(["-a", "-j", "list", "table", "ip", table_name])
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
                    nft::list_rules(table_name, &chain)
                        .unwrap_or_else(|error| panic!("GETRULE {table_name}/{chain}: {error}")),
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

fn chain_handle(table: &Value, chain_name: &str) -> Option<u64> {
    table["nftables"]
        .as_array()
        .expect("nftables array")
        .iter()
        .filter_map(|entry| entry.get("chain"))
        .find(|chain| chain.get("name").and_then(Value::as_str) == Some(chain_name))
        .and_then(|chain| chain.get("handle"))
        .and_then(Value::as_u64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FaultPoint(&'static str);

impl FaultPoint {
    const NONE: Self = Self("");
}

struct DeltaScopedMalformedPrerouting {
    baseline: PacketPathBaseline,
    table_name: String,
    watchdog: Option<Child>,
    stop_file: PathBuf,
    _watchdog_dir: TempDir,
}

impl DeltaScopedMalformedPrerouting {
    fn install() -> Self {
        Self::try_install(std::process::id(), FaultPoint::NONE)
            .unwrap_or_else(|error| panic!("install delta-scoped INPUT-hook fixture: {error}"))
    }

    fn try_install(parent: u32, fault: FaultPoint) -> Result<Self, String> {
        Self::try_install_in(TABLE, parent, fault)
    }

    fn try_install_in(table_name: &str, parent: u32, fault: FaultPoint) -> Result<Self, String> {
        let baseline = PacketPathBaseline::capture_table(table_name);
        let watchdog_dir = TempDir::new().map_err(|error| error.to_string())?;
        let dir = watchdog_dir.path();
        let ready_file = dir.join("ready");
        let stop_file = dir.join("stop");
        let script = dir.join("watchdog.sh");
        let saved_chain = format!("prerouting-gti-saved-{parent}");
        let owner = format!("overdrive-gti-{parent}-{}", std::process::id());

        mark_if(dir, "table-present", baseline.nft_table.is_some())?;
        let chains = baseline.nft_table.as_ref().map_or_else(Vec::new, chain_names);
        mark_if(dir, "prerouting-present", chains.iter().any(|name| name == PREROUTING))?;
        mark_if(dir, "output-present", chains.iter().any(|name| name == OUTPUT))?;
        mark_if(dir, "saved-present", chains.iter().any(|name| name == &saved_chain))?;
        if let Some(handle) =
            baseline.nft_table.as_ref().and_then(|table| chain_handle(table, PREROUTING))
        {
            std::fs::write(dir.join("baseline-prerouting-handle"), handle.to_string())
                .map_err(|error| error.to_string())?;
        }
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
        std::fs::write(&script, watchdog_script()).map_err(|error| error.to_string())?;
        let watchdog = Command::new("sh")
            .arg(&script)
            .arg(parent.to_string())
            .arg(dir)
            .arg(&saved_chain)
            .arg(table_name)
            .arg(fault.0)
            .arg(&owner)
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut fixture = Self {
            baseline,
            table_name: table_name.to_owned(),
            watchdog: Some(watchdog),
            stop_file,
            _watchdog_dir: watchdog_dir,
        };
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
            PacketPathBaseline::capture_table(&self.table_name),
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
table="$4"
fault="$5"
owner="$6"

table_exists() {
  nft list table ip "$table" >/dev/null 2>&1
}

chain_exists() {
  nft list chain ip "$table" "$1" >/dev/null 2>&1
}

table_owned() {
  nft -j list table ip "$table" 2>/dev/null \
    | grep -Eq "\"comment\"[[:space:]]*:[[:space:]]*\"$owner\""
}

chain_owned() {
  nft -j list chain ip "$table" "$1" 2>/dev/null \
    | grep -Eq "\"comment\"[[:space:]]*:[[:space:]]*\"$owner\""
}

chain_kernel_handle() {
  nft -a list chain ip "$table" "$1" 2>/dev/null \
    | sed -n 's/.*# handle \([0-9][0-9]*\).*/\1/p' \
    | head -1
}

owned_table_is_disposable() {
  json="$(nft -j list table ip "$table" 2>/dev/null)" || return 1
  chain_count="$(printf '%s' "$json" | grep -o '"chain":' | wc -l)"
  rule_count="$(printf '%s' "$json" | grep -o '"rule":' | wc -l)"
  owner_count="$(printf '%s' "$json" \
    | grep -Eo "\"comment\"[[:space:]]*:[[:space:]]*\"$owner\"" | wc -l)"
  [ "$rule_count" -eq 0 ] && [ "$owner_count" -eq $((chain_count + 1)) ]
}

sync_journal() {
  sync -f "$dir" >/dev/null 2>&1 || sync >/dev/null 2>&1
}

inject() {
  point="$1"
  mode=return
  requested="$fault"
  case "$requested" in
    signal:*) mode=signal; requested="${requested#signal:}" ;;
    parent-death:*) mode=parent-death; requested="${requested#parent-death:}" ;;
  esac
  if [ -n "$requested" ] && [ "$requested" = "$point" ] \
    && [ ! -f "$dir/fault-consumed" ]; then
    : >"$dir/fault-consumed" || return 1
    sync_journal || return 1
    if [ "$mode" = signal ]; then
      kill -TERM "$$"
    elif [ "$mode" = parent-death ]; then
      kill -TERM "$parent" 2>/dev/null || true
    fi
    return 0
  fi
  return 1
}

journal_intent() {
  action="$1"
  inject "${action}-before-intent" && return 70
  : >"$dir/intent-${action}" || return 71
  sync_journal || return 72
  inject "${action}-after-intent" && return 73
  return 0
}

journal_applied() {
  action="$1"
  inject "${action}-before-applied" && return 74
  : >"$dir/applied-${action}" || return 75
  sync_journal || return 76
  inject "${action}-after-applied" && return 77
  return 0
}

setup() {
  if [ ! -f "$dir/table-present" ]; then
    journal_intent setup-table-add || return $?
    nft add table ip "$table" "{ comment \"$owner\"; }" || return 60
    inject setup-table-add-after-mutation && return 61
    journal_applied setup-table-add || return $?
  elif [ -f "$dir/prerouting-present" ]; then
    if chain_exists "$saved"; then
      return 62
    fi
    journal_intent setup-original-rename || return $?
    nft rename chain ip "$table" prerouting "$saved" || return 63
    inject setup-original-rename-after-mutation && return 64
    journal_applied setup-original-rename || return $?
  fi

  journal_intent setup-malformed-create || return $?
  nft add chain ip "$table" prerouting \
    "{ type filter hook input priority mangle; policy accept; comment \"$owner\"; }" || return 65
  inject setup-malformed-create-after-mutation && return 66
  journal_applied setup-malformed-create || return $?
  inject setup-ready-before-marker && return 78
  : >"$dir/ready" || return 79
  sync_journal || return 80
  inject setup-ready-after-marker && return 81
  return 0
}

restore_once() {
  if [ ! -f "$dir/table-present" ]; then
    if table_exists; then
      table_owned && owned_table_is_disposable || return 89
      journal_intent cleanup-table-delete || return $?
      nft delete table ip "$table" >/dev/null 2>&1 || return 90
      inject cleanup-table-delete-after-mutation && return 91
      journal_applied cleanup-table-delete || return $?
    fi
  else
    table_exists || return 92
    if [ ! -f "$dir/saved-present" ] && chain_exists "$saved"; then
      expected_handle="$(cat "$dir/baseline-prerouting-handle" 2>/dev/null)" || return 92
      [ "$(chain_kernel_handle "$saved")" = "$expected_handle" ] || return 92
      if chain_exists prerouting; then
        chain_owned prerouting || return 92
        journal_intent cleanup-malformed-flush || return $?
        nft flush chain ip "$table" prerouting >/dev/null 2>&1 || return 93
        inject cleanup-malformed-flush-after-mutation && return 94
        journal_applied cleanup-malformed-flush || return $?
      fi
      if chain_exists prerouting; then
        journal_intent cleanup-malformed-delete || return $?
        nft delete chain ip "$table" prerouting >/dev/null 2>&1 || return 95
        inject cleanup-malformed-delete-after-mutation && return 96
        journal_applied cleanup-malformed-delete || return $?
      fi
      journal_intent cleanup-original-rename || return $?
      nft rename chain ip "$table" "$saved" prerouting >/dev/null 2>&1 || return 97
      inject cleanup-original-rename-after-mutation && return 98
      journal_applied cleanup-original-rename || return $?
    elif [ ! -f "$dir/prerouting-present" ] && chain_exists prerouting; then
      chain_owned prerouting || return 92
      journal_intent cleanup-malformed-flush || return $?
      nft flush chain ip "$table" prerouting >/dev/null 2>&1 || return 99
      inject cleanup-malformed-flush-after-mutation && return 100
      journal_applied cleanup-malformed-flush || return $?
      if chain_exists prerouting; then
        journal_intent cleanup-malformed-delete || return $?
        nft delete chain ip "$table" prerouting >/dev/null 2>&1 || return 101
        inject cleanup-malformed-delete-after-mutation && return 102
        journal_applied cleanup-malformed-delete || return $?
      fi
    fi

    if [ ! -f "$dir/output-present" ] && chain_exists output; then
      chain_owned output || return 92
      journal_intent cleanup-output-flush || return $?
      nft flush chain ip "$table" output >/dev/null 2>&1 || return 103
      inject cleanup-output-flush-after-mutation && return 104
      journal_applied cleanup-output-flush || return $?
      if chain_exists output; then
        journal_intent cleanup-output-delete || return $?
        nft delete chain ip "$table" output >/dev/null 2>&1 || return 105
        inject cleanup-output-delete-after-mutation && return 106
        journal_applied cleanup-output-delete || return $?
      fi
    fi
  fi

  if [ ! -f "$dir/had-fwmark-rule" ] \
    && ip rule show | grep -q 'fwmark 0x1.*lookup 100'; then
    journal_intent cleanup-fwmark-rule || return $?
    ip rule del fwmark 0x1 lookup 100 >/dev/null 2>&1 || return 93
    inject cleanup-fwmark-rule-after-mutation && return 94
    journal_applied cleanup-fwmark-rule || return $?
  fi
  if [ ! -f "$dir/had-local-route" ] \
    && ip route show table 100 | grep -q '^local .* dev lo'; then
    journal_intent cleanup-local-route || return $?
    ip route del local 0.0.0.0/0 dev lo table 100 >/dev/null 2>&1 || return 95
    inject cleanup-local-route-after-mutation && return 96
    journal_applied cleanup-local-route || return $?
  fi
  return 0
}

restore() {
  attempt=0
  restored=1
  while [ "$attempt" -lt 5 ]; do
    attempt=$((attempt + 1))
    if restore_once; then
      restored=0
      break
    fi
  done
  [ "$restored" -eq 0 ] || return 97

  if [ -f "$dir/table-present" ]; then
    table_exists || return 98
    if [ -f "$dir/prerouting-present" ]; then
      chain_exists prerouting || return 99
    else
      ! chain_exists prerouting || return 100
    fi
    if [ -f "$dir/saved-present" ]; then
      chain_exists "$saved" || return 101
    else
      ! chain_exists "$saved" || return 101
    fi
    if [ -f "$dir/output-present" ]; then
      chain_exists output || return 102
    else
      ! chain_exists output || return 103
    fi
  else
    ! table_exists || return 104
  fi
  if [ ! -f "$dir/had-fwmark-rule" ]; then
    ! ip rule show | grep -q 'fwmark 0x1.*lookup 100' || return 105
  fi
  if [ ! -f "$dir/had-local-route" ]; then
    ! ip route show table 100 | grep -q '^local .* dev lo' || return 106
  fi
  return 0
}

finish() {
  prior=$?
  trap '' HUP INT TERM
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

setup || exit $?

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
    let mut orphaned = DeltaScopedMalformedPrerouting::try_install(parent.id(), FaultPoint::NONE)
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
        FaultPoint("setup-original-rename-after-mutation"),
    );
    assert!(partial.is_err(), "the injected construction interruption is observable");
    assert_eq!(PacketPathBaseline::capture(), partial_baseline, "partial construction restoration");
}

const TABLE_SETUP_FAULTS: &[&str] = &[
    "setup-table-add-before-intent",
    "setup-table-add-after-intent",
    "setup-table-add-after-mutation",
    "signal:setup-table-add-after-mutation",
    "setup-table-add-before-applied",
    "setup-table-add-after-applied",
];
const RENAME_SETUP_FAULTS: &[&str] = &[
    "setup-original-rename-before-intent",
    "setup-original-rename-after-intent",
    "setup-original-rename-after-mutation",
    "signal:setup-original-rename-after-mutation",
    "setup-original-rename-before-applied",
    "setup-original-rename-after-applied",
];
const MALFORMED_SETUP_FAULTS: &[&str] = &[
    "setup-malformed-create-before-intent",
    "setup-malformed-create-after-intent",
    "setup-malformed-create-after-mutation",
    "signal:setup-malformed-create-after-mutation",
    "setup-malformed-create-before-applied",
    "setup-malformed-create-after-applied",
    "setup-ready-before-marker",
    "setup-ready-after-marker",
];
const TABLE_CLEANUP_FAULTS: &[&str] = &[
    "cleanup-table-delete-before-intent",
    "cleanup-table-delete-after-intent",
    "cleanup-table-delete-after-mutation",
    "signal:cleanup-table-delete-after-mutation",
    "cleanup-table-delete-before-applied",
    "cleanup-table-delete-after-applied",
];
const EXISTING_CHAIN_CLEANUP_FAULTS: &[&str] = &[
    "cleanup-malformed-flush-before-intent",
    "cleanup-malformed-flush-after-intent",
    "cleanup-malformed-flush-after-mutation",
    "signal:cleanup-malformed-flush-after-mutation",
    "cleanup-malformed-flush-before-applied",
    "cleanup-malformed-flush-after-applied",
    "cleanup-malformed-delete-before-intent",
    "cleanup-malformed-delete-after-intent",
    "cleanup-malformed-delete-after-mutation",
    "signal:cleanup-malformed-delete-after-mutation",
    "cleanup-malformed-delete-before-applied",
    "cleanup-malformed-delete-after-applied",
    "cleanup-original-rename-before-intent",
    "cleanup-original-rename-after-intent",
    "cleanup-original-rename-after-mutation",
    "signal:cleanup-original-rename-after-mutation",
    "cleanup-original-rename-before-applied",
    "cleanup-original-rename-after-applied",
];
const OUTPUT_CLEANUP_FAULTS: &[&str] = &[
    "cleanup-output-flush-before-intent",
    "cleanup-output-flush-after-intent",
    "cleanup-output-flush-after-mutation",
    "signal:cleanup-output-flush-after-mutation",
    "cleanup-output-flush-before-applied",
    "cleanup-output-flush-after-applied",
    "cleanup-output-delete-before-intent",
    "cleanup-output-delete-after-intent",
    "cleanup-output-delete-after-mutation",
    "signal:cleanup-output-delete-after-mutation",
    "cleanup-output-delete-before-applied",
    "cleanup-output-delete-after-applied",
];
const ABSENT_TABLE_PARENT_DEATH_FAULTS: &[&str] = &[
    "parent-death:setup-table-add-after-mutation",
    "parent-death:setup-malformed-create-after-mutation",
];
const PRODUCTION_PARENT_DEATH_FAULTS: &[&str] = &[
    "parent-death:setup-original-rename-after-mutation",
    "parent-death:setup-malformed-create-after-mutation",
];

struct OwnedDisposableTable(String);

impl OwnedDisposableTable {
    fn remove(mut self) {
        if PacketPathBaseline::capture_table(&self.0).nft_table.is_some() {
            assert!(self.0.starts_with("ovd-gti-"), "refuse a non-fixture table name");
            let status = Command::new("nft")
                .args(["delete", "table", "ip", &self.0])
                .status()
                .expect("remove owned disposable nft table");
            assert!(status.success(), "remove owned disposable nft table {}", self.0);
        }
        self.0.clear();
    }
}

impl Drop for OwnedDisposableTable {
    fn drop(&mut self) {
        if self.0.starts_with("ovd-gti-") {
            let _ = Command::new("nft").args(["delete", "table", "ip", &self.0]).status();
        }
    }
}

fn run_nft(args: &[&str]) {
    let output = Command::new("nft").args(args).output().expect("run disposable nft mutation");
    assert!(
        output.status.success(),
        "nft {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Every table-add intent, kernel-transaction, completion-marker, and READY
/// boundary restores an originally absent disposable table exactly.
#[test]
#[serial(cgroup)]
fn table_creation_gap_restores_an_absent_disposable_table() {
    let table = format!("ovd-gti-table-{}", std::process::id());
    let owned = OwnedDisposableTable(table.clone());
    for fault in TABLE_SETUP_FAULTS.iter().chain(MALFORMED_SETUP_FAULTS) {
        let before = PacketPathBaseline::capture_table(&table);
        assert!(before.nft_table.is_none(), "the disposable table starts absent");
        let result = DeltaScopedMalformedPrerouting::try_install_in(
            &table,
            std::process::id(),
            FaultPoint(fault),
        );
        if *fault == "setup-ready-after-marker" {
            let mut fixture = result.expect("READY may race the injected post-marker exit");
            let status = fixture.wait().expect("wait for post-READY injected exit");
            assert!(!status.success(), "the injected {fault} interruption is observable");
        } else {
            assert!(result.is_err(), "the injected {fault} interruption is observable");
        }
        assert_eq!(
            PacketPathBaseline::capture_table(&table),
            before,
            "{fault} restores the absent table and exact global FIB baseline"
        );
    }

    for fault in ABSENT_TABLE_PARENT_DEATH_FAULTS {
        let before = PacketPathBaseline::capture_table(&table);
        let mut parent = Command::new("sleep").arg("60").spawn().expect("spawn fault parent");
        let result =
            DeltaScopedMalformedPrerouting::try_install_in(&table, parent.id(), FaultPoint(fault));
        let status = parent.wait().expect("reap fault parent");
        assert!(!status.success(), "{fault} kills only its disposable parent");
        assert!(result.is_err(), "the injected {fault} interruption is observable");
        assert_eq!(
            PacketPathBaseline::capture_table(&table),
            before,
            "{fault} restores the absent table and exact global FIB baseline"
        );
    }
    owned.remove();
}

/// The production table's original chain survives every rename/create intent,
/// mutation, signal, parent-death, and marker boundary with exact handles,
/// programs, counters,
/// userdata, and FIB state.
#[test]
#[serial(cgroup)]
fn setup_fault_matrix_restores_the_exact_production_object_graph() {
    for fault in RENAME_SETUP_FAULTS.iter().chain(MALFORMED_SETUP_FAULTS) {
        let before = PacketPathBaseline::capture();
        let result =
            DeltaScopedMalformedPrerouting::try_install(std::process::id(), FaultPoint(fault));
        if *fault == "setup-ready-after-marker" {
            let mut fixture = result.expect("READY may race the injected post-marker exit");
            let status = fixture.wait().expect("wait for post-READY injected exit");
            assert!(!status.success(), "the injected {fault} interruption is observable");
        } else {
            assert!(result.is_err(), "the injected {fault} interruption is observable");
        }
        assert_eq!(PacketPathBaseline::capture(), before, "{fault} exact restoration");
    }

    for fault in PRODUCTION_PARENT_DEATH_FAULTS {
        let before = PacketPathBaseline::capture();
        let mut parent = Command::new("sleep").arg("60").spawn().expect("spawn fault parent");
        let result = DeltaScopedMalformedPrerouting::try_install(parent.id(), FaultPoint(fault));
        let status = parent.wait().expect("reap fault parent");
        assert!(!status.success(), "{fault} kills only its disposable parent");
        assert!(result.is_err(), "the injected {fault} interruption is observable");
        assert_eq!(PacketPathBaseline::capture(), before, "{fault} exact restoration");
    }
}

/// Cleanup is idempotent and retryable at every mutation and journal boundary;
/// live object inspection completes reconciliation after any injected gap.
#[test]
#[serial(cgroup)]
fn cleanup_fault_matrix_restores_exact_production_and_disposable_baselines() {
    for fault in EXISTING_CHAIN_CLEANUP_FAULTS {
        let fixture =
            DeltaScopedMalformedPrerouting::try_install(std::process::id(), FaultPoint(fault))
                .unwrap_or_else(|error| panic!("install cleanup fault {fault}: {error}"));
        fixture.finish();
    }

    let absent_table = format!("ovd-gti-delete-{}", std::process::id());
    let absent_owned = OwnedDisposableTable(absent_table.clone());
    for fault in TABLE_CLEANUP_FAULTS {
        let fixture = DeltaScopedMalformedPrerouting::try_install_in(
            &absent_table,
            std::process::id(),
            FaultPoint(fault),
        )
        .unwrap_or_else(|error| panic!("install table cleanup fault {fault}: {error}"));
        fixture.finish();
    }
    absent_owned.remove();

    let no_output_table = format!("ovd-gti-output-{}", std::process::id());
    let output_owned = OwnedDisposableTable(no_output_table.clone());
    run_nft(&["add", "table", "ip", &no_output_table]);
    run_nft(&[
        "add",
        "chain",
        "ip",
        &no_output_table,
        PREROUTING,
        "{ type filter hook prerouting priority mangle; policy accept; }",
    ]);
    for fault in OUTPUT_CLEANUP_FAULTS {
        let owner = format!("overdrive-gti-{}-{}", std::process::id(), std::process::id());
        let fixture = DeltaScopedMalformedPrerouting::try_install_in(
            &no_output_table,
            std::process::id(),
            FaultPoint(fault),
        )
        .unwrap_or_else(|error| panic!("install output cleanup fault {fault}: {error}"));
        run_nft(&[
            "add",
            "chain",
            "ip",
            &no_output_table,
            OUTPUT,
            &format!("{{ comment \"{owner}\"; }}"),
        ]);
        fixture.finish();
    }
    output_owned.remove();
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
    let expected_exprs = nft::mark_accept_exemption_exprs(MTLS_LEG_S_DIAL_MARK);
    let mut transaction = Vec::with_capacity(26);
    for (chain, handles) in
        [(PREROUTING, CONTAMINATED_PREROUTING_HANDLES), (OUTPUT, CONTAMINATED_OUTPUT_HANDLES)]
    {
        for handle in handles {
            transaction.push(nft::AtomicRuleMutation::Delete {
                table: TABLE,
                chain,
                handle: *handle,
            });
        }
        transaction.push(nft::AtomicRuleMutation::Insert {
            table: TABLE,
            chain,
            exprs: &expected_exprs,
            userdata: &expected_tag,
        });
    }
    nft::apply_rule_transaction_atomically(&transaction)
        .unwrap_or_else(|error| panic!("atomically repair audited contamination: {error}"));

    let after = PacketPathBaseline::capture();
    assert_canonical_clean_state(&after);
    assert_eq!((after.fib_rules, after.fib_routes), fib_before, "repair changes no FIB state");
}

/// The destructive 24-delete/two-insert branch is exercised on an exclusively
/// owned disposable table. A NACK injected before, between, or after every
/// operation rolls the complete kernel transaction back to the exact audited
/// start; the valid transaction then reaches the exact canonical state.
#[test]
#[serial(cgroup)]
fn audited_duplicate_repair_is_atomic_at_every_operation_boundary() {
    let table = format!("ovd-gti-repair-{}", std::process::id());
    let owned = OwnedDisposableTable(table.clone());
    run_nft(&["add", "table", "ip", &table]);
    run_nft(&["add", "chain", "ip", &table, PREROUTING]);
    run_nft(&["add", "chain", "ip", &table, OUTPUT]);

    let expected_exprs = nft::mark_accept_exemption_exprs(MTLS_LEG_S_DIAL_MARK);
    let expected_program = canonical_exemption_program();
    let expected_tag = nft::userdata_exemption();
    for chain in [PREROUTING, OUTPUT] {
        for _ in 0..12 {
            nft::insert_rule(&table, chain, &expected_exprs, &expected_tag)
                .unwrap_or_else(|error| panic!("seed owned {chain} duplicate: {error}"));
        }
    }

    let audited_start = PacketPathBaseline::capture_table(&table);
    let programs = rule_programs(audited_start.nft_table.as_ref().expect("disposable table"));
    let mut valid = Vec::with_capacity(26);
    for chain in [PREROUTING, OUTPUT] {
        let ownership = audited_start.ownership.get(chain).expect("disposable chain ownership");
        assert_eq!(ownership.len(), 12, "audit exact owned duplicate count in {chain}");
        for rule in ownership {
            assert_eq!(rule.userdata, expected_tag, "audit exact userdata in {chain}");
            assert_eq!(
                programs.get(&(chain.to_owned(), rule.handle)),
                Some(&expected_program),
                "audit exact expression program at {chain}/{}",
                rule.handle
            );
            valid.push(nft::AtomicRuleMutation::Delete {
                table: &table,
                chain,
                handle: rule.handle,
            });
        }
        valid.push(nft::AtomicRuleMutation::Insert {
            table: &table,
            chain,
            exprs: &expected_exprs,
            userdata: &expected_tag,
        });
    }

    for boundary in 0..=valid.len() {
        let mut failed = valid.clone();
        failed.insert(
            boundary,
            nft::AtomicRuleMutation::Delete {
                table: &table,
                chain: "gti-intentionally-absent",
                handle: u64::MAX,
            },
        );
        let error = nft::apply_rule_transaction_atomically(&failed)
            .expect_err("the injected unknown-handle delete rejects the transaction");
        assert_eq!(error.errno(), Some(-libc::ENOENT), "boundary {boundary} typed NACK");
        assert_eq!(
            PacketPathBaseline::capture_table(&table),
            audited_start,
            "boundary {boundary} rolls back handles/programs/counters/userdata and FIB exactly"
        );
    }

    nft::apply_rule_transaction_atomically(&valid)
        .unwrap_or_else(|error| panic!("commit the exact audited repair transaction: {error}"));
    let canonical = PacketPathBaseline::capture_table(&table);
    assert_canonical_clean_state(&canonical);
    assert_eq!(
        (canonical.fib_rules, canonical.fib_routes),
        (audited_start.fib_rules, audited_start.fib_routes),
        "atomic duplicate repair changes no FIB state"
    );
    owned.remove();
}
