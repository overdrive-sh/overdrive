//! `prefix_landing` real-kernel conformance harness (netns-density-295,
//! QPROJ-8).
//!
//! `specs/quint/guest-flow-owner/prefix_landing.qnt` is stateless: its
//! `lands(net, client, port)` is the oracle and the kernel is the arbiter.
//! This module supplies the three halves the scenario compares, each from an
//! independent source (feature-delta § *Conformance observation surface and
//! action map*, QPROJ-1 "Sources of `State`" (c) and (d)):
//!
//! - **Inputs (`Net`)** are read by kernel queries and never written:
//!   `ip route show table local` (`route`), `bpftool link` on the steering
//!   pin directory (`steer`), `bpftool map dump` of that link's socket map
//!   (`entries`), `ss -ltn` on the workload address (`lis`),
//!   `nft list table ip overdrive-mtls` (`divert`, `reject`, leg-C's port) and
//!   `ss -ltn` on leg-C's port (`legC`). `marked` is the caller's own fact:
//!   whether the `serve` composition it booted is serving.
//! - **Observed landing** comes from a real connect by a client of each class
//!   and the first bytes the accepting socket writes: the guest application
//!   and the wildcard sentinel each write a distinct banner, leg-C answers the
//!   probe's TLS `ClientHello` with a TLS record, a refusal is a reset /
//!   `ECONNREFUSED` (or, for a remote client only, a connect timeout), and
//!   `elsewhere` is a non-`local` `ip route get` with no local acceptor.
//! - **Model landing** is `lands(..)` evaluated by the `quint` REPL against the
//!   spec file in place (never copied; `.claude/rules/testing.md` § "Quint
//!   conformance", "One spec, one oracle"). quint-connect replays traces of
//!   stateful modules only and has no definition-evaluation surface, so the
//!   oracle is the `quint` CLI itself.
//!
//! The configurations are reached only through production paths: an in-process
//! `run_server` with the production adapters (`HostMtlsIntercept`,
//! `HostGuestDnsFactory`, `RealCgroupFs`), workloads deployed through the
//! HTTPS API, `ServerHandle::kill_for_test` for the crashed `serve`, and
//! `ServerHandle::shutdown` for the graceful stop. No route, rule, steering
//! entry or listener is installed by this module; the only kernel writes it
//! makes are its own client fixtures (the wildcard sentinels, the remote
//! client's namespace and veth) and the never-booted precondition / cleanup
//! complement in [`clear_node_guest_prefix_state`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use ipnet::Ipv4Net;
use nix::sched::{CloneFlags, setns};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, setsockopt, socket, sockopt};
use overdrive_control_plane::dns_responder::HostGuestDnsFactory;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::dataplane::{MTLS_LEG_S_DIAL_MARK, MTLS_SHARED_NFT_TABLE};
use overdrive_core::guest_transport::{
    GUEST_INTAKE_HOST_PORT_BASE, GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX, GUEST_NODE_PREFIX,
    GUEST_PREFIX_ROUTE_PROTOCOL,
};
use overdrive_core::traits::cgroup_fs::CgroupFs;
use overdrive_dataplane::guest_steering::{
    GUEST_PREFIX_STEERING_PIN_DIR, GUEST_STEERING_KEY_LEN, guest_steering_key,
};
use overdrive_host::RealCgroupFs;
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_testing::netns::NetNs;
use overdrive_testing::vm_fixture::VmFixture;
use overdrive_worker::mtls_intercept_port::HostMtlsIntercept;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpSocket, TcpStream};

use crate::{PublicApi, command_output};

/// The spec, referenced in place.
pub const SPEC: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../specs/quint/guest-flow-owner/prefix_landing.qnt");

/// Offset of [`UNLEASED_PREFIX_ADDRESS`] above the guest prefix's network
/// address: high in the prefix's host range, far above the low end the pool
/// assigns from.
const UNLEASED_PREFIX_HOST_OFFSET: u32 = 0xF0F0;

// The offset names a host address inside `GUEST_NODE_PREFIX` (and is not its
// broadcast address).
const _: () = assert!(
    UNLEASED_PREFIX_HOST_OFFSET < (1_u32 << (32 - GUEST_NODE_PREFIX.prefix_len())) - 1,
    "the unleased address must lie inside the guest prefix",
);

/// An address inside the node guest prefix (`GUEST_NODE_PREFIX`) that no
/// allocation is leased in.
///
/// The pool assigns from the low end, so no configuration this scenario
/// reaches leases it. It is the prefix's network address plus
/// [`UNLEASED_PREFIX_HOST_OFFSET`].
pub const UNLEASED_PREFIX_ADDRESS: Ipv4Addr = Ipv4Addr::from_bits(
    (GUEST_NODE_PREFIX.addr().to_bits() & (u32::MAX << (32 - GUEST_NODE_PREFIX.prefix_len())))
        + UNLEASED_PREFIX_HOST_OFFSET,
);

