//! Guest public DNS resolution — the forwarder's contract (GH #382; ADR-0356,
//! ADR-0358, ADR-0359, ADR-0361, ADR-0362). Default lane, in-process, no
//! socket: the REAL `Forwarder` driven through its two driven ports with a
//! scripted upstream and a `SimClock`.
//!
//! Driving port: `Forwarder::{try_acquire, forward}` (the only entry the
//! responder loop uses). Driven ports faked: `UpstreamResolvers` and
//! `UpstreamExchange` (scripted, test-local), `Clock` (`SimClock`). Oracles:
//! the outcome and the reply bytes (decoded with hickory's own parser), what
//! the scripted exchange was asked (server, transport, timeout, query bytes),
//! the elapsed `SimClock` time, and the warn events the forwarder emits.
//!
//! Every test below is a pending scenario: its body is complete and fails
//! today against the DISTILL scaffold with the semantic RED named in its
//! `should_panic(expected = ...)`. DELIVER removes the marker, one scenario at
//! a time (`.claude/rules/testing.md` § "RED acceptance scaffolds").

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]
#![allow(clippy::doc_markdown, clippy::items_after_statements, clippy::type_complexity)]
#![allow(clippy::cast_possible_truncation, clippy::missing_panics_doc, clippy::panic)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::redundant_clone,
    clippy::duration_suboptimal_units,
    clippy::significant_drop_tightening,
    clippy::significant_drop_in_scrutinee,
    reason = "test fixtures read clearer in milliseconds and with plain guards"
)]
#![allow(
    clippy::drop_non_drop,
    reason = "`ForwardPermit` is RAII (released on drop) per the contract; the DISTILL scaffold has no Drop yet, \
              and these drops are the scenario's release step"
)]

use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hickory_proto::op::{Edns, Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::rdata::A;
use hickory_proto::rr::rdata::opt::{EdnsCode, EdnsOption};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use ipnet::Ipv4Net;
use overdrive_control_plane::dns_responder::forwarder::{
    AttemptFailure, FDS_PER_PERMIT, FORWARD_BUDGET, ForwardFailure, ForwardOutcome, ForwardRequest,
    Forwarder, UPSTREAM_ATTEMPT_TIMEOUT, forward_permits,
};
use overdrive_control_plane::dns_responder::owned_ranges::{OwnedAddressRanges, OwnedRangeKind};
use overdrive_control_plane::dns_responder::upstream::{
    DnsTransport, ExchangeError, UpstreamConfigError, UpstreamExchange, UpstreamResolvers,
};
use overdrive_core::traits::clock::Clock;
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_testing::placement;
use parking_lot::Mutex;
use proptest::prelude::*;
use tracing::Level;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

placement!(host_pure);

// ---------------------------------------------------------------------------
// Fixtures: names, addresses, wire builders
// ---------------------------------------------------------------------------

/// The guest gateway the forwarder must never forward to.
const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const UP1: IpAddr = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));
const UP2: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
const UP3: IpAddr = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
const UP4: IpAddr = IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4));

fn vip_range() -> VipRange {
    VipRange::new(
        vec![Ipv4Net::new(Ipv4Addr::new(10, 200, 0, 0), 24).expect("net")],
        BTreeSet::from([Ipv4Addr::new(10, 200, 0, 1)]),
    )
    .expect("a valid service VIP pool")
}

fn query(id: u16, name: &str, qtype: RecordType, edns_do: Option<bool>) -> Vec<u8> {
    let mut message = Message::new(id, MessageType::Query, OpCode::Query);
    message.metadata.recursion_desired = true;
    message.add_query(Query::query(Name::from_ascii(name).expect("name"), qtype));
    if let Some(dnssec_ok) = edns_do {
        let mut edns = Edns::new();
        edns.set_max_payload(4096);
        edns.set_dnssec_ok(dnssec_ok);
        message.set_edns(edns);
    }
    message.to_vec().expect("encode query")
}

/// What an upstream's reply carries, built from the query it answers so the id
/// and question match.
#[derive(Clone, Default)]
struct Reply {
    rcode: Option<ResponseCode>,
    answers: Vec<RData>,
    additionals: Vec<RData>,
    authentic: bool,
    truncated: bool,
}

impl Reply {
    fn a(addrs: &[[u8; 4]]) -> Self {
        Self {
            answers: addrs.iter().map(|o| RData::A(A::from(Ipv4Addr::from(*o)))).collect(),
            ..Self::default()
        }
    }
    fn rcode(rcode: ResponseCode) -> Self {
        Self { rcode: Some(rcode), ..Self::default() }
    }
    fn render(&self, query_bytes: &[u8]) -> Vec<u8> {
        let q = Message::from_vec(query_bytes).expect("decode query");
        let mut m = Message::new(q.metadata.id, MessageType::Response, OpCode::Query);
        m.metadata.recursion_desired = q.metadata.recursion_desired;
        m.metadata.recursion_available = true;
        m.metadata.response_code = self.rcode.unwrap_or(ResponseCode::NoError);
        m.metadata.authentic_data = self.authentic;
        m.metadata.truncation = self.truncated;
        let owner = q.queries[0].name().clone();
        for query in &q.queries {
            m.add_query(query.clone());
        }
        for rdata in &self.answers {
            m.add_answer(Record::from_rdata(owner.clone(), 60, rdata.clone()));
        }
        for rdata in &self.additionals {
            m.add_additional(Record::from_rdata(owner.clone(), 60, rdata.clone()));
        }
        m.to_vec().expect("encode reply")
    }
}

fn decode(bytes: &[u8]) -> Message {
    Message::from_vec(bytes).expect("a reply the guest's resolver can decode")
}

