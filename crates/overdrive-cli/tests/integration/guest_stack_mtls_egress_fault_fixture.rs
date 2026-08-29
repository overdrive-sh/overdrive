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
const CLEAN_NETNS_BASELINE_TEST: &str = "integration::guest_stack_mtls_egress::fault_fixture::a_clean_network_namespace_captures_an_absent_table_100_route_baseline";
const CLEAN_NETNS_MODE: &str = "OVERDRIVE_GTI_CLEAN_NETNS_MODE";
const CLEAN_NETNS_PRODUCTION_TEST: &str = "integration::guest_stack_mtls_egress::fault_fixture::clean_network_namespace_real_installer_delta_is_exactly_reconciled";
const CLEAN_AUDIT_DIR: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_DIR";
const CLEAN_AUDIT_TABLE: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_TABLE";
const CLEAN_AUDIT_OWNER: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_OWNER";
const CLEAN_AUDIT_SAVED: &str = "OVERDRIVE_GTI_CLEAN_AUDIT_SAVED";
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

struct DeltaScopedMalformedPrerouting {
    baseline: PacketPathBaseline,
    table_name: String,
    watchdog: Option<Child>,
    stop_file: PathBuf,
    watchdog_dir: TempDir,
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
        Self::try_install_in_mode(table_name, parent, fault, None)
    }

    fn try_install_clean(parent: u32, fault: FaultPoint, test_name: &str) -> Result<Self, String> {
        Self::try_install_in_mode(TABLE, parent, fault, Some(test_name))
    }

    fn try_install_in_mode(
        table_name: &str,
        parent: u32,
        fault: FaultPoint,
        clean_audit_test: Option<&str>,
    ) -> Result<Self, String> {
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
        if let Some(handle) = baseline.nft_table.as_ref().map(table_handle) {
            std::fs::write(dir.join("baseline-table-handle"), handle.to_string())
                .map_err(|error| error.to_string())?;
        }
        if let Some(handle) =
            baseline.nft_table.as_ref().and_then(|table| chain_handle(table, PREROUTING))
        {
            std::fs::write(dir.join("baseline-prerouting-handle"), handle.to_string())
                .map_err(|error| error.to_string())?;
        }
        mark_if(
            dir,
            "had-fwmark-rule",
            baseline
                .fib_rules
                .as_array()
                .is_some_and(|rules| rules.iter().any(is_production_fwmark_rule)),
        )?;
        mark_if(
            dir,
            "had-local-route",
            baseline
                .fib_routes
                .as_array()
                .is_some_and(|routes| routes.iter().any(is_production_local_route)),
        )?;
        let (test_executable, test_name) = if let Some(test_name) = clean_audit_test {
            let audit_kind = if baseline.nft_table.is_some() {
                b"existing-table".as_slice()
            } else {
                b"absent-table".as_slice()
            };
            durable_write(dir, "allow-production-delta", audit_kind)?;
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

fn baseline_plus_optional_exact_delta(
    baseline: &Value,
    current: &Value,
    allowed_delta: fn(&Value) -> bool,
) -> bool {
    let (Some(baseline), Some(current)) = (baseline.as_array(), current.as_array()) else {
        return false;
    };
    let mut residual = current.clone();
    for expected in baseline {
        let Some(position) = residual.iter().position(|candidate| candidate == expected) else {
            return false;
        };
        residual.remove(position);
    }
    residual.is_empty() || (residual.len() == 1 && allowed_delta(&residual[0]))
}

fn clean_nft_delta_is_exact(snapshot: &PacketPathBaseline, table_name: &str, owner: &str) -> bool {
    let Some(document) = &snapshot.nft_table else {
        return snapshot.ownership.is_empty();
    };
    let Some(entries) = document.get("nftables").and_then(Value::as_array) else {
        return false;
    };
    let mut tables = Vec::new();
    let mut chains = BTreeMap::new();
    let mut rules = Vec::new();
    for entry in entries {
        if let Some(table) = entry.get("table") {
            tables.push(table);
        } else if let Some(chain) = entry.get("chain") {
            let Some(name) = chain.get("name").and_then(Value::as_str) else {
                return false;
            };
            if chains.insert(name.to_owned(), chain).is_some() {
                return false;
            }
        } else if let Some(rule) = entry.get("rule") {
            rules.push(rule);
        } else {
            return false;
        }
    }

    if tables.len() != 1
        || tables[0].get("family").and_then(Value::as_str) != Some("ip")
        || tables[0].get("name").and_then(Value::as_str) != Some(table_name)
        || tables[0].get("comment").and_then(Value::as_str) != Some(owner)
        || chains.len() > 2
    {
        return false;
    }
    if chains.is_empty() {
        return rules.is_empty() && snapshot.ownership.is_empty();
    }
    if !chains.contains_key(PREROUTING) {
        return false;
    }

    let chain_is = |chain: &Value, name: &str, kind: &str, hook: &str, comment: Option<&str>| {
        chain.get("family").and_then(Value::as_str) == Some("ip")
            && chain.get("table").and_then(Value::as_str) == Some(table_name)
            && chain.get("name").and_then(Value::as_str) == Some(name)
            && chain.get("type").and_then(Value::as_str) == Some(kind)
            && chain.get("hook").and_then(Value::as_str) == Some(hook)
            && chain.get("prio").and_then(Value::as_i64) == Some(-150)
            && chain.get("policy").and_then(Value::as_str) == Some("accept")
            && chain.get("comment").and_then(Value::as_str) == comment
    };
    if !chain_is(chains[PREROUTING], PREROUTING, "filter", "input", Some(owner)) {
        return false;
    }
    if let Some(output) = chains.get(OUTPUT)
        && !chain_is(output, OUTPUT, "route", "output", None)
    {
        return false;
    }

    let expected_tag = nft::userdata_exemption();
    let expected_program = canonical_exemption_program();
    let programs = rule_programs(document);
    let mut owned_rule_count = 0;
    for chain in [PREROUTING, OUTPUT] {
        let Some(chain_rules) = snapshot.ownership.get(chain) else {
            if chains.contains_key(chain) {
                return false;
            }
            continue;
        };
        if chain_rules.len() > 1
            || chain_rules.iter().any(|rule| {
                rule.userdata != expected_tag
                    || programs.get(&(chain.to_owned(), rule.handle)) != Some(&expected_program)
            })
        {
            return false;
        }
        owned_rule_count += chain_rules.len();
    }
    if snapshot.ownership.len() != chains.len() || rules.len() != owned_rule_count {
        return false;
    }
    if let Some(output_rules) = snapshot.ownership.get(OUTPUT) {
        let prerouting_rules = &snapshot.ownership[PREROUTING];
        if (!output_rules.is_empty() && prerouting_rules.len() != 1)
            || (chains.contains_key(OUTPUT) && prerouting_rules.len() != 1)
        {
            return false;
        }
    }
    rules.iter().all(|rule| {
        rule.get("family").and_then(Value::as_str) == Some("ip")
            && rule.get("table").and_then(Value::as_str) == Some(table_name)
            && matches!(rule.get("chain").and_then(Value::as_str), Some(PREROUTING | OUTPUT))
    })
}

fn existing_clean_nft_delta_is_exact(
    snapshot: &PacketPathBaseline,
    table_name: &str,
    owner: &str,
    saved_name: &str,
    expected_table_handle: u64,
    expected_prerouting_handle: u64,
) -> bool {
    let Some(document) = &snapshot.nft_table else {
        return false;
    };
    let Some(entries) = document.get("nftables").and_then(Value::as_array) else {
        return false;
    };
    let mut tables = Vec::new();
    let mut chains = BTreeMap::new();
    let mut rules = Vec::new();
    for entry in entries {
        if let Some(table) = entry.get("table") {
            tables.push(table);
        } else if let Some(chain) = entry.get("chain") {
            let Some(name) = chain.get("name").and_then(Value::as_str) else {
                return false;
            };
            if chains.insert(name.to_owned(), chain).is_some() {
                return false;
            }
        } else if let Some(rule) = entry.get("rule") {
            rules.push(rule);
        } else {
            return false;
        }
    }
    if tables.len() != 1
        || tables[0].get("family").and_then(Value::as_str) != Some("ip")
        || tables[0].get("name").and_then(Value::as_str) != Some(table_name)
        || tables[0].get("handle").and_then(Value::as_u64) != Some(expected_table_handle)
        || tables[0].get("comment").is_some()
        || chains.is_empty()
        || chains.len() > 3
        || chains
            .keys()
            .any(|name| name != PREROUTING && name != OUTPUT && name.as_str() != saved_name)
    {
        return false;
    }

    let chain_is = |chain: &Value,
                    name: &str,
                    kind: &str,
                    hook: &str,
                    handle: Option<u64>,
                    comment: Option<&str>| {
        chain.get("family").and_then(Value::as_str) == Some("ip")
            && chain.get("table").and_then(Value::as_str) == Some(table_name)
            && chain.get("name").and_then(Value::as_str) == Some(name)
            && chain.get("type").and_then(Value::as_str) == Some(kind)
            && chain.get("hook").and_then(Value::as_str) == Some(hook)
            && chain.get("prio").and_then(Value::as_i64) == Some(-150)
            && chain.get("policy").and_then(Value::as_str) == Some("accept")
            && handle.is_none_or(|expected| {
                chain.get("handle").and_then(Value::as_u64) == Some(expected)
            })
            && chain.get("comment").and_then(Value::as_str) == comment
    };

    let original_is_saved = chains.get(saved_name).is_some_and(|chain| {
        chain_is(chain, saved_name, "filter", "prerouting", Some(expected_prerouting_handle), None)
    });
    let original_is_restored = chains.get(PREROUTING).is_some_and(|chain| {
        chain_is(chain, PREROUTING, "filter", "prerouting", Some(expected_prerouting_handle), None)
    });
    if original_is_saved == original_is_restored {
        return false;
    }
    let fixture_prerouting = original_is_saved
        && chains
            .get(PREROUTING)
            .is_some_and(|chain| chain_is(chain, PREROUTING, "filter", "input", None, Some(owner)));
    if original_is_saved && chains.contains_key(PREROUTING) && !fixture_prerouting {
        return false;
    }
    if let Some(output) = chains.get(OUTPUT)
        && !chain_is(output, OUTPUT, "route", "output", None, None)
    {
        return false;
    }

    let expected_tag = nft::userdata_exemption();
    let expected_program = canonical_exemption_program();
    let programs = rule_programs(document);
    let mut owned_rule_count = 0;
    for (name, chain) in &chains {
        let Some(chain_rules) = snapshot.ownership.get(name) else {
            return false;
        };
        let may_hold_exemption = (name == PREROUTING && fixture_prerouting) || name == OUTPUT;
        if (!may_hold_exemption && !chain_rules.is_empty())
            || chain_rules.len() > 1
            || chain_rules.iter().any(|rule| {
                rule.userdata != expected_tag
                    || programs.get(&(name.clone(), rule.handle)) != Some(&expected_program)
            })
        {
            return false;
        }
        owned_rule_count += chain_rules.len();
        if chain.get("name").and_then(Value::as_str) != Some(name) {
            return false;
        }
    }
    if snapshot.ownership.len() != chains.len() || rules.len() != owned_rule_count {
        return false;
    }
    rules.iter().all(|rule| {
        rule.get("family").and_then(Value::as_str) == Some("ip")
            && rule.get("table").and_then(Value::as_str) == Some(table_name)
            && matches!(rule.get("chain").and_then(Value::as_str), Some(PREROUTING | OUTPUT))
    })
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
    let baseline_rules = read_json(&dir.join(CLEAN_BASELINE_RULES_FILE));
    let baseline_routes = read_json(&dir.join(CLEAN_BASELINE_ROUTES_FILE));
    let current = PacketPathBaseline::capture_table(&table_name);
    let audit_kind =
        std::fs::read_to_string(dir.join("allow-production-delta")).expect("read clean audit kind");
    let nft_delta_is_exact = match audit_kind.as_str() {
        "absent-table" => clean_nft_delta_is_exact(&current, &table_name, &owner),
        "existing-table" => {
            let expected_table_handle = std::fs::read_to_string(dir.join("baseline-table-handle"))
                .expect("read baseline table handle")
                .parse()
                .expect("parse baseline table handle");
            let expected_prerouting_handle =
                std::fs::read_to_string(dir.join("baseline-prerouting-handle"))
                    .expect("read baseline prerouting handle")
                    .parse()
                    .expect("parse baseline prerouting handle");
            existing_clean_nft_delta_is_exact(
                &current,
                &table_name,
                &owner,
                &saved_name,
                expected_table_handle,
                expected_prerouting_handle,
            )
        }
        other => panic!("unknown clean audit kind {other}"),
    };
    assert!(
        nft_delta_is_exact,
        "refuse cleanup of a foreign or noncanonical nft delta: {current:#?}"
    );
    assert!(
        baseline_plus_optional_exact_delta(
            &baseline_rules,
            &current.fib_rules,
            is_production_fwmark_rule,
        ),
        "refuse cleanup of foreign FIB-rule state"
    );
    assert!(
        baseline_plus_optional_exact_delta(
            &baseline_routes,
            &current.fib_routes,
            is_production_local_route,
        ),
        "refuse cleanup of foreign table-100 route state: baseline={baseline_routes:#?} current={:#?}",
        current.fib_routes
    );
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

clean_delta_is_exact() {
  [ -n "$test_executable" ] && [ -n "$test_name" ] || return 1
  OVERDRIVE_GTI_CLEAN_NETNS_MODE=audit \
    OVERDRIVE_GTI_CLEAN_AUDIT_DIR="$dir" \
    OVERDRIVE_GTI_CLEAN_AUDIT_TABLE="$table" \
    OVERDRIVE_GTI_CLEAN_AUDIT_OWNER="$owner" \
    OVERDRIVE_GTI_CLEAN_AUDIT_SAVED="$saved" \
    "$test_executable" --exact "$test_name" --nocapture >/dev/null 2>&1
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
      table_owned || return 89
      if [ ! -f "$dir/allow-production-delta" ]; then
        owned_table_is_disposable || return 89
      fi
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
      if [ ! -f "$dir/allow-production-delta" ]; then
        chain_owned output || return 92
      fi
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

fn call_real_installer_expect_typed_input_hook_failure() {
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

fn assert_typed_input_hook_failure() {
    let fixture = DeltaScopedMalformedPrerouting::install();
    call_real_installer_expect_typed_input_hook_failure();
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

fn install_clean_fixture(fault: FaultPoint) -> DeltaScopedMalformedPrerouting {
    DeltaScopedMalformedPrerouting::try_install_clean(
        std::process::id(),
        fault,
        CLEAN_NETNS_PRODUCTION_TEST,
    )
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

fn run_clean_production_delta_scenarios() {
    let truly_absent = PacketPathBaseline::capture_table(TABLE);
    assert!(truly_absent.nft_table.is_none());
    assert_eq!(truly_absent.fib_routes, json!([]));
    let fixture = install_clean_fixture(FaultPoint::NONE);
    fixture
        .journal_production_intents(None)
        .expect("durably journal every possible production mutation");
    call_real_installer_expect_typed_input_hook_failure();
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
        let result = DeltaScopedMalformedPrerouting::try_install_clean(
            std::process::id(),
            FaultPoint(fault),
            CLEAN_NETNS_PRODUCTION_TEST,
        );
        assert!(result.is_err(), "{fault} is observable");
        assert_clean_fixture_baseline(&unrelated_fib, fault);
    }

    for fault in TABLE_CLEANUP_FAULTS.iter().chain(FIB_CLEANUP_FAULTS) {
        let fixture = install_clean_fixture(FaultPoint(fault));
        fixture
            .journal_production_intents(None)
            .unwrap_or_else(|error| panic!("journal production intents for {fault}: {error}"));
        call_real_installer_expect_typed_input_hook_failure();
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
    foreign_nft
        .journal_production_intents(None)
        .expect("journal production mutations before foreign-nft probe");
    call_real_installer_expect_typed_input_hook_failure();
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
    foreign_fib
        .journal_production_intents(None)
        .expect("journal production mutations before foreign-FIB probe");
    call_real_installer_expect_typed_input_hook_failure();
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
        fixture
            .journal_production_intents(None)
            .expect("journal production mutations before assertion path");
        call_real_installer_expect_typed_input_hook_failure();
        let _fixture = fixture;
        panic!("intentional post-production assertion failure");
    });
    assert!(panic_result.is_err());
    assert_clean_fixture_baseline(&unrelated_fib, "post-production assertion/panic");

    let mut signalled = install_clean_fixture(FaultPoint::NONE);
    signalled
        .journal_production_intents(None)
        .expect("journal production mutations before watchdog signal");
    call_real_installer_expect_typed_input_hook_failure();
    let status = signalled.signal_and_wait(libc::SIGTERM);
    assert_eq!(status.code(), Some(128));
    assert_clean_fixture_baseline(&unrelated_fib, "post-production watchdog signal");

    let mut parent =
        Command::new("sleep").arg("60").spawn().expect("spawn disposable clean-fixture parent");
    let mut orphaned = DeltaScopedMalformedPrerouting::try_install_clean(
        parent.id(),
        FaultPoint::NONE,
        CLEAN_NETNS_PRODUCTION_TEST,
    )
    .expect("install clean parent-death fixture");
    orphaned
        .journal_production_intents(None)
        .expect("journal production mutations before parent death");
    call_real_installer_expect_typed_input_hook_failure();
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
    let existing_table = PacketPathBaseline::capture_table(TABLE);
    let fixture = install_clean_fixture(FaultPoint::NONE);
    fixture
        .journal_production_intents(None)
        .expect("journal production mutations for existing-table normal path");
    call_real_installer_expect_typed_input_hook_failure();
    fixture.finish();
    assert_clean_fixture_baseline(&existing_table, "existing table without output normal path");

    for fault in
        EXISTING_CHAIN_CLEANUP_FAULTS.iter().chain(OUTPUT_CLEANUP_FAULTS).chain(FIB_CLEANUP_FAULTS)
    {
        let fixture = install_clean_fixture(FaultPoint(fault));
        fixture.journal_production_intents(None).unwrap_or_else(|error| {
            panic!("journal existing-table production for {fault}: {error}")
        });
        call_real_installer_expect_typed_input_hook_failure();
        fixture.finish();
        assert_clean_fixture_baseline(&existing_table, fault);
    }
    run_nft(&["delete", "table", "ip", TABLE]);
    assert_clean_fixture_baseline(&unrelated_fib, "test-owned existing-table teardown");

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