/// The node-global nft table that holds the constant guest-prefix rules
/// (family `ip`, `MTLS_SHARED_NFT_TABLE`, DG-9).
pub const SHARED_NFT_TABLE: (&str, &str) = ("ip", MTLS_SHARED_NFT_TABLE);

/// Banner the guest application writes to every connection it accepts.
pub const INTAKE_BANNER: &[u8] = b"OVERDRIVE-LANDING-INTAKE\n";
/// Banner the wildcard host sentinel writes to every connection it accepts.
pub const WILDCARD_BANNER: &[u8] = b"OVERDRIVE-LANDING-WILDCARD\n";

const CONNECT_BOUND: Duration = Duration::from_secs(2);
const FIRST_BYTES_BOUND: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Model vocabulary
// ---------------------------------------------------------------------------

/// The spec's client classes (`prefix_landing.CLIENTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Client {
    /// Overdrive's own marked clients (the leg-S mark).
    Marked,
    /// Any other host-local client.
    Unmarked,
    /// A packet arriving on a non-loopback device.
    Remote,
}

impl Client {
    pub const ALL: [Self; 3] = [Self::Marked, Self::Unmarked, Self::Remote];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Marked => "marked",
            Self::Unmarked => "unmarked",
            Self::Remote => "remote",
        }
    }
}

/// The spec's landing outcomes, plus the failure class the oracle never
/// produces: a landing the observation could not place in any outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    Intake,
    LegC,
    Refused,
    Wildcard,
    Elsewhere,
    Unclassified(String),
}