fn answer_addrs(m: &Message) -> Vec<Ipv4Addr> {
    m.answers
        .iter()
        .filter_map(|r| match &r.data {
            RData::A(a) => Some(a.0),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Scripted driven ports (test-local; the DESIGN's Sim* upstream adapters are
// not required by these tests)
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Config {
    Candidates(Vec<IpAddr>),
    Absent,
    Unreadable(io::ErrorKind),
}

struct ScriptedResolvers {
    config: Mutex<Config>,
    reads: Mutex<usize>,
}

impl ScriptedResolvers {
    fn new(candidates: Vec<IpAddr>) -> Arc<Self> {
        Arc::new(Self { config: Mutex::new(Config::Candidates(candidates)), reads: Mutex::new(0) })
    }
    fn set(&self, config: Config) {
        *self.config.lock() = config;
    }
    fn reads(&self) -> usize {
        *self.reads.lock()
    }
}

#[async_trait::async_trait]
impl UpstreamResolvers for ScriptedResolvers {
    async fn candidates(&self) -> Result<Vec<IpAddr>, UpstreamConfigError> {
        *self.reads.lock() += 1;
        match self.config.lock().clone() {
            Config::Candidates(c) => Ok(c),
            Config::Absent => Err(UpstreamConfigError::Absent),
            Config::Unreadable(kind) => {
                Err(UpstreamConfigError::Unreadable { source: io::Error::from(kind) })
            }
        }
    }
}

/// One scripted upstream behaviour.
enum Step {
    /// Answer with a reply built from the query.
    Respond(Reply),
    /// Answer with these exact bytes whatever the query.
    Bytes(Vec<u8>),
    /// Never answer: the attempt consumes its whole timeout.
    Silent,
    /// ICMP-style refusal, instantly.
    Refused,
    /// Datagram larger than the receive buffer.
    TooLarge,
}

/// One recorded call to the exchange.
#[derive(Clone, Debug)]
struct Call {
    server: IpAddr,
    transport: DnsTransport,
    timeout: Duration,
    query: Vec<u8>,
    at: Duration,
}

struct ScriptedExchange {
    clock: Arc<SimClock>,
    start: std::time::Instant,
    scripts: Mutex<Vec<(IpAddr, VecDeque<Step>)>>,
    calls: Mutex<Vec<Call>>,
}

impl ScriptedExchange {
    fn new(clock: &Arc<SimClock>, scripts: Vec<(IpAddr, Vec<Step>)>) -> Arc<Self> {
        Arc::new(Self {
            clock: Arc::clone(clock),
            start: clock.now(),
            scripts: Mutex::new(scripts.into_iter().map(|(a, s)| (a, s.into())).collect()),
            calls: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().clone()
    }
    fn servers(&self) -> Vec<IpAddr> {
        self.calls().iter().map(|c| c.server).collect()
    }
}

#[async_trait::async_trait]
impl UpstreamExchange for ScriptedExchange {
    async fn exchange(
        &self,
        server: IpAddr,
        transport: DnsTransport,
        query: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, ExchangeError> {
        self.calls.lock().push(Call {
            server,
            transport,
            timeout,
            query: query.to_vec(),
            at: self.clock.now().duration_since(self.start),
        });
        let step = {
            let mut scripts = self.scripts.lock();
            let queue = scripts.iter_mut().find(|(a, _)| *a == server).map(|(_, q)| q);
            match queue {
                // The last step repeats, so one scripted step covers every later attempt.
                Some(q) if q.len() > 1 => q.pop_front(),
                Some(q) => q.front().map(|s| match s {
                    Step::Respond(r) => Step::Respond(r.clone()),
                    Step::Bytes(b) => Step::Bytes(b.clone()),
                    Step::Silent => Step::Silent,
                    Step::Refused => Step::Refused,
                    Step::TooLarge => Step::TooLarge,
                }),
                None => Some(Step::Refused),
            }
        };
        match step.unwrap_or(Step::Refused) {
            Step::Respond(reply) => Ok(reply.render(query)),
            Step::Bytes(bytes) => Ok(bytes),
            Step::Silent => {
                self.clock.tick(timeout);
                Err(ExchangeError::Timeout)
            }
            Step::Refused => Err(ExchangeError::Unreachable {
                source: io::Error::from(io::ErrorKind::ConnectionRefused),
            }),
            Step::TooLarge => Err(ExchangeError::ReplyTooLarge),
        }
    }
}

struct World {
    clock: Arc<SimClock>,
    resolvers: Arc<ScriptedResolvers>,
    exchange: Arc<ScriptedExchange>,
    forwarder: Forwarder,
}

fn world_with(candidates: Vec<IpAddr>, scripts: Vec<(IpAddr, Vec<Step>)>, permits: usize) -> World {
    let clock = Arc::new(SimClock::new());
    let resolvers = ScriptedResolvers::new(candidates);
    let exchange = ScriptedExchange::new(&clock, scripts);
    let forwarder = Forwarder::new(
        Arc::clone(&resolvers) as Arc<dyn UpstreamResolvers>,
        Arc::clone(&exchange) as Arc<dyn UpstreamExchange>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        OwnedAddressRanges::platform(&vip_range()),
        GATEWAY,
        permits,
    );
    World { clock, resolvers, exchange, forwarder }
}

fn world(candidates: Vec<IpAddr>, scripts: Vec<(IpAddr, Vec<Step>)>) -> World {
    world_with(candidates, scripts, 8)
}

async fn forward(w: &World, wire: &[u8], transport: DnsTransport) -> ForwardOutcome {
    let permit = w.forwarder.try_acquire().expect("a permit is available");
    w.forwarder.forward(&ForwardRequest { wire, transport }, &permit).await
}

// ---------------------------------------------------------------------------
// Event capture (the episode warns)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<(Level, String)>>>);

impl Events {
    fn count(&self, name: &str) -> usize {
        self.0.lock().iter().filter(|(_, n)| n == name).count()
    }
    fn warns(&self) -> Vec<String> {
        self.0.lock().iter().filter(|(l, _)| *l <= Level::WARN).map(|(_, n)| n.clone()).collect()
    }
}

struct Capture(Events);

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        self.0.0.lock().push((*meta.level(), meta.name().to_owned()));
    }
}

fn capture() -> (Events, tracing::subscriber::DefaultGuard) {
    let events = Events::default();
    let subscriber = tracing_subscriber::registry().with(Capture(events.clone()));
    (events, tracing::subscriber::set_default(subscriber))
}

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

// ---------------------------------------------------------------------------
// Relay: verbatim queries, verbatim replies
// ---------------------------------------------------------------------------

/// Contract shape: bounded-change. AC-1, AC-3, ADR-0356 step 3 and 5: the
/// guest's query bytes reach the upstream unchanged, and the upstream's reply
/// reaches the guest byte for byte.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_public_lookup_is_sent_upstream_unchanged_and_the_upstream_reply_is_relayed_byte_for_byte() {
    run(async {
        let q = query(0x4242, "github.com.", RecordType::A, Some(true));
        let w = world(vec![UP1], vec![(UP1, vec![Step::Respond(Reply::a(&[[140, 82, 121, 4]]))])]);
        let outcome = forward(&w, &q, DnsTransport::Udp).await;
        let calls = w.exchange.calls();
        assert_eq!(calls.len(), 1, "one exchange for one lookup");
        assert_eq!(calls[0].query, q, "the guest's query goes upstream with no byte changed");
        assert_eq!(calls[0].server, UP1);
        assert_eq!(calls[0].transport, DnsTransport::Udp);
        let expected = Reply::a(&[[140, 82, 121, 4]]).render(&q);
        match outcome {
            ForwardOutcome::Relayed(bytes) => {
                assert_eq!(bytes, expected, "the reply is relayed verbatim");
            }
            other => panic!("expected a verbatim relay, got {other:?}"),
        }
    });
}

/// Contract shape: bounded-change. AC-3 (a nonexistent public name returns
/// NXDOMAIN), ADR-0356 step 5: an accepted reply of ANY rcode is final; there
/// is no failover on an rcode.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_upstream_reply_of_any_rcode_is_final_and_never_triggers_failover() {
    run(async {
        for rcode in [
            ResponseCode::NoError,
            ResponseCode::NXDomain,
            ResponseCode::ServFail,
            ResponseCode::Refused,
        ] {
            let q = query(7, "no-such-host-7f3a.example.com.", RecordType::A, None);
            let w = world(
                vec![UP1, UP2],
                vec![(UP1, vec![Step::Respond(Reply::rcode(rcode))]), (UP2, vec![Step::Silent])],
            );
            let outcome = forward(&w, &q, DnsTransport::Udp).await;
            assert_eq!(
                w.exchange.servers(),
                vec![UP1],
                "{rcode:?}: the second upstream is never asked"
            );
            let ForwardOutcome::Relayed(bytes) = outcome else { panic!("{rcode:?}: relayed") };
            assert_eq!(
                decode(&bytes).metadata.response_code,
                rcode,
                "the upstream's rcode reaches the guest"
            );
        }
    });
}

/// Contract shape: bounded-change. ADR-0361: every non-mesh record type
/// passes through; AAAA, MX, TXT and HTTPS are relayed like A.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn every_non_mesh_record_type_is_relayed_unchanged() {
    run(async {
        for qtype in [
            RecordType::A,
            RecordType::AAAA,
            RecordType::MX,
            RecordType::TXT,
            RecordType::HTTPS,
            RecordType::CNAME,
        ] {
            let q = query(11, "crates.io.", qtype, None);
            let reply = Reply {
                answers: vec![RData::A(A::from(Ipv4Addr::new(52, 85, 1, 1)))],
                ..Reply::default()
            };
            let w = world(vec![UP1], vec![(UP1, vec![Step::Respond(reply.clone())])]);
            let outcome = forward(&w, &q, DnsTransport::Udp).await;
            let ForwardOutcome::Relayed(bytes) = outcome else { panic!("{qtype:?}: relayed") };
            assert_eq!(bytes, reply.render(&q), "{qtype:?}: no byte changed");
        }
    });
}

/// Contract shape: bounded-change. ADR-0361 and validation item V-6: a
/// DNSSEC-signed reply (AD bit, TC bit, an RRSIG record) is relayed with no
/// byte changed, and is never mistaken for an undecodable reply.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_dnssec_signed_truncated_reply_is_relayed_with_every_bit_preserved() {
    run(async {
        let q = query(0x0102, "example.com.", RecordType::A, Some(true));
        let signed = signed_reply_bytes(&q);
        let w = world(vec![UP1], vec![(UP1, vec![Step::Bytes(signed.clone())])]);
        let outcome = forward(&w, &q, DnsTransport::Udp).await;
        let ForwardOutcome::Relayed(bytes) = outcome else { panic!("a signed reply is relayed") };
        assert_eq!(bytes, signed, "RRSIG, AD and TC reach the guest untouched");
        assert_eq!(w.exchange.servers(), vec![UP1], "the signed reply was decodable: no failover");
        let m = decode(&bytes);
        assert!(m.metadata.authentic_data && m.metadata.truncation);
    });
}

/// A reply for `query_bytes` carrying one A record and one hand-assembled RRSIG
/// record, with the AD and TC bits set.
fn signed_reply_bytes(query_bytes: &[u8]) -> Vec<u8> {
    let base = Reply { authentic: true, truncated: true, ..Reply::a(&[[93, 184, 216, 34]]) }
        .render(query_bytes);
    let mut m = Message::from_vec(&base).expect("base");
    let mut raw = base;
    // Append an RRSIG RR (type 46) to the answer section by bumping ANCOUNT.
    let name = m.queries[0].name().to_ascii();
    let _ = &mut m;
    let mut owner = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        owner.push(label.len() as u8);
        owner.extend_from_slice(label.as_bytes());
    }
    owner.push(0);
    let mut rdata = Vec::new();
    rdata.extend_from_slice(&1u16.to_be_bytes()); // type covered: A
    rdata.push(13); // algorithm
    rdata.push(2); // labels
    rdata.extend_from_slice(&3600u32.to_be_bytes()); // original ttl
    rdata.extend_from_slice(&4_000_000_000u32.to_be_bytes()); // expiration
    rdata.extend_from_slice(&1_000_000_000u32.to_be_bytes()); // inception
    rdata.extend_from_slice(&2371u16.to_be_bytes()); // key tag
    rdata.extend_from_slice(&owner); // signer name (uncompressed)
    rdata.extend_from_slice(&[7u8; 64]); // signature
    raw.extend_from_slice(&owner);
    raw.extend_from_slice(&46u16.to_be_bytes());
    raw.extend_from_slice(&1u16.to_be_bytes());
    raw.extend_from_slice(&60u32.to_be_bytes());
    raw.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
    raw.extend_from_slice(&rdata);
    let ancount = u16::from_be_bytes([raw[6], raw[7]]) + 1;
    raw[6..8].copy_from_slice(&ancount.to_be_bytes());
    raw
}

