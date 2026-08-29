//! Delta-scoped native nft fault fixture for the S-GTI-05 prerequisite.
//!
//! The later owner step still carries the scenario RED scaffold. These tests
//! close only the fixture contract: preserve the existing table object and
//! every unrelated rule while temporarily substituting the `prerouting`
//! chain, drive the real typed production failure, and restore exact
//! structural state on normal, panic, watchdog-signal, parent-death, and
//! partial-construction exits.

use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::Write as _;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_netlink::{Client, NetlinkError, block_on_host_netlink, nft};
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
const CLEAN_NETNS_BASELINE_TEST: &str = "integration::guest_stack_mtls_egress::fault_fixture::a_clean_network_namespace_captures_an_absent_table_100_route_baseline";
const CLEAN_NETNS_MODE: &str = "OVERDRIVE_GTI_CLEAN_NETNS_MODE";
const CLEAN_NETNS_PRODUCTION_TEST: &str = "integration::guest_stack_mtls_egress::fault_fixture::clean_network_namespace_real_installer_delta_is_exactly_reconciled";
const ORDINARY_FIXTURE_NETNS_MODE: &str = "OVERDRIVE_GTI_ORDINARY_FIXTURE_NETNS_MODE";
const ORDINARY_FIXTURE_TEST: &str = "integration::guest_stack_mtls_egress::fault_fixture::input_hook_fault_is_typed_and_normal_restoration_is_structurally_exact";
const CLEAN_AUDIT_DIR: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_DIR";
const CLEAN_AUDIT_TABLE: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_TABLE";
const CLEAN_AUDIT_OWNER: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_OWNER";
const CLEAN_AUDIT_SAVED: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_SAVED";
const CLEAN_AUDIT_ACTION: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_ACTION";
const CLEAN_BASELINE_NFT_FILE: &str = "baseline-nft.json";
const CLEAN_BASELINE_OWNERSHIP_FILE: &str = "baseline-nft-ownership.json";
const CLEAN_BASELINE_RULES_FILE: &str = "baseline-fib-rules.json";
const CLEAN_BASELINE_ROUTES_FILE: &str = "baseline-fib-routes.json";

const PRODUCTION_INTENTS: &[&str] = &[
    "production-fwmark-rule-add",
    "production-local-route-add",
    "production-table-ensure",
    "production-prerouting-exemption-insert",
    "production-output-chain-create",
    "production-output-exemption-insert",
    "production-egress-append",
];

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
        let inventory = json_output("nft", &["-j", "list", "tables"]);
        let table_present = inventory["nftables"]
            .as_array()
            .expect("nft table inventory has nftables array")
            .iter()
            .filter_map(|entry| entry.get("table"))
            .any(|table| {
                table.get("family").and_then(Value::as_str) == Some("ip")
                    && table.get("name").and_then(Value::as_str) == Some(table_name)
            });
        let nft_table = if table_present {
            let mut value = json_output("nft", &["-a", "-j", "list", "table", "ip", table_name]);
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
            fib_routes: table_100_routes(),
        }
    }
}

/// Query the complete IPv4 FIB and select table 100 from the typed JSON. Unlike
/// `ip route show table 100`, the complete dump succeeds when table 100 is
/// absent, while real command and decoder failures still fail the fixture.
fn table_100_routes() -> Value {
    let Value::Array(routes) = json_output("ip", &["-j", "route", "show", "table", "all"]) else {
        panic!("ip route JSON is not an array");
    };
    Value::Array(
        routes
            .into_iter()
            .filter(|route| {
                matches!(route.get("table"), Some(Value::String(table)) if table == "100")
                    || route.get("table").and_then(Value::as_u64) == Some(100)
            })
            .collect(),
    )
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

trait RecoveryMode {
    fn audit_test() -> Option<&'static str>;
}

struct FixtureOnlyRecovery;

impl RecoveryMode for FixtureOnlyRecovery {
    fn audit_test() -> Option<&'static str> {
        None
    }
}

struct ExactProductionRecovery;

impl RecoveryMode for ExactProductionRecovery {
    fn audit_test() -> Option<&'static str> {
        Some(CLEAN_NETNS_PRODUCTION_TEST)
    }
}

struct DeltaScopedMalformedPrerouting<R: RecoveryMode> {
    baseline: PacketPathBaseline,
    table_name: String,
    watchdog: Option<Child>,
    stop_file: PathBuf,
    watchdog_dir: TempDir,
    recovery: PhantomData<R>,
}