impl Landing {
    fn from_model(value: &str) -> Self {
        match value {
            "intake" => Self::Intake,
            "legC" => Self::LegC,
            "refused" => Self::Refused,
            "wildcard" => Self::Wildcard,
            "elsewhere" => Self::Elsewhere,
            other => Self::Unclassified(format!("model returned {other:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteForm {
    Absent,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Steer {
    None,
    Ours,
}

/// `prefix_landing.Net` for one destination address, read from the kernel.
///
/// `assignRemote` is the model's hazard hook and is always `false` here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "mirrors the spec's `Net` record field for field")]
pub struct Net {
    pub marked: bool,
    pub leg_c: bool,
    pub divert: bool,
    pub reject: bool,
    pub route: RouteForm,
    pub steer: Steer,
    pub entries: BTreeSet<u16>,
    pub lis: BTreeSet<u16>,
}

impl Net {
    fn to_quint(&self) -> String {
        let set = |ports: &BTreeSet<u16>| {
            let items: Vec<String> = ports.iter().map(u16::to_string).collect();
            format!("Set({})", items.join(", "))
        };
        format!(
            "{{ marked: {}, legC: {}, divert: {}, reject: {}, route: \"{}\", steer: \"{}\", \
             entries: {}, lis: {}, assignRemote: false }}",
            self.marked,
            self.leg_c,
            self.divert,
            self.reject,
            match self.route {
                RouteForm::Absent => "absent",
                RouteForm::Local => "local",
            },
            match self.steer {
                Steer::None => "none",
                Steer::Ours => "ours",
            },
            set(&self.entries),
            set(&self.lis),
        )
    }
}

/// One `lands(net, client, port)` evaluation.
#[derive(Debug, Clone)]
pub struct LandingQuery {
    pub net: Net,
    pub client: Client,
    pub port: u16,
}

/// Evaluate `lands(..)` for every query in one `quint` REPL session.
///
/// The spec is read in place. Panics when `quint` is absent or answers a different number
/// of lines than it was asked: the oracle is a precondition, never skipped.
#[must_use]
pub fn model_landings(queries: &[LandingQuery]) -> Vec<Landing> {
    let mut child = Command::new("quint")
        .args(["repl", "--backend", "typescript", "-q", "-r"])
        .arg(format!("{SPEC}::prefix_landing"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn the quint REPL for the landing oracle: {error}"));
    {
        let mut stdin = child.stdin.take().expect("quint REPL stdin is piped");
        for query in queries {
            writeln!(
                stdin,
                "lands({}, \"{}\", {})",
                query.net.to_quint(),
                query.client.as_str(),
                query.port
            )
            .expect("write a lands() query to the quint REPL");
        }
    }
    let output = child.wait_with_output().expect("wait for the quint REPL");
    assert!(
        output.status.success(),
        "quint REPL failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answers: Vec<Landing> = BufReader::new(output.stdout.as_slice())
        .lines()
        .map(|line| line.expect("read a quint REPL answer"))
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: String = serde_json::from_str(line.trim())
                .unwrap_or_else(|error| panic!("quint answer {line:?} is not a string: {error}"));
            Landing::from_model(&value)
        })
        .collect();
    assert_eq!(
        answers.len(),
        queries.len(),
        "the quint REPL answered {} of {} lands() queries: {}",
        answers.len(),
        queries.len(),
        String::from_utf8_lossy(&output.stderr)
    );
    answers
}

// ---------------------------------------------------------------------------
// Inputs: independent kernel queries
// ---------------------------------------------------------------------------

fn json_command(program: &str, args: &[&str]) -> Option<Value> {
    let output = command_output(program, args);
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if text.trim().is_empty() {
        return Some(Value::Array(Vec::new()));
    }
    Some(serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!("{program} {args:?} printed invalid JSON ({error}): {text}")
    }))
}

/// `route`: a `local` route on `lo` in the local table, tagged with
/// `GUEST_PREFIX_ROUTE_PROTOCOL` (G-PL-2, DG-14), whose prefix (shorter than
/// /32, so not a host address of an interface) contains `dst`.
#[must_use]
pub fn read_route(dst: Ipv4Addr) -> RouteForm {
    let protocol = GUEST_PREFIX_ROUTE_PROTOCOL.to_string();
    let routes = json_command(
        "ip",
        &["-j", "route", "show", "table", "local", "type", "local", "proto", &protocol],
    )
    .expect("ip route show table local proto <guest-prefix> succeeds");
    let local = routes.as_array().into_iter().flatten().any(|route| {
        let Some(prefix) = route["dst"].as_str() else { return false };
        let Ok(net) = prefix.parse::<Ipv4Net>() else { return false };
        route["dev"].as_str() == Some("lo") && net.prefix_len() < 32 && net.contains(&dst)
    });
    if local { RouteForm::Local } else { RouteForm::Absent }
}

/// The constant guest-prefix rules of the shared table.
///
/// The facts the model reads: a divert to leg-C (with leg-C's TPROXY port), and a reject
/// or drop. A rule applies to `dst` when one of its matches is an IPv4
/// destination match (address or prefix) containing `dst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrefixRules {
    pub divert_port: Option<u16>,
    pub reject: bool,
}

#[must_use]
pub fn read_prefix_rules(dst: Ipv4Addr) -> PrefixRules {
    let (family, table) = SHARED_NFT_TABLE;
    let Some(listing) = json_command("nft", &["-j", "list", "table", family, table]) else {
        return PrefixRules::default();
    };
    let mut rules = PrefixRules::default();
    for item in listing["nftables"].as_array().into_iter().flatten() {
        let Some(exprs) = item["rule"]["expr"].as_array() else { continue };
        let applies = exprs.iter().any(|expr| daddr_match_contains(&expr["match"], dst));
        if !applies {
            continue;
        }
        for expr in exprs {
            if let Some(port) = expr["tproxy"]["port"].as_u64() {
                rules.divert_port = u16::try_from(port).ok();
            }
            if expr.get("reject").is_some() || expr.get("drop").is_some() {
                rules.reject = true;
            }
        }
    }
    rules
}

fn daddr_match_contains(matched: &Value, dst: Ipv4Addr) -> bool {
    if matched["left"]["payload"]["field"].as_str() != Some("daddr") {
        return false;
    }
    let right = &matched["right"];
    if let (Some(addr), Some(len)) =
        (right["prefix"]["addr"].as_str(), right["prefix"]["len"].as_u64())
    {
        return addr
            .parse::<Ipv4Addr>()
            .ok()
            .and_then(|addr| Ipv4Net::new(addr, u8::try_from(len).ok()?).ok())
            .is_some_and(|net| net.contains(&dst));
    }
    right.as_str().and_then(|addr| addr.parse::<Ipv4Addr>().ok()) == Some(dst)
}

/// TCP listening sockets bound to `addr` (or any address when `None`), as
/// `(local address, port)`, from `ss -Hltn`.
fn listening_sockets(filter: &str) -> Vec<SocketAddrV4> {
    let output = command_output("ss", &["-Hltn", filter]);
    assert!(output.status.success(), "ss -Hltn {filter} failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(3))
        .filter_map(|local| {
            let local = local.replace('*', "0.0.0.0");
            local.parse::<SocketAddrV4>().ok()
        })
        .collect()
}

/// `legC`: a listener exists on leg-C's TPROXY port.
#[must_use]
pub fn read_leg_c(divert_port: Option<u16>) -> bool {
    divert_port.is_some_and(|port| !listening_sockets(&format!("sport = :{port}")).is_empty())
}

/// `lis`: declared ports with an open intake listener on `dst`.
///
/// An intake listener binds `dst:(GUEST_INTAKE_HOST_PORT_BASE + i)` for the declared
/// port at index `i` of the allocation's ascending distinct declared TCP
/// ports (INTAKE-BIND; `guest_intake_host_port`).
#[must_use]
pub fn read_intake_listeners(dst: Ipv4Addr, declared: &BTreeSet<u16>) -> BTreeSet<u16> {
    let ordered: Vec<u16> = declared.iter().copied().collect();
    listening_sockets(&format!("src {dst}"))
        .into_iter()
        .filter(|socket| *socket.ip() == dst)
        .filter_map(|socket| {
            let index = socket.port().checked_sub(GUEST_INTAKE_HOST_PORT_BASE)?;
            (index < GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX)
                .then(|| ordered.get(usize::from(index)).copied())
                .flatten()
        })
        .collect()
}

/// `steer` and the steering link's program, from the pins under the steering
/// pin directory.
#[must_use]
pub fn read_steering_program() -> Option<u64> {
    let entries = std::fs::read_dir(GUEST_PREFIX_STEERING_PIN_DIR).ok()?;
    entries.filter_map(Result::ok).find_map(|entry| {
        let path = entry.path();
        let path = path.to_str()?;
        let link = json_command("bpftool", &["-j", "link", "show", "pinned", path])?;
        (link["type"].as_str() == Some("sk_lookup")).then(|| link["prog_id"].as_u64()).flatten()
    })
}

/// `entries`: declared ports with a steering entry for `dst`, from a dump of
/// the steering program's socket map.
///
/// The key layout is the G-PL-1 contract (DG-14): a key is an entry for
/// `(dst, port)` exactly when it equals `guest_steering_key((dst, port))`, the
/// one encoder the steering program also uses. A key of another length, or
/// one the encoder does not reproduce, names no intake.
#[must_use]
pub fn read_steering_entries(program: u64, dst: Ipv4Addr) -> BTreeSet<u16> {
    let program = json_command("bpftool", &["-j", "prog", "show", "id", &program.to_string()])
        .expect("bpftool prog show of the steering program");
    let mut entries = BTreeSet::new();
    for map in program["map_ids"].as_array().into_iter().flatten() {
        let Some(map) = map.as_u64().map(|id| id.to_string()) else { continue };
        let shown = json_command("bpftool", &["-j", "map", "show", "id", &map])
            .expect("bpftool map show of a steering-program map");
        if !matches!(shown["type"].as_str(), Some("sockmap" | "sockhash")) {
            continue;
        }
        let dump = json_command("bpftool", &["-j", "map", "dump", "id", &map])
            .expect("bpftool map dump of the steering socket map");
        for element in dump.as_array().into_iter().flatten() {
            let key: Vec<u8> = element["key"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|byte| u8::from_str_radix(byte.trim_start_matches("0x"), 16).ok())
                .collect();
            if key.len() != GUEST_STEERING_KEY_LEN {
                continue;
            }
            let port = u16::from_be_bytes([key[4], key[5]]);
            if key[..] == guest_steering_key(SocketAddrV4::new(dst, port))[..] {
                entries.insert(port);
            }
        }
    }
    entries
}

/// Read `Net` for `dst` from the kernel. `marked` is the caller's fact.
#[must_use]
pub fn read_net(marked: bool, dst: Ipv4Addr, declared: &BTreeSet<u16>) -> Net {
    let rules = read_prefix_rules(dst);
    let program = read_steering_program();
    Net {
        marked,
        leg_c: read_leg_c(rules.divert_port),
        divert: rules.divert_port.is_some(),
        reject: rules.reject,
        route: read_route(dst),
        steer: if program.is_some() { Steer::Ours } else { Steer::None },
        entries: program.map(|program| read_steering_entries(program, dst)).unwrap_or_default(),
        lis: read_intake_listeners(dst, declared),
    }
}

/// `ip route get dst` resolves through a `local` route.
#[must_use]
pub fn routes_locally(dst: Ipv4Addr) -> bool {
    json_command("ip", &["-j", "route", "get", &dst.to_string()])
        .and_then(|routes| routes.as_array().and_then(|routes| routes.first().cloned()))
        .is_some_and(|route| route["type"].as_str() == Some("local"))
}

// ---------------------------------------------------------------------------
// Never-booted precondition and cleanup complement
// ---------------------------------------------------------------------------

/// What one pass of [`clear_node_guest_prefix_state`] removed or failed to remove.
///
/// Failures are recorded, never raised mid-cleanup, so the
/// scenario's comparison record survives a cleanup failure.
#[derive(Debug, Default)]
pub struct NodeStateClearing {
    pub removed: Vec<String>,
    pub failures: Vec<String>,
}

/// Remove the node-global guest-prefix state a `serve` leaves behind.
///
/// U-4: production never removes the `local` prefix route, the steering pins
/// or the shared nft table. Run before the first boot, it reaches the kernel
/// state of a node that never ran `serve`; run after the last shutdown, it is
/// the exact complement of this scenario's boots.
#[must_use]
pub fn clear_node_guest_prefix_state() -> NodeStateClearing {
    let mut outcome = NodeStateClearing::default();
    if read_route(UNLEASED_PREFIX_ADDRESS) == RouteForm::Local {
        // overdrive-netlink has no typed delete for a prefix `local` route.
        let prefix = GUEST_NODE_PREFIX.to_string();
        let output = command_output(
            "ip",
            &["route", "del", "table", "local", "local", &prefix, "dev", "lo"],
        );
        if output.status.success() {
            outcome.removed.push(format!("route local {prefix} dev lo"));
        } else {
            outcome.failures.push(format!(
                "remove route local {prefix}: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    if let Ok(pins) = std::fs::read_dir(GUEST_PREFIX_STEERING_PIN_DIR) {
        for pin in pins {
            match pin.map(|pin| pin.path()) {
                Ok(pin) => match std::fs::remove_file(&pin) {
                    Ok(()) => outcome.removed.push(format!("pin {}", pin.display())),
                    Err(error) => {
                        outcome.failures.push(format!("unpin {}: {error}", pin.display()));
                    }
                },
                Err(error) => outcome.failures.push(format!("read a steering pin: {error}")),
            }
        }
        if let Err(error) = std::fs::remove_dir(GUEST_PREFIX_STEERING_PIN_DIR) {
            outcome.failures.push(format!("remove {GUEST_PREFIX_STEERING_PIN_DIR}: {error}"));
        }
    }
    let (family, table) = SHARED_NFT_TABLE;
    if command_output("nft", &["list", "table", family, table]).status.success() {
        match overdrive_netlink::nft::delete_table(table) {
            Ok(()) => outcome.removed.push(format!("nft table {family} {table}")),
            Err(error) => {
                outcome.failures.push(format!("delete nft table {family} {table}: {error}"));
            }
        }
    }
    outcome
}

// ---------------------------------------------------------------------------
// Clients and observed landing
// ---------------------------------------------------------------------------

/// A host service bound on `0.0.0.0:port` before boot (R5-1's trap): it writes
/// [`WILDCARD_BANNER`] to every connection it accepts.
pub struct WildcardSentinel {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WildcardSentinel {
    #[must_use]
    pub fn bind(port: u16) -> Self {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port))
            .unwrap_or_else(|error| {
                panic!("bind the wildcard sentinel on 0.0.0.0:{port}: {error}")
            });
        listener.set_nonblocking(true).expect("non-blocking wildcard sentinel");
        let stop = Arc::new(AtomicBool::new(false));
        let observed_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !observed_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // A peer that resets first is not this sentinel's failure.
                        if let Err(error) = stream.write_all(WILDCARD_BANNER) {
                            tracing::debug!(%error, "wildcard sentinel write failed");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => panic!("wildcard sentinel accept failed: {error}"),
                }
            }
        });
        Self { port, stop, thread: Some(thread) }
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for WildcardSentinel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("wildcard sentinel thread exits");
        }
    }
}

/// The remote client: a namespace joined to the host by a veth pair, routed to
/// the guest prefix through the host side, so its packets arrive on a
/// non-loopback device (K-L5).
pub struct RemoteClient {
    netns: NetNs,
    lease: TestCidrLease,
    host_if: String,
}

impl RemoteClient {
    const NETNS: &'static str = "ovd-landing-remote";
    const HOST_IF: &'static str = "ovdlnd-h";
    const PEER_IF: &'static str = "ovdlnd-n";

    #[must_use]
    pub fn create() -> Self {
        let lease = TestCidrLease::acquire("prefix-landing-remote")
            .expect("lease a test CIDR for the remote client");
        let netns = NetNs::create(Self::NETNS).expect("create the remote client's namespace");
        let host_gateway = lease.host_gateway();
        let prefix = lease.prefix_len();
        overdrive_netlink::block_on_host_netlink(|| async move {
            let client = overdrive_netlink::Client::new()?;
            client.del_link(Self::HOST_IF).await?;
            client.add_veth_pair(Self::HOST_IF, Self::PEER_IF).await?;
            client.move_link_to_netns(Self::PEER_IF, Self::NETNS).await?;
            client.add_addr(Self::HOST_IF, host_gateway, prefix).await?;
            client.set_link_up(Self::HOST_IF).await
        })
        .expect("create the remote client's veth pair");
        netns
            .assign_ip_and_up(Self::PEER_IF, &format!("{}/{prefix}", lease.workload_addr()))
            .expect("address the remote client's veth end");
        netns
            .add_route(&GUEST_NODE_PREFIX.to_string(), Some(&host_gateway.to_string()), None)
            .expect("route the guest prefix through the host");
        Self { netns, lease, host_if: Self::HOST_IF.to_owned() }
    }

    fn socket_in_namespace(&self) -> OwnedFd {
        let path = format!("/var/run/netns/{}", self.netns.name);
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let namespace = File::open(&path).expect("open the remote client's namespace");
                    setns(namespace.as_fd(), CloneFlags::CLONE_NEWNET)
                        .expect("enter the remote client's namespace");
                    tcp_socket()
                })
                .join()
                .expect("remote socket thread")
        })
    }

    /// The remote client's own address, as the host sees the peer.
    #[must_use]
    pub const fn address(&self) -> Ipv4Addr {
        self.lease.workload_addr()
    }
}