/// Contract shape: bounded-change. ADR-0356 step 3: a TCP guest query is
/// forwarded over TCP.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_query_that_arrived_over_tcp_is_forwarded_over_tcp() {
    run(async {
        let q = query(5, "github.com.", RecordType::A, None);
        let w = world(vec![UP1], vec![(UP1, vec![Step::Respond(Reply::a(&[[140, 82, 121, 4]]))])]);
        let _ = forward(&w, &q, DnsTransport::Tcp).await;
        assert_eq!(w.exchange.calls()[0].transport, DnsTransport::Tcp);
    });
}

// ---------------------------------------------------------------------------
// Failover and the 4000 ms budget (AC-10, K-GPD-5)
// ---------------------------------------------------------------------------

/// Contract shape: bounded-change. A silent first upstream costs one attempt
/// timeout, then the second upstream answers: the guest gets its answer well
/// inside its 5 s resolver timeout.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_silent_first_upstream_fails_over_to_the_second_after_one_attempt_timeout() {
    run(async {
        let q = query(1, "crates.io.", RecordType::A, None);
        let w = world(
            vec![UP1, UP2],
            vec![
                (UP1, vec![Step::Silent]),
                (UP2, vec![Step::Respond(Reply::a(&[[52, 85, 1, 1]]))]),
            ],
        );
        let outcome = forward(&w, &q, DnsTransport::Udp).await;
        let calls = w.exchange.calls();
        assert_eq!(calls.iter().map(|c| c.server).collect::<Vec<_>>(), vec![UP1, UP2]);
        assert_eq!(calls[0].timeout, UPSTREAM_ATTEMPT_TIMEOUT);
        assert_eq!(
            calls[1].at, UPSTREAM_ATTEMPT_TIMEOUT,
            "the second attempt starts when the first timed out"
        );
        assert!(matches!(outcome, ForwardOutcome::Relayed(_)));
    });
}

