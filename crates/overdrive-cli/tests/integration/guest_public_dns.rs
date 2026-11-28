//! Guest public DNS resolution through `overdrive serve` + `overdrive deploy`
//! (GH #382, US-GPD-1..3; ADR-0356..ADR-0362), on native metal.
//!
//! A VM guest is deployed through the operator's verb against a real `serve`.
//! Its program calls `getaddrinfo` through the guest's default resolver (one
//! nameserver, the guest gateway, ADR-0154) and connects. The node's upstream
//! resolver is the host's own resolver file naming a scripted upstream that
//! this test runs; the names it answers lead to a route-disjoint remote peer
//! (the same fixture the vsock walking skeleton uses), so a lookup that
//! resolves proves the connect through the guest's real egress. Nothing is
//! installed in the guest: no resolver, route or rule.
//!
//! **Oracle (observable outcomes only):** the operator sees the Job end
//! `Terminated`; the guest console shows its configured nameserver, each
//! lookup's result and elapsed time, and the byte-exact exchange; the remote
//! peer served the request once; the scripted upstream saw the queries it
//! should and none it should not.
//!
//! One walking skeleton (the first test) and three more guest-level scenarios;
//! the rest of the matrix runs on the wire in
//! `overdrive-control-plane/tests/integration/dns_forwarding_real_wire.rs`.
//! The first three tests are pending: a complete body that fails today (the
//! node drops the guest's public lookups, so each lookup waits out the guest
//! resolver's 5 s timeout) with its semantic RED in `should_panic(expected =
//! ...)`. The last characterises behaviour the feature keeps and is active.
//!
//! Lane: `integration-tests,kvm-tests`, `cargo xtask metal run --`.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]

use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_control_plane::api::AllocStateWire;
use overdrive_testing::dns_upstream::{Behaviour, HostResolverFile, ScriptedDnsUpstream};
use overdrive_testing::guest_app::{
    RESOLVE_CONNECT_OK_LINE, RESOLVE_LINE_PREFIX, RemotePeer, subcommand,
};
use overdrive_testing::overdrive_test;
use serial_test::serial;

use super::vm_walking_skeleton::TeardownBound;
use super::vsock_support::{Lane, alloc_of, console_of};

/// The scripted upstream's address: the host resolver file names it.
const UPSTREAM: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
/// K-GPD-1 / K-GPD-2 (approved 2026-11-25).
const PROMPT_MS: u128 = 1_000;

struct Node {
    lane: Lane,
    upstream: Option<ScriptedDnsUpstream>,
    _resolver_file: HostResolverFile,
}

impl Node {
    /// `serve` first (the node's responder owns `:53`), then the upstream, then
    /// the host resolver file naming it.
    async fn boot(name: &str) -> Self {
        let lane = Lane::boot(name).await;
        let upstream = ScriptedDnsUpstream::start(UPSTREAM).expect("the scripted upstream binds");
        let resolver_file = HostResolverFile::take();
        resolver_file.names(UPSTREAM);
        Self { lane, upstream: Some(upstream), _resolver_file: resolver_file }
    }

    const fn upstream(&self) -> &ScriptedDnsUpstream {
        self.upstream.as_ref().expect("the upstream is running")
    }
}

/// One `resolve:` console line for `name`: the outcome (`addr=<ip>` or
/// `error=<code>`) and the elapsed milliseconds.
fn lookup_line(console: &str, name: &str) -> Option<(String, u128)> {
    let prefix = format!("{RESOLVE_LINE_PREFIX}{name} ");
    let line = console.lines().find_map(|l| l.find(&prefix).map(|at| &l[at + prefix.len()..]))?;
    let mut parts = line.split_whitespace();
    let outcome = parts.next()?.to_owned();
    let ms = parts.next()?.strip_prefix("ms=")?.parse().ok()?;
    Some((outcome, ms))
}

/// Deploys the guest program with `args` and waits for its Job to end; returns
/// the final state and the guest console.
async fn run_job(node: &Node, id: &str, args: &[&str]) -> (AllocStateWire, String) {
    let workload = node.lane.deploy_job(id, args).await;
    let rows = node
        .lane
        .wait_rows(&workload, Duration::from_secs(120), "a terminal allocation", |rows| {
            rows.first().is_some_and(|r| {
                matches!(r.state, AllocStateWire::Terminated | AllocStateWire::Failed)
            })
        })
        .await;
    let row = rows.first().expect("one allocation");
    (row.state, console_of(&alloc_of(row)))
}

/// US-GPD-1 walking skeleton (AC-1, AC-4, K-GPD-1): a VM guest deployed through
/// `overdrive deploy` on `overdrive serve` resolves a public name through its
/// default resolver and connects to what it names.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_vm_guest_resolves_a_public_name_through_its_default_resolver_and_connects() {
    let _bound = TeardownBound::arm();
    let peer = RemotePeer::start("gpd-ws", 0).expect("precondition: the remote peer starts");
    let node = Node::boot("gpd-ws").await;
    node.upstream().script("github.com", Behaviour::Answer(vec![*peer.tcp_addr().ip()]));

    let tag = "gpd-ws";
    let port = peer.tcp_addr().port().to_string();
    let (state, console) =
        run_job(&node, tag, &[subcommand::RESOLVE_CONNECT, "github.com", &port, tag]).await;

    assert_eq!(
        state,
        AllocStateWire::Terminated,
        "the guest must resolve the public name and connect: its program exits 0 only when the \
         lookup returned an address and the byte-exact exchange completed; console:\n{console}"
    );
    assert!(console.contains(RESOLVE_CONNECT_OK_LINE), "console:\n{console}");
    assert!(
        console.contains(&format!("{RESOLVE_LINE_PREFIX}nameserver=100.95.0.1")),
        "the guest's resolver configuration is unchanged: one nameserver, the gateway\n{console}"
    );
    let (outcome, ms) = lookup_line(&console, "github.com").expect("a lookup line");
    assert_eq!(outcome, format!("addr={}", peer.tcp_addr().ip()));
    assert!(ms < PROMPT_MS, "K-GPD-1: the lookup took {ms} ms");
    let served = peer.snapshot();
    assert_eq!(served.requests.len(), 1, "the peer served the guest's request once: {served:?}");
    assert_eq!(node.upstream().seen_count("github.com"), 1, "one upstream exchange");
    node.lane.shutdown().await;
}