impl Drop for RemoteClient {
    fn drop(&mut self) {
        let host_if = self.host_if.clone();
        let removed = overdrive_netlink::block_on_host_netlink(|| async move {
            overdrive_netlink::Client::new()?.del_link(&host_if).await
        });
        if let Err(error) = removed {
            // Teardown keeps the first failure visible without masking it.
            tracing::warn!(%error, iface = %self.host_if, "remote client cleanup: deleting the veth failed");
        }
    }
}

fn tcp_socket() -> OwnedFd {
    socket(
        AddressFamily::Inet,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        None,
    )
    .expect("create a TCP socket")
}

fn client_socket(client: Client, remote: &RemoteClient) -> TcpSocket {
    let fd = match client {
        Client::Marked => {
            let fd = tcp_socket();
            setsockopt(&fd, sockopt::Mark, &MTLS_LEG_S_DIAL_MARK)
                .expect("stamp the leg-S mark on the marked client");
            fd
        }
        Client::Unmarked => tcp_socket(),
        Client::Remote => remote.socket_in_namespace(),
    };
    TcpSocket::from_std_stream(std::net::TcpStream::from(fd))
}

/// A TLS `ClientHello` for leg-C to answer: leg-C is a TLS server, so the
/// first bytes it writes are a TLS record (handshake or alert), while the
/// intake application and the wildcard sentinel write their banners.
fn client_hello() -> Vec<u8> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("rustls protocol versions")
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("landing.probe.invalid")
        .expect("probe server name")
        .to_owned();
    let mut connection =
        rustls::ClientConnection::new(Arc::new(config), name).expect("probe TLS client");
    let mut hello = Vec::new();
    connection.write_tls(&mut hello).expect("encode the probe ClientHello");
    hello
}