/// Contract shape: bounded-change. A refused connection is not a wait: the
/// next upstream is tried at once.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_refusing_upstream_costs_no_time_before_the_next_is_tried() {
    run(async {
        let q = query(2, "crates.io.", RecordType::A, None);
        let w = world(
            vec![UP1, UP2],
            vec![
                (UP1, vec![Step::Refused]),
                (UP2, vec![Step::Respond(Reply::a(&[[52, 85, 1, 1]]))]),
            ],
        );
        let _ = forward(&w, &q, DnsTransport::Udp).await;
        assert_eq!(w.exchange.calls()[1].at, Duration::ZERO);
    });
}

/// Contract shape: bounded-change. AC-10 and ADR-0356 step 3/6/7: with every
/// upstream silent the budget, not the upstream count, ends the loop; the third
/// attempt is trimmed to the time left and the guest gets a SERVFAIL (echoing
/// its id and question, no records) no later than the budget.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn with_every_upstream_silent_the_guest_gets_a_servfail_within_the_budget() {
    run(async {
        let q = query(0x0BAD, "crates.io.", RecordType::A, None);
        let w = world(
            vec![UP1, UP2, UP3, UP4],
            vec![
                (UP1, vec![Step::Silent]),
                (UP2, vec![Step::Silent]),
                (UP3, vec![Step::Silent]),
                (UP4, vec![Step::Silent]),
            ],
        );
        let outcome = forward(&w, &q, DnsTransport::Udp).await;
        let calls = w.exchange.calls();
        assert_eq!(
            calls.iter().map(|c| c.timeout).collect::<Vec<_>>(),
            vec![UPSTREAM_ATTEMPT_TIMEOUT, UPSTREAM_ATTEMPT_TIMEOUT, Duration::from_millis(1000)],
            "1500, 1500, then only the 1000 ms left; the fourth upstream is never tried"
        );
        assert!(w.clock.now().duration_since(w.exchange.start) <= FORWARD_BUDGET);
        let ForwardOutcome::Failed { reply, cause } = outcome else { panic!("a failed forward") };
        assert!(matches!(
            cause,
            ForwardFailure::Attempts {
                last: Some(AttemptFailure::Timeout),
                budget_exhausted: true
            }
        ));
        let m = decode(&reply);
        assert_eq!(m.metadata.id, 0x0BAD, "the guest's id is echoed");
        assert_eq!(m.metadata.response_code, ResponseCode::ServFail);
        assert!(m.metadata.recursion_desired && m.metadata.recursion_available);
        assert!(m.answers.is_empty() && m.authorities.is_empty());
        assert_eq!(m.queries.len(), 1, "the question is echoed");
    });
}

/// Contract shape: bounded-change. AC-10: when every upstream refuses, the
/// guest gets a definite SERVFAIL at once, not after a wait.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn with_every_upstream_refusing_the_servfail_is_immediate_and_names_the_last_failure() {
    run(async {
        let q = query(3, "crates.io.", RecordType::A, None);
        let w = world(vec![UP1, UP2], vec![(UP1, vec![Step::Refused]), (UP2, vec![Step::Refused])]);
        let outcome = forward(&w, &q, DnsTransport::Udp).await;
        assert_eq!(w.clock.now().duration_since(w.exchange.start), Duration::ZERO);
        let ForwardOutcome::Failed { reply, cause } = outcome else { panic!("failed") };
        assert!(matches!(
            cause,
            ForwardFailure::Attempts {
                last: Some(AttemptFailure::Unreachable { kind: io::ErrorKind::ConnectionRefused }),
                budget_exhausted: false
            }
        ));
        assert_eq!(decode(&reply).metadata.response_code, ResponseCode::ServFail);
    });
}

/// Contract shape: bounded-change. ADR-0356 step 4: a reply that cannot be
/// decoded is a failed attempt and the next upstream is tried; an address that
/// cannot be inspected is never relayed.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_undecodable_reply_is_a_failed_attempt_and_the_next_upstream_is_tried() {
    run(async {
        let q = query(4, "crates.io.", RecordType::A, None);
        let w = world(
            vec![UP1, UP2],
            vec![
                (UP1, vec![Step::Bytes(vec![0xde, 0xad, 0xbe])]),
                (UP2, vec![Step::Respond(Reply::a(&[[52, 85, 1, 1]]))]),
            ],
        );
        let outcome = forward(&w, &q, DnsTransport::Udp).await;
        assert_eq!(w.exchange.servers(), vec![UP1, UP2]);
        assert!(matches!(outcome, ForwardOutcome::Relayed(_)));

        let only = world(vec![UP1], vec![(UP1, vec![Step::Bytes(vec![0xde, 0xad, 0xbe])])]);
        let ForwardOutcome::Failed { cause, .. } = forward(&only, &q, DnsTransport::Udp).await
        else {
            panic!("failed")
        };
        assert!(matches!(
            cause,
            ForwardFailure::Attempts { last: Some(AttemptFailure::UndecodableReply), .. }
        ));
    });
}

/// Contract shape: bounded-change. ADR-0356: an oversize UDP reply is a
/// distinct failure mode and is treated like any failed attempt.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_reply_too_large_for_the_buffer_is_a_failed_attempt_with_its_own_cause() {
    run(async {
        let q = query(6, "crates.io.", RecordType::A, None);
        let w = world(vec![UP1], vec![(UP1, vec![Step::TooLarge])]);
        let ForwardOutcome::Failed { cause, .. } = forward(&w, &q, DnsTransport::Udp).await else {
            panic!("failed")
        };
        assert!(matches!(
            cause,
            ForwardFailure::Attempts { last: Some(AttemptFailure::ReplyTooLarge), .. }
        ));
    });
}

// ---------------------------------------------------------------------------
// Upstream selection and the resolver source (ADR-0358)
// ---------------------------------------------------------------------------

fn v4(o: [u8; 4]) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(o))
}

fn mapped_gateway() -> IpAddr {
    IpAddr::V6(GATEWAY.to_ipv6_mapped())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Contract shape: pure-function (observed through the exchange). ADR-0358:
    /// the gateway, in its IPv4 and IPv4-mapped IPv6 forms, is the one address
    /// excluded from the candidates; every other candidate is tried, in file order.
    #[test]
    #[should_panic(expected = "forwarder: exchange the query verbatim")]
    fn only_the_gateway_is_excluded_and_file_order_is_kept(
        others in proptest::collection::vec((1u8..=250, 1u8..=250, 1u8..=250), 0..5),
        gateway_slots in proptest::collection::vec((0usize..6, any::<bool>()), 0..4),
    ) {
        let mut candidates: Vec<IpAddr> =
            others.iter().map(|(a, b, c)| v4([9, *a, *b, *c])).collect();
        candidates.dedup();
        let expected = candidates.clone();
        for (slot, mapped) in &gateway_slots {
            let at = (*slot).min(candidates.len());
            candidates.insert(at, if *mapped { mapped_gateway() } else { IpAddr::V4(GATEWAY) });
        }
        let scripts = candidates.iter().map(|c| (*c, vec![Step::Refused])).collect();
        let w = world(candidates.clone(), scripts);
        let outcome = run(async { forward(&w, &query(9, "crates.io.", RecordType::A, None), DnsTransport::Udp).await });
        prop_assert_eq!(w.exchange.servers(), expected.clone(), "only non-gateway candidates, in order");
        let ForwardOutcome::Failed { cause, .. } = outcome else { panic!("all refused: failed") };
        if expected.is_empty() {
            prop_assert!(matches!(cause, ForwardFailure::NoUsableUpstream | ForwardFailure::NoNameserverListed));
        }
    }
}