impl<R: RecoveryMode> DeltaScopedMalformedPrerouting<R> {
    fn try_install_typed(table_name: &str, parent: u32, fault: FaultPoint) -> Result<Self, String> {
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
        let (test_executable, test_name) = if let Some(test_name) = R::audit_test() {
            let audit_kind = if baseline.nft_table.is_some() {
                b"existing-table".as_slice()
            } else {
                b"absent-table".as_slice()
            };
            durable_write(dir, "allow-production-delta", audit_kind)?;
            durable_write(
                dir,
                CLEAN_BASELINE_NFT_FILE,
                &serde_json::to_vec(&baseline.nft_table).map_err(|error| error.to_string())?,
            )?;
            durable_write(
                dir,
                CLEAN_BASELINE_OWNERSHIP_FILE,
                &serde_json::to_vec(&ownership_to_json(&baseline.ownership))
                    .map_err(|error| error.to_string())?,
            )?;
            durable_write(
                dir,
                CLEAN_BASELINE_RULES_FILE,
                &serde_json::to_vec(&baseline.fib_rules).map_err(|error| error.to_string())?,
            )?;
            durable_write(
                dir,
                CLEAN_BASELINE_ROUTES_FILE,
                &serde_json::to_vec(&baseline.fib_routes).map_err(|error| error.to_string())?,
            )?;
            (
                env::current_exe()
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .into_owned(),
                test_name.to_owned(),
            )
        } else {
            (String::new(), String::new())
        };
        std::fs::write(&script, watchdog_script()).map_err(|error| error.to_string())?;
        let watchdog = Command::new("sh")
            .arg(&script)
            .arg(parent.to_string())
            .arg(dir)
            .arg(&saved_chain)
            .arg(table_name)
            .arg(fault.0)
            .arg(&owner)
            .arg(&test_executable)
            .arg(&test_name)
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut fixture = Self {
            baseline,
            table_name: table_name.to_owned(),
            watchdog: Some(watchdog),
            stop_file,
            watchdog_dir,
            recovery: PhantomData,
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

impl DeltaScopedMalformedPrerouting<FixtureOnlyRecovery> {
    fn install_fixture_only() -> Self {
        Self::try_install_fixture_only(std::process::id(), FaultPoint::NONE)
            .unwrap_or_else(|error| panic!("install fixture-only INPUT-hook substitute: {error}"))
    }

    fn try_install_fixture_only(parent: u32, fault: FaultPoint) -> Result<Self, String> {
        Self::try_install_fixture_only_in(TABLE, parent, fault)
    }

    fn try_install_fixture_only_in(
        table_name: &str,
        parent: u32,
        fault: FaultPoint,
    ) -> Result<Self, String> {
        Self::try_install_typed(table_name, parent, fault)
    }
}

impl DeltaScopedMalformedPrerouting<ExactProductionRecovery> {
    fn try_install_production(parent: u32, fault: FaultPoint) -> Result<Self, String> {
        Self::try_install_typed(TABLE, parent, fault)
    }

    /// Record every shared mutation the unchanged production installer could
    /// perform before giving it authority to touch the namespace. Each record
    /// is fsynced independently so recovery never depends on a later marker.
    fn journal_production_intents(&self, fault: Option<&str>) -> Result<(), String> {
        for action in PRODUCTION_INTENTS {
            let before = format!("{action}-before-intent");
            if fault == Some(before.as_str()) {
                return Err(format!("injected journal failure at {before}"));
            }
            durable_write(self.watchdog_dir.path(), &format!("intent-{action}"), b"")?;
            let after = format!("{action}-after-intent");
            if fault == Some(after.as_str()) {
                return Err(format!("injected journal failure at {after}"));
            }
        }
        Ok(())
    }

    fn invoke_real_installer_expect_typed_input_hook_failure(&self) {
        self.journal_production_intents(None)
            .expect("durably journal every possible production mutation");
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
    }
}

impl<R: RecoveryMode> Drop for DeltaScopedMalformedPrerouting<R> {
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

fn durable_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = dir.join(name);
    let mut file =
        File::create(&path).map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(bytes).map_err(|error| format!("write {}: {error}", path.display()))?;
    file.sync_all().map_err(|error| format!("sync {}: {error}", path.display()))?;
    File::open(dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync journal directory {}: {error}", dir.display()))
}

fn is_production_fwmark_rule(rule: &Value) -> bool {
    rule == &json!({
        "priority": 32765,
        "src": "all",
        "fwmark": "0x1",
        "table": "100"
    })
}

fn is_production_local_route(route: &Value) -> bool {
    route
        == &json!({
            "type": "local",
            "dst": "default",
            "dev": "lo",
            "table": "100",
            "protocol": "static",
            "scope": "host",
            "flags": []
        })
}

fn ownership_to_json(ownership: &BTreeMap<String, Vec<nft::RuleInfo>>) -> Value {
    Value::Object(
        ownership
            .iter()
            .map(|(chain, rules)| {
                (
                    chain.clone(),
                    Value::Array(
                        rules
                            .iter()
                            .map(|rule| json!({"handle": rule.handle, "userdata": rule.userdata}))
                            .collect(),
                    ),
                )
            })
            .collect(),
    )
}

fn ownership_from_json(value: &Value) -> Option<BTreeMap<String, Vec<nft::RuleInfo>>> {
    value.as_object().map(|chains| {
        chains
            .iter()
            .map(|(chain, rules)| {
                let rules = rules
                    .as_array()?
                    .iter()
                    .map(|rule| {
                        Some(nft::RuleInfo {
                            handle: rule.get("handle")?.as_u64()?,
                            userdata: serde_json::from_value(rule.get("userdata")?.clone()).ok()?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some((chain.clone(), rules))
            })
            .collect::<Option<BTreeMap<_, _>>>()
    })?
}

#[derive(Debug)]
struct NftObjects {
    table: Value,
    chains: BTreeMap<String, Value>,
    rules: BTreeMap<String, Vec<Value>>,
}

fn split_nft_objects(document: &Value) -> Option<NftObjects> {
    let entries = document.get("nftables")?.as_array()?;
    let mut table = None;
    let mut chains = BTreeMap::new();
    let mut rules: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for entry in entries {
        if let Some(value) = entry.get("table") {
            if table.replace(value.clone()).is_some() {
                return None;
            }
        } else if let Some(value) = entry.get("chain") {
            let name = value.get("name")?.as_str()?.to_owned();
            if chains.insert(name, value.clone()).is_some() {
                return None;
            }
        } else if let Some(value) = entry.get("rule") {
            let chain = value.get("chain")?.as_str()?.to_owned();
            rules.entry(chain).or_default().push(value.clone());
        } else {
            return None;
        }
    }
    Some(NftObjects { table: table?, chains, rules })
}

fn normalized_named_object(value: &Value, field: &str, name: &str) -> Option<Value> {
    let mut normalized = value.clone();
    normalized.as_object_mut()?.insert(field.to_owned(), Value::String(name.to_owned()));
    Some(normalized)
}

fn without_handle(value: &Value) -> Option<Value> {
    let mut normalized = value.clone();
    normalized.as_object_mut()?.remove("handle")?;
    Some(normalized)
}

fn exact_fixture_chain(chain: &Value, table_name: &str, owner: &str) -> bool {
    without_handle(chain)
        == Some(json!({
            "family": "ip",
            "table": table_name,
            "name": PREROUTING,
            "type": "filter",
            "hook": "input",
            "prio": -150,
            "policy": "accept",
            "comment": owner,
        }))
}

fn exact_production_output_chain(chain: &Value, table_name: &str) -> bool {
    without_handle(chain)
        == Some(json!({
            "family": "ip",
            "table": table_name,
            "name": OUTPUT,
            "type": "route",
            "hook": "output",
            "prio": -150,
            "policy": "accept",
        }))
}

fn exact_created_table(table: &Value, table_name: &str, owner: &str) -> bool {
    without_handle(table)
        == Some(json!({
            "family": "ip",
            "name": table_name,
            "comment": owner,
        }))
}

#[derive(Clone, Copy, Debug)]
struct ExactDeltaMismatch;

fn exemption_delta_handle(
    table_name: &str,
    chain: &str,
    infos: &[nft::RuleInfo],
    rules: &[Value],
) -> Result<Option<u64>, ExactDeltaMismatch> {
    if infos.is_empty() && rules.is_empty() {
        return Ok(None);
    }
    let [info] = infos else {
        return Err(ExactDeltaMismatch);
    };
    let [rule] = rules else {
        return Err(ExactDeltaMismatch);
    };
    if info.userdata != nft::userdata_exemption()
        || rule.get("handle").and_then(Value::as_u64) != Some(info.handle)
        || without_handle(rule)
            != Some(json!({
                "family": "ip",
                "table": table_name,
                "chain": chain,
                "expr": canonical_exemption_program(),
            }))
    {
        return Err(ExactDeltaMismatch);
    }
    Ok(Some(info.handle))
}

#[derive(Clone, Copy, Debug, Default)]
struct ExactNftDelta {
    created_table: bool,
    created_prerouting: bool,
    created_output: bool,
    prerouting_exemption: Option<u64>,
    output_exemption: Option<u64>,
}

fn exact_nft_delta(
    baseline: &PacketPathBaseline,
    current: &PacketPathBaseline,
    table_name: &str,
    owner: &str,
    saved_name: &str,
) -> Option<ExactNftDelta> {
    let Some(baseline_document) = &baseline.nft_table else {
        let Some(current_document) = &current.nft_table else {
            return current.ownership.is_empty().then_some(ExactNftDelta::default());
        };
        let mut current_objects = split_nft_objects(current_document)?;
        if !exact_created_table(&current_objects.table, table_name, owner) {
            return None;
        }
        let mut delta = ExactNftDelta { created_table: true, ..ExactNftDelta::default() };
        if let Some(chain) = current_objects.chains.remove(PREROUTING) {
            if !exact_fixture_chain(&chain, table_name, owner) {
                return None;
            }
            let infos = current.ownership.get(PREROUTING)?;
            let rules = current_objects.rules.remove(PREROUTING).unwrap_or_default();
            delta.prerouting_exemption =
                exemption_delta_handle(table_name, PREROUTING, infos, &rules).ok()?;
            delta.created_prerouting = true;
        }
        if let Some(chain) = current_objects.chains.remove(OUTPUT) {
            if !exact_production_output_chain(&chain, table_name) {
                return None;
            }
            let infos = current.ownership.get(OUTPUT)?;
            let rules = current_objects.rules.remove(OUTPUT).unwrap_or_default();
            delta.output_exemption =
                exemption_delta_handle(table_name, OUTPUT, infos, &rules).ok()?;
            delta.created_output = true;
        }
        if !current_objects.chains.is_empty()
            || !current_objects.rules.is_empty()
            || current.ownership.len()
                != usize::from(delta.created_prerouting) + usize::from(delta.created_output)
        {
            return None;
        }
        return Some(delta);
    };

    let current_document = current.nft_table.as_ref()?;
    let baseline_objects = split_nft_objects(baseline_document)?;
    let mut current_objects = split_nft_objects(current_document)?;
    if current_objects.table != baseline_objects.table {
        return None;
    }
    if baseline.ownership.len() != baseline_objects.chains.len() {
        return None;
    }

    let mut current_ownership = current.ownership.clone();
    let mut delta = ExactNftDelta::default();
    let mut original_is_saved = false;
    for (baseline_name, baseline_chain) in &baseline_objects.chains {
        let current_name =
            if baseline_name == PREROUTING && current_objects.chains.contains_key(saved_name) {
                original_is_saved = true;
                saved_name
            } else {
                baseline_name
            };
        let current_chain = current_objects.chains.remove(current_name)?;
        if normalized_named_object(&current_chain, "name", baseline_name)? != *baseline_chain {
            return None;
        }

        let baseline_infos = baseline.ownership.get(baseline_name)?;
        let live_infos = current_ownership.remove(current_name)?;
        let baseline_handles = baseline_infos.iter().map(|rule| rule.handle).collect::<Vec<_>>();
        let preserved_infos = live_infos
            .iter()
            .filter(|rule| baseline_handles.contains(&rule.handle))
            .cloned()
            .collect::<Vec<_>>();
        if preserved_infos != *baseline_infos {
            return None;
        }

        let baseline_rules = baseline_objects.rules.get(baseline_name).cloned().unwrap_or_default();
        let live_rules = current_objects.rules.remove(current_name).unwrap_or_default();
        let preserved_rules = live_rules
            .iter()
            .filter(|rule| {
                rule.get("handle")
                    .and_then(Value::as_u64)
                    .is_some_and(|handle| baseline_handles.contains(&handle))
            })
            .map(|rule| normalized_named_object(rule, "chain", baseline_name))
            .collect::<Option<Vec<_>>>()?;
        if preserved_rules != baseline_rules {
            return None;
        }

        let added_infos = live_infos
            .iter()
            .filter(|rule| !baseline_handles.contains(&rule.handle))
            .cloned()
            .collect::<Vec<_>>();
        let added_rules = live_rules
            .iter()
            .filter(|rule| {
                rule.get("handle")
                    .and_then(Value::as_u64)
                    .is_none_or(|handle| !baseline_handles.contains(&handle))
            })
            .cloned()
            .collect::<Vec<_>>();
        if baseline_name == OUTPUT && !nft::has_exemption(baseline_infos) {
            delta.output_exemption =
                exemption_delta_handle(table_name, OUTPUT, &added_infos, &added_rules).ok()?;
        } else if !added_infos.is_empty() || !added_rules.is_empty() {
            return None;
        }
    }

    if (original_is_saved || !baseline_objects.chains.contains_key(PREROUTING))
        && let Some(chain) = current_objects.chains.remove(PREROUTING)
    {
        if !exact_fixture_chain(&chain, table_name, owner) {
            return None;
        }
        let infos = current_ownership.remove(PREROUTING)?;
        let rules = current_objects.rules.remove(PREROUTING).unwrap_or_default();
        delta.prerouting_exemption =
            exemption_delta_handle(table_name, PREROUTING, &infos, &rules).ok()?;
        delta.created_prerouting = true;
    }
    if !baseline_objects.chains.contains_key(OUTPUT)
        && let Some(chain) = current_objects.chains.remove(OUTPUT)
    {
        if !exact_production_output_chain(&chain, table_name) {
            return None;
        }
        let infos = current_ownership.remove(OUTPUT)?;
        let rules = current_objects.rules.remove(OUTPUT).unwrap_or_default();
        delta.output_exemption = exemption_delta_handle(table_name, OUTPUT, &infos, &rules).ok()?;
        delta.created_output = true;
    }
    if !current_objects.chains.is_empty()
        || !current_objects.rules.is_empty()
        || !current_ownership.is_empty()
    {
        return None;
    }
    Some(delta)
}

fn parse_u32_rendering(value: &Value) -> Option<u32> {
    if let Some(number) = value.as_u64() {
        return u32::try_from(number).ok();
    }
    let text = value.as_str()?.split('/').next()?;
    text.strip_prefix("0x")
        .map_or_else(|| text.parse().ok(), |hex| u32::from_str_radix(hex, 16).ok())
}

fn is_adopted_fwmark_rule(rule: &Value) -> bool {
    rule.get("fwmark").and_then(parse_u32_rendering) == Some(1)
        && rule.get("table").and_then(parse_u32_rendering) == Some(100)
}

fn is_adopted_local_route(route: &Value) -> bool {
    route.get("type").and_then(Value::as_str) == Some("local")
        && route.get("dst").and_then(Value::as_str) == Some("default")
        && route.get("dev").and_then(Value::as_str) == Some("lo")
        && route.get("table").and_then(parse_u32_rendering) == Some(100)
        && route.get("scope").and_then(Value::as_str) == Some("host")
        && route.get("gateway").is_none()
}

fn exact_optional_delta(
    baseline: &Value,
    current: &Value,
    adopted: fn(&Value) -> bool,
    canonical_delta: fn(&Value) -> bool,
) -> Option<bool> {
    let baseline_array = baseline.as_array()?;
    let current_array = current.as_array()?;
    let mut residual = current_array.clone();
    for expected in baseline_array {
        let position = residual.iter().position(|candidate| candidate == expected)?;
        residual.remove(position);
    }
    if baseline_array.iter().any(adopted) {
        residual.is_empty().then_some(false)
    } else if residual.is_empty() {
        Some(false)
    } else {
        (residual.len() == 1 && canonical_delta(&residual[0])).then_some(true)
    }
}

#[derive(Clone, Copy, Debug)]
struct ExactProductionDelta {
    nft: ExactNftDelta,
    created_fwmark_rule: bool,
    created_local_route: bool,
}

fn read_json(path: &Path) -> Value {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("read clean-delta audit {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("decode clean-delta audit {}: {error}", path.display()))
}

fn audit_clean_production_delta_from_environment() {
    let dir = PathBuf::from(env::var_os(CLEAN_AUDIT_DIR).expect("clean audit directory"));
    let table_name = env::var(CLEAN_AUDIT_TABLE).expect("clean audit table");
    let owner = env::var(CLEAN_AUDIT_OWNER).expect("clean audit owner");
    let saved_name = env::var(CLEAN_AUDIT_SAVED).expect("clean audit saved chain");
    let baseline = PacketPathBaseline {
        nft_table: serde_json::from_value(read_json(&dir.join(CLEAN_BASELINE_NFT_FILE)))
            .expect("decode exact baseline nft document"),
        ownership: ownership_from_json(&read_json(&dir.join(CLEAN_BASELINE_OWNERSHIP_FILE)))
            .expect("decode exact baseline nft ownership"),
        fib_rules: read_json(&dir.join(CLEAN_BASELINE_RULES_FILE)),
        fib_routes: read_json(&dir.join(CLEAN_BASELINE_ROUTES_FILE)),
    };
    let current = PacketPathBaseline::capture_table(&table_name);
    let delta = ExactProductionDelta {
        nft: exact_nft_delta(&baseline, &current, &table_name, &owner, &saved_name).unwrap_or_else(
            || panic!("refuse cleanup of a foreign or noncanonical nft delta: {current:#?}"),
        ),
        created_fwmark_rule: exact_optional_delta(
            &baseline.fib_rules,
            &current.fib_rules,
            is_adopted_fwmark_rule,
            is_production_fwmark_rule,
        )
        .expect("refuse cleanup of foreign FIB-rule state"),
        created_local_route: exact_optional_delta(
            &baseline.fib_routes,
            &current.fib_routes,
            is_adopted_local_route,
            is_production_local_route,
        )
        .expect("refuse cleanup of foreign table-100 route state"),
    };
    match env::var(CLEAN_AUDIT_ACTION).as_deref().unwrap_or("audit") {
        "delete-table" if delta.nft.created_table => {
            nft::delete_table(&table_name).expect("delete exact production-created nft table");
        }
        "delete-prerouting-exemption" => {
            if let Some(handle) = delta.nft.prerouting_exemption {
                nft::delete_rule(&table_name, PREROUTING, handle)
                    .expect("delete exact production-created prerouting exemption");
            }
        }
        "delete-prerouting-chain"
            if delta.nft.created_prerouting && delta.nft.prerouting_exemption.is_none() =>
        {
            nft::delete_chain(&table_name, PREROUTING)
                .expect("delete exact fixture-created prerouting chain");
        }
        "delete-output-exemption" => {
            if let Some(handle) = delta.nft.output_exemption {
                nft::delete_rule(&table_name, OUTPUT, handle)
                    .expect("delete exact production-created output exemption");
            }
        }
        "delete-output-chain"
            if delta.nft.created_output && delta.nft.output_exemption.is_none() =>
        {
            nft::delete_chain(&table_name, OUTPUT)
                .expect("delete exact production-created output chain");
        }
        "delete-fwmark-rule" if delta.created_fwmark_rule => {
            let deleted = block_on_host_netlink(|| async {
                Client::new()?.delete_unique_fib_rule_fwmark(1, 100).await
            })
            .expect("delete exact production-created fwmark rule");
            assert!(deleted, "the audited production-created fwmark rule exists");
        }
        "delete-local-route" if delta.created_local_route => {
            let deleted = block_on_host_netlink(|| async {
                Client::new()?.delete_unique_local_route(100, "lo").await
            })
            .expect("delete exact production-created local route");
            assert!(deleted, "the audited production-created local route exists");
        }
        "audit"
        | "delete-table"
        | "delete-prerouting-chain"
        | "delete-output-chain"
        | "delete-fwmark-rule"
        | "delete-local-route" => {}
        other => panic!("unknown or unsafe clean-delta action {other}"),
    }
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
test_executable="$7"
test_name="$8"

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

clean_delta_action() {
  action="$1"
  [ -n "$test_executable" ] && [ -n "$test_name" ] || return 1
  OVERDRIVE_GTI_CLEAN_NETNS_MODE=audit \
    OVERDRIVE_GTI_CLEAN_AUDIT_DIR="$dir" \
    OVERDRIVE_GTI_CLEAN_AUDIT_TABLE="$table" \
    OVERDRIVE_GTI_CLEAN_AUDIT_OWNER="$owner" \
    OVERDRIVE_GTI_CLEAN_AUDIT_SAVED="$saved" \
    OVERDRIVE_GTI_CLEAN_AUDIT_ACTION="$action" \
    "$test_executable" --exact "$test_name" --nocapture >/dev/null 2>&1
}

clean_delta_is_exact() {
  clean_delta_action audit
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
  if [ -f "$dir/allow-production-delta" ]; then
    clean_delta_is_exact || return 88
  fi
  if [ ! -f "$dir/table-present" ]; then
    if table_exists; then
      if [ -f "$dir/allow-production-delta" ]; then
        journal_intent cleanup-table-delete || return $?
        clean_delta_action delete-table || return 90
        inject cleanup-table-delete-after-mutation && return 91
        journal_applied cleanup-table-delete || return $?
      else
        table_owned || return 89
        owned_table_is_disposable || return 89
        journal_intent cleanup-table-delete || return $?
        nft delete table ip "$table" >/dev/null 2>&1 || return 90
        inject cleanup-table-delete-after-mutation && return 91
        journal_applied cleanup-table-delete || return $?
      fi
    fi
  else
    table_exists || return 92
    if [ -f "$dir/allow-production-delta" ]; then
      journal_intent cleanup-malformed-flush || return $?
      clean_delta_action delete-prerouting-exemption || return 93
      inject cleanup-malformed-flush-after-mutation && return 94
      journal_applied cleanup-malformed-flush || return $?

      journal_intent cleanup-malformed-delete || return $?
      clean_delta_action delete-prerouting-chain || return 95
      inject cleanup-malformed-delete-after-mutation && return 96
      journal_applied cleanup-malformed-delete || return $?

      if chain_exists "$saved"; then
        journal_intent cleanup-original-rename || return $?
        nft rename chain ip "$table" "$saved" prerouting >/dev/null 2>&1 || return 97
        inject cleanup-original-rename-after-mutation && return 98
        journal_applied cleanup-original-rename || return $?
      fi

      journal_intent cleanup-output-flush || return $?
      clean_delta_action delete-output-exemption || return 103
      inject cleanup-output-flush-after-mutation && return 104
      journal_applied cleanup-output-flush || return $?

      journal_intent cleanup-output-delete || return $?
      clean_delta_action delete-output-chain || return 105
      inject cleanup-output-delete-after-mutation && return 106
      journal_applied cleanup-output-delete || return $?
    else
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
  fi

  if [ -f "$dir/allow-production-delta" ]; then
    journal_intent cleanup-fwmark-rule || return $?
    clean_delta_action delete-fwmark-rule || return 93
    inject cleanup-fwmark-rule-after-mutation && return 94
    journal_applied cleanup-fwmark-rule || return $?

    journal_intent cleanup-local-route || return $?
    clean_delta_action delete-local-route || return 95
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

  if [ -f "$dir/allow-production-delta" ]; then
    clean_delta_is_exact || return 98
  elif [ -f "$dir/table-present" ]; then
    table_exists || return 99
    if [ -f "$dir/prerouting-present" ]; then
      chain_exists prerouting || return 100
    else
      ! chain_exists prerouting || return 101
    fi
    if [ -f "$dir/saved-present" ]; then
      chain_exists "$saved" || return 102
    else
      ! chain_exists "$saved" || return 102
    fi
    if [ -f "$dir/output-present" ]; then
      chain_exists output || return 103
    else
      ! chain_exists output || return 104
    fi
  else
    ! table_exists || return 105
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
    let fixture = install_clean_fixture(FaultPoint::NONE);
    fixture.invoke_real_installer_expect_typed_input_hook_failure();
    fixture.finish();
}

/// The real encoded INPUT-hook failure retains its complete typed cause chain,
/// while the delta-scoped substitute restores every unrelated structural fact.
#[test]
#[serial(cgroup)]
fn input_hook_fault_is_typed_and_normal_restoration_is_structurally_exact() {
    if env::var_os(ORDINARY_FIXTURE_NETNS_MODE).is_some() {
        run_nft(&["add", "table", "ip", TABLE]);
        run_nft(&[
            "add",
            "chain",
            "ip",
            TABLE,
            PREROUTING,
            "{ type filter hook prerouting priority mangle; policy accept; }",
        ]);
        run_nft(&[
            "add",
            "chain",
            "ip",
            TABLE,
            OUTPUT,
            "{ type route hook output priority mangle; policy accept; }",
        ]);
        let baseline = PacketPathBaseline::capture_table(TABLE);
        assert_typed_input_hook_failure();
        let restored = PacketPathBaseline::capture_table(TABLE);
        assert_eq!(restored, baseline, "ordinary fixture restores the exact counterexample");
        assert!(restored.ownership.get(OUTPUT).is_some_and(Vec::is_empty));
        assert!(
            !restored
                .fib_rules
                .as_array()
                .is_some_and(|rules| { rules.iter().any(is_adopted_fwmark_rule) })
        );
        assert!(
            !restored
                .fib_routes
                .as_array()
                .is_some_and(|routes| { routes.iter().any(is_adopted_local_route) })
        );
        run_nft(&["delete", "table", "ip", TABLE]);
        return;
    }

    let status = Command::new("unshare")
        .arg("--net")
        .arg(env::current_exe().expect("locate ordinary fixture test executable"))
        .args(["--exact", ORDINARY_FIXTURE_TEST, "--nocapture"])
        .env(ORDINARY_FIXTURE_NETNS_MODE, "counterexample")
        .status()
        .expect("run ordinary fixture counterexample in a disposable network namespace");
    assert!(status.success(), "ordinary fixture counterexample child failed: {status}");
}

/// Every trapped fixture exit restores the exact normalized packet-path
/// baseline; no whole-table delete/replay is used when the table pre-exists.
#[test]
#[serial(cgroup)]
fn trapped_fixture_paths_restore_programs_counters_handles_userdata_and_fib() {
    let panic_baseline = PacketPathBaseline::capture();
    let panic_result = std::panic::catch_unwind(|| {
        let _fixture =
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::install_fixture_only();
        panic!("intentional fixture-body panic");
    });
    assert!(panic_result.is_err());
    assert_eq!(PacketPathBaseline::capture(), panic_baseline, "panic/drop restoration");

    let signal_baseline = PacketPathBaseline::capture();
    let mut signalled =
        DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::install_fixture_only();
    let status = signalled.signal_and_wait(libc::SIGTERM);
    assert_eq!(status.code(), Some(128), "watchdog signal path exits through its trap");
    assert_eq!(PacketPathBaseline::capture(), signal_baseline, "signal restoration");

    let parent_baseline = PacketPathBaseline::capture();
    let mut parent =
        Command::new("sleep").arg("60").spawn().expect("spawn disposable watchdog parent");
    let mut orphaned =
        DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only(
            parent.id(),
            FaultPoint::NONE,
        )
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
    let partial = DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only(
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
const FIB_CLEANUP_FAULTS: &[&str] = &[
    "cleanup-fwmark-rule-before-intent",
    "cleanup-fwmark-rule-after-intent",
    "cleanup-fwmark-rule-after-mutation",
    "signal:cleanup-fwmark-rule-after-mutation",
    "cleanup-fwmark-rule-before-applied",
    "cleanup-fwmark-rule-after-applied",
    "cleanup-local-route-before-intent",
    "cleanup-local-route-after-intent",
    "cleanup-local-route-after-mutation",
    "signal:cleanup-local-route-after-mutation",
    "cleanup-local-route-before-applied",
    "cleanup-local-route-after-applied",
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
const CREATED_PREROUTING_CLEANUP_FAULTS: &[&str] = &[
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
        let result =
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only_in(
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
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only_in(
                &table,
                parent.id(),
                FaultPoint(fault),
            );
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
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only(
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
        assert_eq!(PacketPathBaseline::capture(), before, "{fault} exact restoration");
    }

    for fault in PRODUCTION_PARENT_DEATH_FAULTS {
        let before = PacketPathBaseline::capture();
        let mut parent = Command::new("sleep").arg("60").spawn().expect("spawn fault parent");
        let result =
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only(
                parent.id(),
                FaultPoint(fault),
            );
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
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only(
                std::process::id(),
                FaultPoint(fault),
            )
            .unwrap_or_else(|error| panic!("install cleanup fault {fault}: {error}"));
        fixture.finish();
    }

    let absent_table = format!("ovd-gti-delete-{}", std::process::id());
    let absent_owned = OwnedDisposableTable(absent_table.clone());
    for fault in TABLE_CLEANUP_FAULTS {
        let fixture =
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only_in(
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
        let fixture =
            DeltaScopedMalformedPrerouting::<FixtureOnlyRecovery>::try_install_fixture_only_in(
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

/// An absent table 100 is valid clean-state data, not an iproute2 query
/// failure. Run this proof in a child process so the test harness thread never
/// changes the host network namespace.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(cgroup)]
fn a_clean_network_namespace_captures_an_absent_table_100_route_baseline() {
    if env::var_os(CLEAN_NETNS_MODE).is_some() {
        let baseline = PacketPathBaseline::capture_table(TABLE);
        assert_eq!(baseline.fib_routes, json!([]));
        return;
    }

    let status = Command::new("unshare")
        .arg("--net")
        .arg(env::current_exe().expect("locate this integration test executable"))
        .args(["--exact", CLEAN_NETNS_BASELINE_TEST, "--nocapture"])
        .env(CLEAN_NETNS_MODE, "baseline")
        .status()
        .expect("run clean native network-namespace baseline proof");
    assert!(status.success(), "clean network-namespace child failed: {status}");
}

fn assert_clean_fixture_baseline(baseline: &PacketPathBaseline, context: &str) {
    assert_eq!(
        PacketPathBaseline::capture_table(TABLE),
        *baseline,
        "{context}: exact clean nft/FIB baseline is restored"
    );
}

fn install_clean_fixture(
    fault: FaultPoint,
) -> DeltaScopedMalformedPrerouting<ExactProductionRecovery> {
    DeltaScopedMalformedPrerouting::try_install_production(std::process::id(), fault)
        .unwrap_or_else(|error| panic!("install clean production fixture: {error}"))
}

fn run_ip(args: &[&str]) {
    let output = Command::new("ip").args(args).output().expect("run disposable FIB mutation");
    assert!(
        output.status.success(),
        "ip {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn manually_remove_test_owned_clean_delta() {
    run_nft(&["delete", "table", "ip", TABLE]);
    run_ip(&["rule", "del", "fwmark", "0x1", "lookup", "100"]);
    run_ip(&["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"]);
}

fn exercise_clean_production_partition(
    baseline: &PacketPathBaseline,
    context: &str,
    cleanup_faults: &[&'static str],
) {
    let fixture = install_clean_fixture(FaultPoint::NONE);
    fixture.invoke_real_installer_expect_typed_input_hook_failure();
    fixture.finish();
    assert_clean_fixture_baseline(baseline, &format!("{context} normal path"));

    for fault in cleanup_faults {
        let fixture = install_clean_fixture(FaultPoint(fault));
        fixture.invoke_real_installer_expect_typed_input_hook_failure();
        fixture.finish();
        assert_clean_fixture_baseline(baseline, &format!("{context} {fault}"));
    }
}

fn run_clean_production_delta_scenarios() {
    let truly_absent = PacketPathBaseline::capture_table(TABLE);
    assert!(truly_absent.nft_table.is_none());
    assert_eq!(truly_absent.fib_routes, json!([]));
    let fixture = install_clean_fixture(FaultPoint::NONE);
    fixture.invoke_real_installer_expect_typed_input_hook_failure();
    fixture.finish();
    assert_clean_fixture_baseline(&truly_absent, "normal absent-table-100 path");

    let route = Command::new("ip")
        .args(["route", "add", "blackhole", "198.51.100.0/24", "table", "100"])
        .output()
        .expect("add unrelated table-100 route");
    assert!(
        route.status.success(),
        "add unrelated table-100 route: {}",
        String::from_utf8_lossy(&route.stderr)
    );
    let unrelated_fib = PacketPathBaseline::capture_table(TABLE);
    assert_eq!(unrelated_fib.fib_routes.as_array().map(Vec::len), Some(1));

    for fault in
        ["signal:setup-table-add-after-mutation", "signal:setup-malformed-create-after-mutation"]
    {
        let result =
            DeltaScopedMalformedPrerouting::<ExactProductionRecovery>::try_install_production(
                std::process::id(),
                FaultPoint(fault),
            );
        assert!(result.is_err(), "{fault} is observable");
        assert_clean_fixture_baseline(&unrelated_fib, fault);
    }

    for fault in TABLE_CLEANUP_FAULTS.iter().chain(FIB_CLEANUP_FAULTS) {
        let fixture = install_clean_fixture(FaultPoint(fault));
        fixture.invoke_real_installer_expect_typed_input_hook_failure();
        fixture.finish();
        assert_clean_fixture_baseline(&unrelated_fib, fault);
    }

    for action in PRODUCTION_INTENTS {
        for boundary in ["before-intent", "after-intent"] {
            let fault = format!("{action}-{boundary}");
            let fixture = install_clean_fixture(FaultPoint::NONE);
            let error = fixture
                .journal_production_intents(Some(&fault))
                .expect_err("journal interruption must withhold production authority");
            assert!(error.contains(&fault));
            fixture.finish();
            assert_clean_fixture_baseline(&unrelated_fib, &fault);
        }
    }

    let mut foreign_nft = install_clean_fixture(FaultPoint::NONE);
    foreign_nft.invoke_real_installer_expect_typed_input_hook_failure();
    run_nft(&["add", "chain", "ip", TABLE, "foreign-chain"]);
    run_nft(&["add", "rule", "ip", TABLE, "foreign-chain", "counter"]);
    let status = foreign_nft.stop_and_wait().expect("wait for fail-closed foreign-nft audit");
    assert!(!status.success(), "foreign nft state must refuse cleanup");
    let refused = PacketPathBaseline::capture_table(TABLE);
    assert!(
        refused
            .nft_table
            .as_ref()
            .is_some_and(|table| chain_names(table).iter().any(|chain| chain == "foreign-chain")),
        "failed-closed recovery preserves the foreign nft object"
    );
    manually_remove_test_owned_clean_delta();
    assert_clean_fixture_baseline(&unrelated_fib, "manual teardown after foreign-nft refusal");

    let mut foreign_fib = install_clean_fixture(FaultPoint::NONE);
    foreign_fib.invoke_real_installer_expect_typed_input_hook_failure();
    run_ip(&["route", "add", "blackhole", "203.0.113.0/24", "table", "100"]);
    run_ip(&["rule", "add", "priority", "123", "to", "203.0.113.0/24", "lookup", "main"]);
    let status = foreign_fib.stop_and_wait().expect("wait for fail-closed foreign-FIB audit");
    assert!(!status.success(), "foreign FIB state must refuse cleanup");
    let refused = PacketPathBaseline::capture_table(TABLE);
    assert!(refused.fib_routes.as_array().is_some_and(|routes| routes.len() == 3));
    assert!(refused.fib_rules.as_array().is_some_and(|rules| rules.len() == 5));
    manually_remove_test_owned_clean_delta();
    run_ip(&["route", "del", "blackhole", "203.0.113.0/24", "table", "100"]);
    run_ip(&["rule", "del", "priority", "123", "to", "203.0.113.0/24", "lookup", "main"]);
    assert_clean_fixture_baseline(&unrelated_fib, "manual teardown after foreign-FIB refusal");

    let panic_result = std::panic::catch_unwind(|| {
        let fixture = install_clean_fixture(FaultPoint::NONE);
        fixture.invoke_real_installer_expect_typed_input_hook_failure();
        let _fixture = fixture;
        panic!("intentional post-production assertion failure");
    });
    assert!(panic_result.is_err());
    assert_clean_fixture_baseline(&unrelated_fib, "post-production assertion/panic");

    let mut signalled = install_clean_fixture(FaultPoint::NONE);
    signalled.invoke_real_installer_expect_typed_input_hook_failure();
    let status = signalled.signal_and_wait(libc::SIGTERM);
    assert_eq!(status.code(), Some(128));
    assert_clean_fixture_baseline(&unrelated_fib, "post-production watchdog signal");

    let mut parent =
        Command::new("sleep").arg("60").spawn().expect("spawn disposable clean-fixture parent");
    let mut orphaned =
        DeltaScopedMalformedPrerouting::<ExactProductionRecovery>::try_install_production(
            parent.id(),
            FaultPoint::NONE,
        )
        .expect("install clean parent-death fixture");
    orphaned.invoke_real_installer_expect_typed_input_hook_failure();
    parent.kill().expect("kill disposable clean-fixture parent");
    parent.wait().expect("reap disposable clean-fixture parent");
    let status = orphaned.wait().expect("wait for clean parent-death reconciliation");
    assert!(status.success(), "clean parent-death reconciliation: {status}");
    assert_clean_fixture_baseline(&unrelated_fib, "post-production parent death");

    run_nft(&["add", "table", "ip", TABLE]);
    run_nft(&[
        "add",
        "chain",
        "ip",
        TABLE,
        PREROUTING,
        "{ type filter hook prerouting priority mangle; policy accept; }",
    ]);
    run_nft(&[
        "add",
        "chain",
        "ip",
        TABLE,
        OUTPUT,
        "{ type route hook output priority mangle; policy accept; }",
    ]);
    run_nft(&[
        "add",
        "rule",
        "ip",
        TABLE,
        PREROUTING,
        "counter",
        "comment",
        "baseline-prerouting-rule",
    ]);
    let existing_table = PacketPathBaseline::capture_table(TABLE);
    let existing_output_faults = EXISTING_CHAIN_CLEANUP_FAULTS
        .iter()
        .chain(OUTPUT_CLEANUP_FAULTS)
        .chain(FIB_CLEANUP_FAULTS)
        .copied()
        .collect::<Vec<_>>();
    exercise_clean_production_partition(
        &existing_table,
        "empty pre-existing output chain",
        &existing_output_faults,
    );
    run_nft(&["delete", "table", "ip", TABLE]);

    run_nft(&["add", "table", "ip", TABLE]);
    run_nft(&[
        "add",
        "chain",
        "ip",
        TABLE,
        PREROUTING,
        "{ type filter hook prerouting priority mangle; policy accept; }",
    ]);
    run_nft(&[
        "add",
        "rule",
        "ip",
        TABLE,
        PREROUTING,
        "counter",
        "comment",
        "baseline-prerouting-only-rule",
    ]);
    let output_absent = PacketPathBaseline::capture_table(TABLE);
    exercise_clean_production_partition(
        &output_absent,
        "pre-existing table without output",
        &existing_output_faults,
    );
    run_nft(&["delete", "table", "ip", TABLE]);

    run_nft(&["add", "table", "ip", TABLE]);
    let prerouting_absent = PacketPathBaseline::capture_table(TABLE);
    let created_chain_faults = CREATED_PREROUTING_CLEANUP_FAULTS
        .iter()
        .chain(OUTPUT_CLEANUP_FAULTS)
        .chain(FIB_CLEANUP_FAULTS)
        .copied()
        .collect::<Vec<_>>();
    exercise_clean_production_partition(
        &prerouting_absent,
        "pre-existing table without prerouting or output",
        &created_chain_faults,
    );
    run_nft(&["delete", "table", "ip", TABLE]);

    run_nft(&["add", "table", "ip", TABLE]);
    run_nft(&[
        "add",
        "chain",
        "ip",
        TABLE,
        OUTPUT,
        "{ type route hook output priority mangle; policy accept; }",
    ]);
    let prerouting_absent_output_empty = PacketPathBaseline::capture_table(TABLE);
    exercise_clean_production_partition(
        &prerouting_absent_output_empty,
        "pre-existing empty output without prerouting",
        &created_chain_faults,
    );
    run_nft(&["delete", "table", "ip", TABLE]);

    let fib_cleanup_faults =
        TABLE_CLEANUP_FAULTS.iter().chain(FIB_CLEANUP_FAULTS).copied().collect::<Vec<_>>();
    run_ip(&["rule", "add", "priority", "123", "fwmark", "0x1/0xff", "lookup", "100"]);
    let masked_rule = PacketPathBaseline::capture_table(TABLE);
    exercise_clean_production_partition(
        &masked_rule,
        "adopted masked fwmark/table rule",
        &fib_cleanup_faults,
    );
    run_ip(&["rule", "del", "priority", "123", "fwmark", "0x1/0xff", "lookup", "100"]);

    run_ip(&["route", "add", "local", "default", "dev", "lo", "table", "100", "protocol", "boot"]);
    let colliding_route = PacketPathBaseline::capture_table(TABLE);
    exercise_clean_production_partition(
        &colliding_route,
        "adopted structurally colliding local route",
        &fib_cleanup_faults,
    );
    run_ip(&["route", "del", "local", "default", "dev", "lo", "table", "100"]);

    run_ip(&["rule", "add", "priority", "123", "fwmark", "0x1/0xff", "lookup", "100"]);
    run_ip(&["route", "add", "local", "default", "dev", "lo", "table", "100", "protocol", "boot"]);
    let adopted_fib_pair = PacketPathBaseline::capture_table(TABLE);
    exercise_clean_production_partition(
        &adopted_fib_pair,
        "adopted FIB pair plus unrelated table-100 route",
        &fib_cleanup_faults,
    );
    run_ip(&["route", "del", "local", "default", "dev", "lo", "table", "100"]);
    run_ip(&["rule", "del", "priority", "123", "fwmark", "0x1/0xff", "lookup", "100"]);
    assert_clean_fixture_baseline(&unrelated_fib, "test-owned exact-partition teardown");

    let route = Command::new("ip")
        .args(["route", "del", "blackhole", "198.51.100.0/24", "table", "100"])
        .output()
        .expect("remove unrelated table-100 route");
    assert!(
        route.status.success(),
        "remove unrelated table-100 route: {}",
        String::from_utf8_lossy(&route.stderr)
    );
    assert_clean_fixture_baseline(&truly_absent, "test-owned unrelated-route teardown");
}

/// The unchanged production installer is exercised from a genuinely clean
/// nft/FIB baseline in a disposable native network namespace. Recovery accepts
/// only its exact un-commented output chain, tagged exemptions, and canonical
/// FIB objects, then preserves an unrelated table-100 route byte-for-byte.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[serial(cgroup)]
fn clean_network_namespace_real_installer_delta_is_exactly_reconciled() {
    match env::var(CLEAN_NETNS_MODE).as_deref() {
        Ok("audit") => audit_clean_production_delta_from_environment(),
        Ok("production") => run_clean_production_delta_scenarios(),
        Ok(other) => panic!("unknown clean network-namespace mode {other}"),
        Err(_) => {
            let status = Command::new("unshare")
                .arg("--net")
                .arg(env::current_exe().expect("locate this integration test executable"))
                .args(["--exact", CLEAN_NETNS_PRODUCTION_TEST, "--nocapture"])
                .env(CLEAN_NETNS_MODE, "production")
                .status()
                .expect("run real production clean network-namespace proof");
            assert!(status.success(), "clean production child failed: {status}");
        }
    }
}