/// Observe where a connection from `client` to `dst` lands.
pub async fn observe_landing(client: Client, dst: SocketAddrV4, remote: &RemoteClient) -> Landing {
    let socket = client_socket(client, remote);
    let connected = tokio::time::timeout(CONNECT_BOUND, socket.connect(SocketAddr::V4(dst))).await;
    match connected {
        Ok(Ok(stream)) => classify_accepted(stream).await,
        // Not accepted, and the address does not resolve through a `local`
        // route: the connection left the node.
        _ if !routes_locally(*dst.ip()) => Landing::Elsewhere,
        Ok(Err(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            Landing::Refused
        }
        Err(_elapsed) if client == Client::Remote => Landing::Refused,
        Err(_elapsed) => Landing::Unclassified(format!(
            "a host-local connect to {dst} timed out after {CONNECT_BOUND:?} through a local route"
        )),
        Ok(Err(error)) => Landing::Unclassified(format!("connect to {dst} failed: {error}")),
    }
}

async fn classify_accepted(mut stream: TcpStream) -> Landing {
    if let Err(error) = stream.write_all(&client_hello()).await {
        return Landing::Unclassified(format!("write the probe ClientHello: {error}"));
    }
    let mut first = vec![0_u8; WILDCARD_BANNER.len().max(INTAKE_BANNER.len())];
    let mut filled = 0;
    let read = tokio::time::timeout(FIRST_BYTES_BOUND, async {
        while filled < first.len() {
            match stream.read(&mut first[filled..]).await {
                Ok(0) => break,
                Ok(n) => {
                    filled += n;
                    if first[..filled].starts_with(INTAKE_BANNER)
                        || first[..filled].starts_with(WILDCARD_BANNER)
                        || (filled >= 2 && matches!(first[0], 0x15 | 0x16) && first[1] == 0x03)
                    {
                        break;
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    })
    .await;
    let first = &first[..filled];
    if first.starts_with(INTAKE_BANNER) {
        Landing::Intake
    } else if first.starts_with(WILDCARD_BANNER) {
        Landing::Wildcard
    } else if first.len() >= 2 && matches!(first[0], 0x15 | 0x16) && first[1] == 0x03 {
        Landing::LegC
    } else {
        Landing::Unclassified(format!(
            "an accepted connection wrote {first:02x?} (read outcome {read:?}); no known acceptor"
        ))
    }
}

// ---------------------------------------------------------------------------
// Guest application and production node
// ---------------------------------------------------------------------------

/// Build the static guest application and stage a rootfs copy carrying it.
///
/// `listen <port>` writes [`INTAKE_BANNER`] to every accepted connection;
/// `idle` never listens. The copy carries it at `/sbin/landing`. The shared
/// fixture image is never mutated.
#[must_use]
pub fn stage_guest_rootfs(tmp: &Path, fixture: &VmFixture) -> PathBuf {
    let banner = String::from_utf8_lossy(INTAKE_BANNER).escape_default().to_string();
    let source = tmp.join("landing.rs");
    std::fs::write(
        &source,
        format!(
            "use std::io::Write;\n\
             fn main() {{\n\
                 let args: Vec<String> = std::env::args().collect();\n\
                 if args.get(1).map(String::as_str) == Some(\"listen\") {{\n\
                     let port: u16 = args[2].parse().expect(\"port\");\n\
                     let listener = std::net::TcpListener::bind((\"0.0.0.0\", port)).expect(\"bind\");\n\
                     for stream in listener.incoming() {{\n\
                         if let Ok(mut stream) = stream {{ let _ = stream.write_all(b\"{banner}\"); }}\n\
                     }}\n\
                 }}\n\
                 loop {{ std::thread::sleep(std::time::Duration::from_secs(3600)); }}\n\
             }}\n"
        ),
    )
    .expect("write the guest application source");
    let binary = tmp.join("landing");
    let built = Command::new("rustc")
        .args(["--edition", "2021", "-C", "opt-level=1", "-C", "target-feature=+crt-static"])
        .args(["--target", "x86_64-unknown-linux-musl", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .expect("spawn rustc for the guest application");
    assert!(
        built.status.success(),
        "build the guest application: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let rootfs = tmp.join("rootfs.ext4");
    std::fs::copy(&fixture.rootfs_path, &rootfs).expect("copy the fixture rootfs");
    let mount_point = tmp.join("rootfs-mnt");
    std::fs::create_dir_all(&mount_point).expect("create the rootfs mount point");
    let attached =
        command_output("losetup", &["--find", "--show", rootfs.to_str().expect("utf-8 path")]);
    assert!(attached.status.success(), "losetup: {}", String::from_utf8_lossy(&attached.stderr));
    let device = String::from_utf8_lossy(&attached.stdout).trim().to_owned();
    let mount_path = mount_point.to_str().expect("utf-8 path");
    let mounted = command_output("mount", &[&device, mount_path]);
    assert!(
        mounted.status.success(),
        "mount {device}: {}",
        String::from_utf8_lossy(&mounted.stderr)
    );
    let destination = mount_point.join("sbin/landing");
    std::fs::copy(&binary, &destination).expect("copy the guest application into the rootfs");
    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o755))
        .expect("make the guest application executable");
    let unmounted = command_output("umount", &[mount_path]);
    assert!(unmounted.status.success(), "umount: {}", String::from_utf8_lossy(&unmounted.stderr));
    let detached = command_output("losetup", &["-d", &device]);
    assert!(
        detached.status.success(),
        "losetup -d {device}: {}",
        String::from_utf8_lossy(&detached.stderr)
    );
    rootfs
}

/// One VM service: its id, its declared TCP ports, and whether its guest
/// application listens on its first declared port.
#[derive(Debug, Clone)]
pub struct LandingWorkload {
    pub id: &'static str,
    pub declared: BTreeSet<u16>,
    pub listens: bool,
}

impl LandingWorkload {
    #[must_use]
    pub fn request(&self, fixture: &VmFixture, rootfs: &Path) -> Value {
        let port = self.declared.first().copied().expect("a declared port");
        let args = if self.listens {
            vec!["listen".to_owned(), port.to_string()]
        } else {
            vec!["idle".to_owned()]
        };
        let listeners: Vec<Value> = self
            .declared
            .iter()
            .map(|port| serde_json::json!({ "port": port, "protocol": "tcp" }))
            .collect();
        serde_json::json!({
            "spec": {
                "kind": "service",
                "id": self.id,
                "replicas": 1,
                "resources": { "cpu_milli": 500, "memory_bytes": 268_435_456_u64 },
                "vm": {
                    "command": "/sbin/landing",
                    "args": args,
                    "kernel": fixture.kernel_path,
                    "rootfs": rootfs,
                },
                "listeners": listeners,
                "startup_probes": [],
                "readiness_probes": [],
                "liveness_probes": []
            }
        })
    }
}

/// A production `serve` composition booted in-process on the real kernel.
pub struct LandingNode {
    handle: ServerHandle,
    api: PublicApi,
}

impl LandingNode {
    /// Boot `run_server` with the production adapters over retained
    /// directories, after the same `CgroupFs` probe `serve` runs.
    pub async fn boot(data_dir: &Path, config_dir: &Path) -> Self {
        let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
        fs.probe().await.expect("the host cgroupfs probe passes");
        let config = ServerConfig {
            data_dir: data_dir.to_path_buf(),
            operator_config_dir: config_dir.to_path_buf(),
            ..ServerConfig::new(
                Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
                Arc::new(HostMtlsIntercept::new()),
                Arc::new(HostGuestDnsFactory),
            )
        };
        let handle = run_server(config, fs).await.expect("the production composition boots");
        handle.local_addr().await.expect("the server reports its bound address");
        let api = PublicApi::from_trust_config(&config_dir.join(".overdrive/config"))
            .expect("load the server's trust triple");
        Self { handle, api }
    }

    #[must_use]
    pub const fn api(&self) -> &PublicApi {
        &self.api
    }

    /// The workload address of the workload's Running allocation.
    pub async fn workload_addr(&self, id: &str) -> Option<Ipv4Addr> {
        let response = self.api.get(&format!("/v1/allocs?job={id}")).await.ok()?;
        let status: Value = response.json().await.ok()?;
        status["rows"]
            .as_array()?
            .iter()
            .find(|row| {
                row["state"].as_str().is_some_and(|state| state.eq_ignore_ascii_case("running"))
            })
            .and_then(|row| row["workload_addr"].as_str())
            .and_then(|addr| addr.parse().ok())
    }

    /// `serve` dies the way process death would (`kill_for_test`).
    pub async fn kill(self) {
        let _residue = self.handle.kill_for_test().await;
    }

    /// Graceful shutdown.
    pub async fn shutdown(self) {
        self.handle
            .shutdown(Duration::from_secs(30))
            .await
            .unwrap_or_else(|error| panic!("graceful shutdown: {error:?}"));
    }
}

// ---------------------------------------------------------------------------
// The comparison record
// ---------------------------------------------------------------------------

/// The reachable configurations of QPROJ-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Configuration {
    NeverBooted,
    Serving,
    ServeKilled,
    ServingAfterRestart,
    ServeStoppedGracefully,
}

impl Configuration {
    pub const ALL: [Self; 5] = [
        Self::NeverBooted,
        Self::Serving,
        Self::ServeKilled,
        Self::ServingAfterRestart,
        Self::ServeStoppedGracefully,
    ];

    #[must_use]
    pub const fn serving(self) -> bool {
        matches!(self, Self::Serving | Self::ServingAfterRestart)
    }
}

/// One destination the clients connect to: an address and the declared ports
/// of the allocation leased there (empty for an unleased address).
#[derive(Debug, Clone)]
pub struct Target {
    pub label: &'static str,
    pub addr: Ipv4Addr,
    pub declared: BTreeSet<u16>,
    pub ports: Vec<u16>,
}

/// One compared row, kept in an append-only record.
#[derive(Debug, Clone)]
pub struct Row {
    pub configuration: Configuration,
    pub target: &'static str,
    pub dst: SocketAddrV4,
    pub client: Client,
    pub net: Net,
    pub model: Landing,
    pub observed: Landing,
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} {} {} {}: model {:?} observed {:?} ({:?})",
            self.configuration,
            self.target,
            self.dst,
            self.client.as_str(),
            self.model,
            self.observed,
            self.net
        )
    }
}

#[derive(Debug, Default)]
pub struct ComparisonRecord {
    rows: Vec<Row>,
    notes: Vec<String>,
}

impl ComparisonRecord {
    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// Compare every client class that exists in `configuration` (the marked
    /// clients exist only while `serve` serves) against every target port.
    pub async fn compare(
        &mut self,
        configuration: Configuration,
        targets: &[Target],
        remote: &RemoteClient,
    ) {
        let serving = configuration.serving();
        let mut pending = Vec::new();
        for target in targets {
            let net = read_net(serving, target.addr, &target.declared);
            for &port in &target.ports {
                for client in Client::ALL {
                    if client == Client::Marked && !serving {
                        continue;
                    }
                    let dst = SocketAddrV4::new(target.addr, port);
                    let observed = observe_landing(client, dst, remote).await;
                    pending.push((target.label, dst, client, net.clone(), observed));
                }
            }
        }
        let queries: Vec<LandingQuery> = pending
            .iter()
            .map(|(_, dst, client, net, _)| LandingQuery {
                net: net.clone(),
                client: *client,
                port: dst.port(),
            })
            .collect();
        let models = model_landings(&queries);
        for ((target, dst, client, net, observed), model) in pending.into_iter().zip(models) {
            self.rows.push(Row { configuration, target, dst, client, net, model, observed });
        }
    }

    /// Every row conforms, and every reachable configuration was compared.
    pub fn assert_conforms(&self) {
        let visited: BTreeSet<Configuration> =
            self.rows.iter().map(|row| row.configuration).collect();
        let history: Vec<String> =
            self.notes.iter().cloned().chain(self.rows.iter().map(Row::to_string)).collect();
        let mismatches: Vec<&Row> =
            self.rows.iter().filter(|row| row.model != row.observed).collect();
        assert!(
            mismatches.is_empty(),
            "prefix_landing: {} of {} landings diverge from lands():\n{}\n\nfull record:\n{}",
            mismatches.len(),
            self.rows.len(),
            mismatches.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"),
            history.join("\n")
        );
        let missing: Vec<Configuration> =
            Configuration::ALL.into_iter().filter(|c| !visited.contains(c)).collect();
        assert!(
            missing.is_empty(),
            "configurations never compared: {missing:?}\n{}",
            history.join("\n")
        );
        let clients: BTreeMap<Client, usize> =
            self.rows.iter().fold(BTreeMap::new(), |mut counts, row| {
                *counts.entry(row.client).or_insert(0) += 1;
                counts
            });
        assert_eq!(clients.len(), Client::ALL.len(), "every client class compared: {clients:?}");
    }
}