/// Contract shape: bounded-change. GPD-3 / ADR-0358: a resolver list holding
/// only the gateway is `NoUsableUpstream`: nothing is exchanged, so the
/// forwarder can never loop to itself.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_resolver_list_naming_only_the_gateway_is_no_usable_upstream_and_nothing_is_sent() {
    run(async {
        let q = query(8, "github.com.", RecordType::A, None);
        let w = world(vec![IpAddr::V4(GATEWAY), mapped_gateway()], vec![]);
        let ForwardOutcome::Failed { reply, cause } = forward(&w, &q, DnsTransport::Udp).await
        else {
            panic!("failed")
        };
        assert!(matches!(cause, ForwardFailure::NoUsableUpstream));
        assert!(w.exchange.calls().is_empty(), "no upstream socket is opened");
        assert_eq!(decode(&reply).metadata.response_code, ResponseCode::ServFail);
    });
}

/// Contract shape: bounded-change. ADR-0358, ruling B: an absent, unreadable
/// or empty resolver file is a SERVFAIL with its own cause, never a boot
/// refusal; the file is re-read for every query, so restoring it recovers with
/// no restart (AC-12).
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_unusable_resolver_source_is_a_servfail_with_a_distinct_cause_and_recovers_when_restored() {
    run(async {
        let q = query(12, "github.com.", RecordType::A, None);
        let w = world(vec![UP1], vec![(UP1, vec![Step::Respond(Reply::a(&[[140, 82, 121, 4]]))])]);

        w.resolvers.set(Config::Absent);
        let ForwardOutcome::Failed { cause, .. } = forward(&w, &q, DnsTransport::Udp).await else {
            panic!("absent")
        };
        assert!(matches!(cause, ForwardFailure::ConfigAbsent));

        w.resolvers.set(Config::Unreadable(io::ErrorKind::PermissionDenied));
        let ForwardOutcome::Failed { cause, .. } = forward(&w, &q, DnsTransport::Udp).await else {
            panic!("unreadable")
        };
        assert!(matches!(
            cause,
            ForwardFailure::ConfigUnreadable { kind: io::ErrorKind::PermissionDenied }
        ));

        w.resolvers.set(Config::Candidates(vec![]));
        let ForwardOutcome::Failed { cause, .. } = forward(&w, &q, DnsTransport::Udp).await else {
            panic!("empty")
        };
        assert!(matches!(cause, ForwardFailure::NoNameserverListed));
        assert!(w.exchange.calls().is_empty(), "no exchange while the source is unusable");

        w.resolvers.set(Config::Candidates(vec![UP1]));
        let outcome = forward(&w, &q, DnsTransport::Udp).await;
        assert!(
            matches!(outcome, ForwardOutcome::Relayed(_)),
            "the next query succeeds, no restart"
        );
        assert_eq!(w.resolvers.reads(), 4, "the source is read on every forwarded query");
    });
}

// ---------------------------------------------------------------------------
// The owned-range guard on a forwarded reply (US-GPD-2, AC-6..9; ADR-0359)
// ---------------------------------------------------------------------------

async fn blocked(
    name: &str,
    qtype: RecordType,
    edns_do: Option<bool>,
    reply: Reply,
) -> (ForwardOutcome, Vec<u8>) {
    let q = query(0x5150, name, qtype, edns_do);
    let w = world(vec![UP1], vec![(UP1, vec![Step::Respond(reply)])]);
    (forward(&w, &q, DnsTransport::Udp).await, q)
}

/// Contract shape: bounded-change. AC-6, AC-9: an upstream answer inside the
/// mesh frontend range is emptied whole and marked Blocked; the guest gets a
/// definite reply (rcode kept, no addresses), never silence and never the
/// owned address. EDNS query: one OPT, payload 1232, DO copied, one EDE 15.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_upstream_answer_inside_the_mesh_frontend_range_is_emptied_and_marked_blocked() {
    run(async {
        for dnssec_ok in [true, false] {
            let reply = Reply { authentic: true, truncated: true, ..Reply::a(&[[10, 98, 0, 7]]) };
            let (outcome, q) =
                blocked("build-cache.corp.example.", RecordType::A, Some(dnssec_ok), reply).await;
            let ForwardOutcome::OwnedRangeBlocked { reply, address, range } = outcome else {
                panic!("an owned answer is blocked")
            };
            assert_eq!(address, Ipv4Addr::new(10, 98, 0, 7));
            assert_eq!(range, OwnedRangeKind::MeshFrontend);
            let m = decode(&reply);
            assert_eq!(m.metadata.id, 0x5150, "the guest's id is kept");
            assert_eq!(
                m.metadata.response_code,
                ResponseCode::NoError,
                "the upstream's rcode is kept"
            );
            assert!(m.answers.is_empty() && m.authorities.is_empty() && m.additionals.is_empty());
            assert!(!m.metadata.authentic_data && !m.metadata.truncation, "AD and TC are cleared");
            assert_eq!(m.queries.len(), 1, "the question is kept");
            let edns = m.edns.expect("an EDNS query gets an OPT");
            assert_eq!(edns.max_payload(), 1232);
            assert_eq!(edns.version(), 0);
            assert_eq!(edns.flags().dnssec_ok, dnssec_ok, "DO is copied from the query (RFC 3225)");
            assert_eq!(
                edns.option(EdnsCode::Unknown(15)),
                Some(&EdnsOption::Unknown(15, vec![0, 15])),
                "one Extended DNS Error, INFO-CODE 15 Blocked, empty EXTRA-TEXT"
            );
            let _ = q;
        }
    });
}

/// Contract shape: bounded-change. ADR-0359: a query without EDNS gets a
/// blocked reply with no OPT record at all.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_blocked_reply_to_a_query_without_edns_carries_no_opt_record() {
    run(async {
        let (outcome, _) =
            blocked("build-cache.corp.example.", RecordType::A, None, Reply::a(&[[10, 98, 0, 7]]))
                .await;
        let ForwardOutcome::OwnedRangeBlocked { reply, .. } = outcome else { panic!("blocked") };
        let m = decode(&reply);
        assert!(m.edns.is_none(), "no OPT without an EDNS query");
        assert!(m.answers.is_empty());
    });
}