/// US-GPD-1 (AC-3, K-GPD-2) and the herdr dev box's usual names (K-GPD-1): each
/// scripted public name resolves promptly, and a name the upstream does not
/// know fails with "Name does not resolve", not the guest resolver's timeout.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn the_dev_boxs_usual_names_resolve_and_a_nonexistent_name_fails_definitely_and_promptly() {
    let _bound = TeardownBound::arm();
    let node = Node::boot("gpd-names").await;
    let usual = ["api.anthropic.com", "github.com", "crates.io", "example.com"];
    for name in usual {
        node.upstream().script(name, Behaviour::Answer(vec![Ipv4Addr::new(203, 0, 113, 10)]));
    }
    let missing = "no-such-host-7f3a.example.com";
    let (state, console) = run_job(
        &node,
        "gpd-names",
        &[subcommand::RESOLVE, usual[0], usual[1], usual[2], usual[3], missing],
    )
    .await;
    assert_eq!(state, AllocStateWire::Terminated, "console:\n{console}");
    for name in usual {
        let (outcome, ms) = lookup_line(&console, name).expect("a lookup line per name");
        assert!(
            outcome.starts_with("addr=") && ms < PROMPT_MS,
            "every lookup must complete promptly with a definite answer: {name} {outcome} {ms} ms\n{console}"
        );
    }
    let (outcome, ms) = lookup_line(&console, missing).expect("a lookup line");
    assert!(
        outcome == "error=EAI_NONAME" && ms < PROMPT_MS,
        "every lookup must complete promptly with a definite answer: {missing} {outcome} {ms} ms\n{console}"
    );
    node.lane.shutdown().await;
}

/// US-GPD-2 (AC-6, AC-7, AC-9, K-GPD-4): an upstream answer inside the mesh
/// frontend range never reaches the guest and the guest still gets a prompt
/// definite result; a private answer outside the owned ranges is delivered.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn an_upstream_answer_in_the_mesh_frontend_range_never_reaches_the_guest() {
    let _bound = TeardownBound::arm();
    let node = Node::boot("gpd-owned").await;
    node.upstream()
        .script("build-cache.corp.example", Behaviour::Answer(vec![Ipv4Addr::new(10, 98, 0, 7)]));
    node.upstream()
        .script("git.corp.example", Behaviour::Answer(vec![Ipv4Addr::new(10, 20, 4, 7)]));
    let (state, console) = run_job(
        &node,
        "gpd-owned",
        &[subcommand::RESOLVE, "build-cache.corp.example", "git.corp.example"],
    )
    .await;
    assert_eq!(state, AllocStateWire::Terminated, "console:\n{console}");
    let (blocked, ms) = lookup_line(&console, "build-cache.corp.example").expect("a lookup line");
    assert!(
        !blocked.contains("10.98.0.7") && ms < PROMPT_MS,
        "the guest must get a prompt definite result for an owned-range answer: {blocked} {ms} ms\n{console}"
    );
    let (benign, _) = lookup_line(&console, "git.corp.example").expect("a lookup line");
    assert_eq!(benign, "addr=10.20.4.7", "a private answer outside the owned ranges is delivered");
    node.lane.shutdown().await;
}

/// US-GPD-3 (AC-10): with the node's upstream unreachable the guest's lookup
/// ends with a definite error result, not a hang or a crashed guest, and the
/// Job still terminates.
///
/// The guest-observed elapsed time is deliberately NOT asserted. It depends on
/// the guest resolver library's SERVFAIL handling: the guest program is built
/// against musl, whose `res_msend` does not accept a SERVFAIL as an answer (it
/// resends a bounded number of times, then keeps waiting for a usable reply),
/// so `getaddrinfo` returns `EAI_AGAIN` at its own timeout whatever the node
/// does; glibc retries a SERVFAIL once. The node's contract (a definite SERVFAIL
/// inside the 4000 ms budget, AC-10) is asserted at the reply level by W-8 in
/// `dns_forwarding_real_wire.rs` and F-8 in `dns_forwarder.rs`, and V-4
/// measures the guest-observed time under glibc and musl at DELIVER. Because
/// the guest sees the same error with and without this feature, this scenario
/// characterises behaviour the feature keeps: it is active, not pending.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn with_the_upstream_unreachable_the_guest_lookup_ends_with_a_definite_error() {
    let _bound = TeardownBound::arm();
    let mut node = Node::boot("gpd-down").await;
    // The upstream goes away: its address now reaches the node itself, which
    // drops what is not addressed to the gateway, so the forward times out.
    node.upstream = None;
    let (state, console) = run_job(&node, "gpd-down", &[subcommand::RESOLVE, "crates.io"]).await;
    assert_eq!(state, AllocStateWire::Terminated, "console:\n{console}");
    let (outcome, _ms) = lookup_line(&console, "crates.io").expect("a lookup line");
    assert!(
        outcome.starts_with("error=") && !outcome.contains("addr="),
        "the lookup must end with a definite error result: {outcome}\n{console}"
    );
    node.lane.shutdown().await;
}