/// Contract shape: bounded-change. AC-6: the outcome names the owned range of the
/// answer for the two ranges no other scenario here names: a reserved VIP
/// address (counted as owned) and the guest prefix. The mesh frontend range is
/// named by the scenarios above; edges and random addresses by the owned-range
/// scenarios and the property below.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_answer_on_a_reserved_vip_or_in_the_guest_prefix_is_blocked_with_its_range() {
    run(async {
        for (addr, kind) in [
            ([10, 200, 0, 1], OwnedRangeKind::ServiceVip),
            ([100, 95, 3, 4], OwnedRangeKind::GuestNode),
        ] {
            let (outcome, _) =
                blocked("x.corp.example.", RecordType::A, None, Reply::a(&[addr])).await;
            let ForwardOutcome::OwnedRangeBlocked { range, address, .. } = outcome else {
                panic!("{addr:?} is blocked")
            };
            assert_eq!(range, kind);
            assert_eq!(address, Ipv4Addr::from(addr));
        }
    });
}

/// Contract shape: bounded-change. AC-7: a private answer outside the owned
/// ranges is delivered unchanged, including the `10.99.0.0/16` workload range
/// that is deliberately not owned. Range edges are the owned-range scenarios'.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_private_answer_outside_the_owned_ranges_is_delivered_unchanged() {
    run(async {
        for addr in [[10, 20, 4, 7], [10, 99, 0, 9]] {
            let (outcome, _) =
                blocked("git.corp.example.", RecordType::A, None, Reply::a(&[addr])).await;
            let ForwardOutcome::Relayed(bytes) = outcome else { panic!("{addr:?} is delivered") };
            assert_eq!(answer_addrs(&decode(&bytes)), vec![Ipv4Addr::from(addr)]);
        }
    });
}

/// Contract shape: bounded-change. ADR-0359 accepted non-coverage, pinned so a
/// change is deliberate: the guard reads answer-section A and SVCB/HTTPS
/// ipv4hint only; AAAA and an owned address in the additional section are
/// relayed (the revisit trigger is guest IPv6 transport, GH #308).
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn aaaa_and_additional_section_addresses_are_outside_the_guard_by_design() {
    run(async {
        let aaaa = Reply {
            answers: vec![RData::AAAA(hickory_proto::rr::rdata::AAAA::from(
                Ipv4Addr::new(10, 98, 0, 7).to_ipv6_mapped(),
            ))],
            ..Reply::default()
        };
        let (outcome, _) = blocked("v6.corp.example.", RecordType::AAAA, None, aaaa).await;
        assert!(matches!(outcome, ForwardOutcome::Relayed(_)), "AAAA is not checked");

        let glue = Reply {
            answers: vec![RData::A(A::from(Ipv4Addr::new(52, 1, 1, 1)))],
            additionals: vec![RData::A(A::from(Ipv4Addr::new(10, 98, 0, 7)))],
            ..Reply::default()
        };
        let (outcome, _) = blocked("g.corp.example.", RecordType::A, None, glue).await;
        assert!(
            matches!(outcome, ForwardOutcome::Relayed(_)),
            "the additional section is not checked"
        );
        let _ = Ipv6Addr::LOCALHOST;
    });
}

/// Contract shape: bounded-change. ADR-0359: an HTTPS/SVCB record whose
/// `ipv4hint` carries an owned address blocks the reply like an A record.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_https_record_whose_ipv4_hint_is_owned_is_blocked() {
    use hickory_proto::rr::rdata::HTTPS;
    use hickory_proto::rr::rdata::svcb::{IpHint, SVCB, SvcParamKey, SvcParamValue};
    run(async {
        let hint = SvcParamValue::Ipv4Hint(IpHint(vec![A::from(Ipv4Addr::new(10, 98, 0, 7))]));
        let svcb = SVCB::new(1, Name::root(), vec![(SvcParamKey::Ipv4Hint, hint)]);
        let reply = Reply { answers: vec![RData::HTTPS(HTTPS(svcb))], ..Reply::default() };
        let (outcome, _) = blocked("svc.corp.example.", RecordType::HTTPS, None, reply).await;
        let ForwardOutcome::OwnedRangeBlocked { range, .. } = outcome else { panic!("blocked") };
        assert_eq!(range, OwnedRangeKind::MeshFrontend);
    });
}

/// Contract shape: bounded-change. Ruling C2: a blocked answer is a per-query
/// debug event (`dns.forward.owned_range_blocked`, emitted by the forwarder, the
/// one owner that knows the upstream the event names), never a warn: many
/// blocked lookups raise no warn at all.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn blocked_answers_never_raise_a_warn() {
    run(async {
        let (events, _guard) = capture();
        let (outcome, _) =
            blocked("x.corp.example.", RecordType::A, None, Reply::a(&[[10, 98, 0, 7]])).await;
        assert!(
            matches!(outcome, ForwardOutcome::OwnedRangeBlocked { .. }),
            "the answer is blocked: {outcome:?}"
        );
        assert_eq!(
            events.count("dns.forward.owned_range_blocked"),
            1,
            "one debug event for the one blocked query"
        );
        for _ in 0..4 {
            let _ =
                blocked("x.corp.example.", RecordType::A, None, Reply::a(&[[10, 98, 0, 7]])).await;
        }
        assert_eq!(
            events.count("dns.forward.owned_range_blocked"),
            5,
            "one debug event per blocked query"
        );
        assert!(events.warns().is_empty(), "no warn per blocked query: {:?}", events.warns());
    });
}

// ---------------------------------------------------------------------------
// Failure episode warn (boundary scenarios 3 and 7; ADR-0356 step 8)
// ---------------------------------------------------------------------------

async fn fail_once(w: &World) {
    let q = query(1, "crates.io.", RecordType::A, None);
    let ForwardOutcome::Failed { .. } = forward(w, &q, DnsTransport::Udp).await else {
        panic!("every upstream refuses: the forward fails")
    };
}

async fn succeed_once(w: &World) {
    let q = query(2, "crates.io.", RecordType::A, None);
    let ForwardOutcome::Relayed(_) = forward(w, &q, DnsTransport::Udp).await else {
        panic!("relayed")
    };
}

fn flapping_world() -> World {
    world(
        vec![UP1],
        vec![(
            UP1,
            vec![
                Step::Refused,
                Step::Respond(Reply::a(&[[52, 85, 1, 1]])),
                Step::Refused,
                Step::Respond(Reply::a(&[[52, 85, 1, 1]])),
            ],
        )],
    )
}

/// Contract shape: bounded-change. A failure episode is one warn at its start
/// however many queries fail, and one warn when it clears once an accepted
/// reply arrives at least `FORWARD_BUDGET` after the last failure; a later
/// failure starts a new episode.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn a_failure_episode_warns_once_at_its_start_and_once_when_it_clears() {
    run(async {
        let (events, _guard) = capture();
        let w = world(
            vec![UP1],
            vec![(
                UP1,
                vec![
                    Step::Refused,
                    Step::Refused,
                    Step::Refused,
                    Step::Respond(Reply::a(&[[52, 85, 1, 1]])),
                    Step::Refused,
                ],
            )],
        );
        for _ in 0..3 {
            fail_once(&w).await;
        }
        assert_eq!(events.count("dns.forward.failed"), 1, "one start for three failed queries");
        assert_eq!(events.count("dns.forward.recovered"), 0);

        w.clock.tick(FORWARD_BUDGET);
        succeed_once(&w).await;
        assert_eq!(
            events.count("dns.forward.recovered"),
            1,
            "cleared once the budget has passed since the last failure"
        );

        fail_once(&w).await;
        assert_eq!(events.count("dns.forward.failed"), 2, "a new episode starts after the clear");
    });
}

/// Contract shape: bounded-change. Boundary scenario 7: an accepted reply
/// EARLIER than `FORWARD_BUDGET` after the last failure neither clears nor
/// changes the flag, so a failure right after it is still the one episode.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_early_accepted_reply_leaves_the_failure_episode_open() {
    run(async {
        let (events, _guard) = capture();
        let w = flapping_world();
        fail_once(&w).await;
        w.clock.tick(FORWARD_BUDGET.saturating_sub(Duration::from_millis(1)));
        succeed_once(&w).await;
        fail_once(&w).await;
        assert_eq!(events.count("dns.forward.failed"), 1, "still one episode: no second start");
        assert_eq!(events.count("dns.forward.recovered"), 0, "and no clear");
    });
}

/// Contract shape: bounded-change. Boundary scenario 3 (resolver file absent):
/// a missing source starts the same episode, with no boot involvement.
#[test]
#[should_panic(expected = "forwarder: exchange the query verbatim")]
fn an_absent_resolver_file_starts_the_same_failure_episode() {
    run(async {
        let (events, _guard) = capture();
        let w = world(vec![UP1], vec![(UP1, vec![Step::Respond(Reply::a(&[[52, 85, 1, 1]]))])]);
        w.resolvers.set(Config::Absent);
        for _ in 0..3 {
            let q = query(3, "crates.io.", RecordType::A, None);
            let ForwardOutcome::Failed { .. } = forward(&w, &q, DnsTransport::Udp).await else {
                panic!("failed")
            };
        }
        assert_eq!(events.count("dns.forward.failed"), 1);
        assert_eq!(events.warns(), vec!["dns.forward.failed".to_owned()], "nothing else warns");
    });
}

// ---------------------------------------------------------------------------
// Permits and the shed (boundary scenario 4 and 6; ADR-0362)
// ---------------------------------------------------------------------------

proptest! {
    /// Contract shape: pure-function. ADR-0362: the in-flight bound is
    /// `(soft - reserve) / FDS_PER_PERMIT`, saturating at zero, for every
    /// limit and reserve.
    #[test]
    #[should_panic(expected = "permit arithmetic")]
    fn the_permit_count_is_the_descriptor_budget_over_the_cost_of_one_forward(
        soft in 0u64..=4_000_000, reserve in 0u64..=4_000_000,
    ) {
        let expected = usize::try_from(soft.saturating_sub(reserve) / FDS_PER_PERMIT).unwrap();
        prop_assert_eq!(forward_permits(soft, reserve), expected);
    }
}

/// Contract shape: pure-function. The design's worked arithmetic: reserve
/// 380 928 gives 333 824 permits at a 1 048 576 limit and 71 680 at 524 288;
/// a limit at or below the reserve gives none.
#[test]
#[should_panic(expected = "permit arithmetic")]
fn the_designs_worked_permit_examples_hold() {
    assert_eq!(forward_permits(1_048_576, 380_928), 333_824);
    assert_eq!(forward_permits(524_288, 380_928), 71_680);
    assert_eq!(forward_permits(380_928, 380_928), 0);
    assert_eq!(forward_permits(1_000, 380_928), 0);
}

/// Contract shape: bounded-change. ADR-0362: exactly `permits` forwards are in
/// flight; the next is shed with no wait, and a released permit is reusable.
#[test]
#[should_panic(expected = "the 4th concurrent forward must be shed")]
fn forwards_beyond_the_permit_count_are_shed_and_a_released_permit_is_reusable() {
    let w = world_with(vec![UP1], vec![], 3);
    let held: Vec<_> =
        (0..3).map(|_| w.forwarder.try_acquire().expect("within the bound")).collect();
    assert!(w.forwarder.try_acquire().is_none(), "the 4th concurrent forward must be shed");
    drop(held);
    assert!(w.forwarder.try_acquire().is_some(), "released permits are reusable");
}

/// Contract shape: bounded-change. Boundary scenario 4: a shed opens no
/// upstream socket and reads no upstream configuration.
#[test]
#[should_panic(expected = "the shed attempt must be refused")]
fn a_shed_query_touches_neither_the_upstream_nor_the_resolver_file() {
    let w = world_with(vec![UP1], vec![], 1);
    let _held = w.forwarder.try_acquire().expect("one permit");
    for _ in 0..10 {
        assert!(w.forwarder.try_acquire().is_none(), "the shed attempt must be refused");
    }
    assert!(w.exchange.calls().is_empty());
    assert_eq!(w.resolvers.reads(), 0);
}

/// Contract shape: bounded-change. Boundary scenarios 4 and 6: one
/// `shed_started` for a whole shedding run; release-and-reacquire at saturation
/// clears nothing; the clear comes only when a permit is acquired with at most
/// half of the permits held by others; a second saturation is a new episode.
#[test]
#[should_panic(expected = "the 5th concurrent forward must be shed")]
fn a_shed_episode_warns_once_and_clears_only_when_the_load_has_drained_to_half() {
    let (events, _guard) = capture();
    let w = world_with(vec![UP1], vec![], 4);
    let mut held: Vec<_> = (0..4).map(|_| w.forwarder.try_acquire().expect("within")).collect();
    for _ in 0..3 {
        assert!(w.forwarder.try_acquire().is_none(), "the 5th concurrent forward must be shed");
    }
    assert_eq!(events.count("dns.forward.shed_started"), 1, "one start for the whole run");

    // Release one and re-acquire it, repeatedly: 3 held before each acquire (6 > 4).
    for _ in 0..3 {
        drop(held.pop());
        held.push(w.forwarder.try_acquire().expect("a permit was just released"));
    }
    assert_eq!(
        events.count("dns.forward.shed_cleared"),
        0,
        "release and acquire at saturation clears nothing"
    );
    assert!(w.forwarder.try_acquire().is_none());
    assert_eq!(
        events.count("dns.forward.shed_started"),
        1,
        "the flag was left set: no second start"
    );

    // Drain to two held, then acquire with exactly half held by others (2 * 2 <= 4).
    drop(held.split_off(2));
    held.push(w.forwarder.try_acquire().expect("two of four are free"));
    assert_eq!(events.count("dns.forward.shed_cleared"), 1, "cleared at the half threshold");

    // Saturate again: a new episode.
    // Three of four are held after the clearing acquire; one more saturates.
    held.push(w.forwarder.try_acquire().expect("the fourth permit is free"));
    assert!(w.forwarder.try_acquire().is_none(), "saturated again: the shed starts a new episode");
    assert_eq!(
        events.count("dns.forward.shed_started"),
        2,
        "a second saturation starts a new episode"
    );
}

/// Contract shape: bounded-change. Boundary scenario 4, the `permits = 1` case:
/// with one permit the held count before the acquire is 0, so a single-permit
/// bound still clears.
#[test]
#[should_panic(expected = "the second concurrent forward must be shed")]
fn a_single_permit_bound_still_clears_its_shed_episode() {
    let (events, _guard) = capture();
    let w = world_with(vec![UP1], vec![], 1);
    let held = w.forwarder.try_acquire().expect("the one permit");
    assert!(w.forwarder.try_acquire().is_none(), "the second concurrent forward must be shed");
    assert_eq!(events.count("dns.forward.shed_started"), 1);
    drop(held);
    let _again = w.forwarder.try_acquire().expect("free again");
    assert_eq!(events.count("dns.forward.shed_cleared"), 1);
}

/// Contract shape: bounded-change. Ruling G: shedding warns twice per episode
/// and never once per shed query.
#[test]
#[should_panic(expected = "a shed run must warn exactly once")]
fn shedding_a_thousand_queries_warns_once() {
    let (events, _guard) = capture();
    let w = world_with(vec![UP1], vec![], 2);
    let _a = w.forwarder.try_acquire().expect("1");
    let _b = w.forwarder.try_acquire().expect("2");
    for _ in 0..1000 {
        let _ = w.forwarder.try_acquire();
    }
    assert_eq!(events.warns().len(), 1, "a shed run must warn exactly once: {:?}", events.warns());
}

/// Contract shape: bounded-change. ADR-0362: the in-flight bound holds under
/// contention. Eight threads race `try_acquire` / drop on a three-permit
/// forwarder; the number of permits held at once, tracked by an atomic
/// high-water mark taken while the permit is held, never exceeds three, and
/// once every thread is done every permit is back: exactly three can be taken
/// and the fourth is shed. Default lane: real threads, no socket, no clock read.
#[test]
#[should_panic(expected = "permit bound violated")]
fn racing_acquirers_never_hold_more_permits_than_the_bound_and_every_permit_is_released() {
    const BOUND: usize = 3;
    const THREADS: usize = 8;
    const ROUNDS: usize = 500;
    let w = world_with(vec![UP1], vec![], BOUND);
    let held = AtomicUsize::new(0);
    let high_water = AtomicUsize::new(0);
    let granted = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|| {
                for _ in 0..ROUNDS {
                    let Some(permit) = w.forwarder.try_acquire() else { continue };
                    let now = held.fetch_add(1, Ordering::SeqCst) + 1;
                    high_water.fetch_max(now, Ordering::SeqCst);
                    granted.fetch_add(1, Ordering::SeqCst);
                    // Hold the permit long enough for the other threads to contend.
                    for _ in 0..64 {
                        std::hint::spin_loop();
                    }
                    std::thread::yield_now();
                    held.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                }
            });
        }
    });
    assert!(granted.load(Ordering::SeqCst) > 0, "no thread was ever granted a permit");
    let peak = high_water.load(Ordering::SeqCst);
    assert!(peak <= BOUND, "permit bound violated: {peak} held at once, bound {BOUND}");
    let held_again: Vec<_> = (0..BOUND)
        .map(|n| {
            w.forwarder.try_acquire().unwrap_or_else(|| {
                panic!("permit bound violated: permit {n} of {BOUND} was never released")
            })
        })
        .collect();
    assert!(
        w.forwarder.try_acquire().is_none(),
        "permit bound violated: a permit beyond the bound of {BOUND} was granted",
    );
    drop(held_again);
}

// ---------------------------------------------------------------------------
// The guard over arbitrary answer sets (AC-6, AC-7; ADR-0359)
// ---------------------------------------------------------------------------

fn independently_owned(a: Ipv4Addr) -> bool {
    let o = a.octets();
    (o[0] == 10 && o[1] == 98) // mesh frontend 10.98.0.0/16
        || (o[0] == 100 && o[1] == 95) // guest prefix 100.95.0.0/16
        || (o[0] == 10 && o[1] == 200 && o[2] == 0) // the test's VIP pool 10.200.0.0/24
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Contract shape: bounded-change over a pure decision. For ANY answer set
    /// of A records: the reply is blocked, and carries no address at all, iff at
    /// least one address is owned; otherwise it is relayed byte for byte. A
    /// public address beside an owned one (US-GPD-2 scenario 3) is one of the
    /// generated sets: the whole reply is emptied.
    #[test]
    #[should_panic(expected = "forwarder: exchange the query verbatim")]
    fn a_reply_is_blocked_whole_iff_any_answer_address_is_owned(
        raw in proptest::collection::vec(
            prop_oneof![
                any::<u32>(),
                // Bias into and around the owned ranges.
                (0u32..=0x0001_ffff).prop_map(|n| u32::from(Ipv4Addr::new(10, 98, 0, 0)).wrapping_sub(0x8000).wrapping_add(n)),
                (0u32..=0x0001_ffff).prop_map(|n| u32::from(Ipv4Addr::new(100, 95, 0, 0)).wrapping_sub(0x8000).wrapping_add(n)),
                (0u32..=0x1ff).prop_map(|n| u32::from(Ipv4Addr::new(10, 200, 0, 0)).wrapping_sub(0x80).wrapping_add(n)),
            ],
            1..6,
        ),
    ) {
        let addrs: Vec<Ipv4Addr> = raw.into_iter().map(Ipv4Addr::from).collect();
        let reply = Reply { answers: addrs.iter().map(|a| RData::A(A::from(*a))).collect(), ..Reply::default() };
        let q = query(3, "any.corp.example.", RecordType::A, None);
        let w = world(vec![UP1], vec![(UP1, vec![Step::Respond(reply.clone())])]);
        let outcome = run(async { forward(&w, &q, DnsTransport::Udp).await });
        if addrs.iter().any(|a| independently_owned(*a)) {
            let ForwardOutcome::OwnedRangeBlocked { reply: bytes, address, .. } = outcome else {
                return Err(TestCaseError::fail("an answer set with an owned address must be blocked"));
            };
            prop_assert!(independently_owned(address));
            prop_assert!(answer_addrs(&decode(&bytes)).is_empty(), "no address, owned or not, is delivered");
        } else {
            let ForwardOutcome::Relayed(bytes) = outcome else {
                return Err(TestCaseError::fail("an answer set with no owned address must be relayed"));
            };
            prop_assert_eq!(bytes, reply.render(&q));
        }
    }
}
