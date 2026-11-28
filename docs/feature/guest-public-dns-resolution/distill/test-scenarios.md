# Test scenarios — `guest-public-dns-resolution` (GH #382)

**Wave:** DISTILL | **Designer:** Quinn | **Date:** 2026-11-27

No `.feature` files (`.claude/rules/testing.md`): this document is the
GIVEN/WHEN/THEN specification; the executable bodies are Rust tests, already
authored in full. **Every pending scenario is `#[should_panic(expected = ...)]`
over a complete body**, so it runs on every run and fails today for the reason
it asserts. DELIVER removes the marker one scenario at a time, runs the
pre-authored body, and confirms the semantic RED before changing production
code. Four scenarios characterise behaviour this feature keeps and are active (W-3, W-6, W-14, G-4).

## Totals

| Tier / lane | Tests | Pending (RED) | Active (GREEN) | Placement |
|---|---|---|---|---|
| Tier 1, default lane: `Forwarder` contract (`dns_forwarder.rs`) | 33 | 33 | 0 | `placement!(host_pure)` (in-process, `SimClock`, scripted ports, no socket) |
| Tier 1, default lane: owned-range list (`dns_owned_ranges.rs`) | 4 | 4 | 0 | `placement!(host_pure)` |
| Tier 3, real wire: resolver file adapter (`dns_upstream_host.rs`) | 4 | 4 | 0 | `#[overdrive_test(host_pure)]` (tempdir file, no node-global effect) |
| Tier 3, real wire: upstream exchange (`dns_upstream_host.rs`) | 4 | 4 | 0 | `guest = "cgroup-default"` |
| Tier 3, real wire: responder on UDP/TCP 53 (`dns_forwarding_real_wire.rs`) | 16 | 13 | 3 | `guest = "cgroup-default"` |
| Tier 3, guest through `serve` + `deploy` (`guest_public_dns.rs`) | 4 | 3 | 1 | `guest = "cgroup-default", profile = "nested"` |
| Tier 3, `serve` boot: forward permit derivation (`dns_forward_permit_derivation.rs`) | 1 | 1 | 0 | `guest = "cgroup-default"` (boots `run_server` in-process) |
| **New total** | **66** | **62** | **4** | |

Error, fault, edge and guard scenarios: **50 of 66 (76%)**; happy path and pure
arithmetic/containment: 16. Existing `dns_responder_bind.rs` tests are
re-run as the regression set for the responder; one fixture line changed
(`GuestDnsDeps` gained two fields), no behaviour.

Paths: `CP-A` = `crates/overdrive-control-plane/tests/acceptance/`, `CP-I` =
`crates/overdrive-control-plane/tests/integration/`, `CLI-I` =
`crates/overdrive-cli/tests/integration/`.

## Walking skeleton

**US-GPD-1 walking skeleton** — `CLI-I/guest_public_dns.rs::a_vm_guest_resolves_a_public_name_through_its_default_resolver_and_connects`
(pending, RED proven on metal).

- GIVEN `overdrive serve` on a node whose host resolver file names a scripted upstream that answers `github.com` with the address of a route-disjoint remote peer
- AND `overdrive deploy` of a VM Service whose program calls `getaddrinfo("github.com")` through its default resolver and connects to the answer
- WHEN the guest runs
- THEN the Job ends `Terminated`; the console shows the single nameserver `100.95.0.1`, an `addr=` lookup under 1000 ms, and the byte-exact exchange; the peer served the request once; the upstream saw one query for `github.com`.
- Observed RED (metal run `f0db8cd7505ada0a31fcbbff`, 7.0.0-38): `resolve: nameserver=100.95.0.1`, `resolve: github.com error=EAI_AGAIN ms=5006`, state `Failed` where `Terminated` is asserted. It is the issue's own symptom.

Litmus: the title is Ana's goal; the oracle is what the operator and the guest
console show, never an internal counter; nothing is installed in the guest.

## Scenarios

Columns: **Owner path** is the production entry exercised. **Fault** is the
injected stimulus (none for a healthy control). **Oracle** is the observable
outcome. Contract shape is `bounded-change` unless stated.

### A. Tier 3 guest through `serve` + `deploy` (US-GPD-1..3)

| ID | Scenario | Owner path | Fault | Oracle | Covers | Test |
|---|---|---|---|---|---|---|
| G-1 | WS: guest resolves `github.com` through its default resolver and connects | `serve` → `deploy` → guest `getaddrinfo` → UDP 53 to gateway | none (healthy control) | Job `Terminated`; console nameserver, `addr=`, `ms < 1000`, byte-exact exchange; peer once; upstream once | AC-1, AC-4, K-GPD-1; boundary 1 | `a_vm_guest_resolves_a_public_name_through_its_default_resolver_and_connects` |
| G-2 | The dev box's four usual names resolve; a nonexistent name fails with "Name does not resolve" | same | upstream answers four names, not the fifth | each usual name `addr=` under 1000 ms; missing name `EAI_NONAME` under 1000 ms | AC-1, AC-3, K-GPD-1, K-GPD-2 | `the_dev_boxs_usual_names_resolve_and_a_nonexistent_name_fails_definitely_and_promptly` |
| G-3 | An upstream answer in the mesh frontend range never reaches the guest; a private answer outside owned ranges is delivered | same | upstream answers `10.98.0.7` and `10.20.4.7` | blocked name: no `10.98.0.7`, definite result under 1000 ms; benign name `addr=10.20.4.7` | AC-6, AC-7, AC-9, K-GPD-4 | `an_upstream_answer_in_the_mesh_frontend_range_never_reaches_the_guest` |
| G-4 | With the upstream unreachable the guest lookup ends with a definite error (**active**; no timing bound, see note) | same | upstream gone: forward times out | Job `Terminated`; `error=` result, never an address | AC-10 (guest side only), K-GPD-5 held by W-8 and V-4 | `with_the_upstream_unreachable_the_guest_lookup_ends_with_a_definite_error` |

All four: CLI-I/guest_public_dns.rs, tier 3 guest (`nested`); G-1 to G-3 pending, G-4 active.

**G-4 note (no `ms < 5000` bound).** The guest-observed time of a failed lookup depends on the guest resolver library's SERVFAIL handling. The guest program is built against musl, whose `res_msend` accepts only NOERROR and NXDOMAIN as answers: it resends a SERVFAIL a bounded number of times, then ignores it and keeps waiting, so `getaddrinfo` returns `EAI_AGAIN` at its own timeout whatever the node does (the same result as today). glibc retries a SERVFAIL once. This is recalled from musl's source, not read from a vendored copy, so it is not asserted as a fact of the test; the test asserts only what holds under either library, a definite error and a terminated Job, and is therefore active (it characterises behaviour the feature keeps). The node's contract, a definite SERVFAIL inside the 4000 ms budget, is asserted at reply level by W-8 and F-8 (AC-10). DESIGN V-4 measures the guest-observed time under glibc and musl at DELIVER; if V-4 shows musl returns before 5000 ms with a SERVFAIL, a timing bound can be restored then.
Tests never spawn the `overdrive` binary; `serve` and `deploy` run in-process
through the same driving ports the binary uses.

### B. Tier 3 real wire: responder on UDP/TCP 53 and the host resolver file

The production responder (`HostGuestDnsFactory::host()`) is driven over real
sockets inside a guest group; the upstream is `overdrive_testing::dns_upstream`
named by the host resolver file. All in `CP-I/dns_forwarding_real_wire.rs`.

| ID | Scenario | Fault | Oracle | Covers | Status | Test |
|---|---|---|---|---|---|---|
| W-1 | A gateway-addressed public lookup is forwarded and answered | none | NOERROR with the scripted address | AC-1; boundary 1 | pending | `a_gateway_addressed_public_lookup_is_forwarded_and_answered` |
| W-2 | A nonexistent public name gets NXDOMAIN promptly | upstream NXDOMAIN | NXDOMAIN, definite | AC-3, K-GPD-2 | pending | `a_nonexistent_public_name_gets_nxdomain_promptly` |
| W-3 | A query not addressed to the gateway is dropped and never forwarded | datagram to a non-gateway node address | no reply; upstream sees nothing | open-resolver gate (V-2) | **active** | `a_query_not_addressed_to_the_gateway_is_dropped_and_never_forwarded` |
| W-4 | A platform-zone name that is not a mesh name is NXDOMAIN and never forwarded | `a.b.svc.overdrive.local`, `overdrive.local` apex | NXDOMAIN; upstream sees nothing | AC-5, D-6, ADR-0357 | pending | `a_platform_zone_name_that_is_not_a_mesh_name_is_nxdomain_and_never_forwarded` |
| W-5 | A name that only resembles the zone is forwarded | `evil-overdrive.local` style | forwarded and answered | ADR-0357 (labels, not text) | pending | `a_name_that_only_resembles_the_platform_zone_is_forwarded` |
| W-6 | Mesh names resolve locally and never reach the upstream | healthy, unknown and unhealthy-backend mesh names | stable frontend; NXDOMAIN for the unknown and for the name with no healthy backend; upstream sees nothing | AC-2, D-6, K-GPD-3 | **active** | `mesh_names_still_resolve_locally_and_never_reach_the_upstream` |
| W-7 | An upstream answer in the mesh frontend range still gets a definite reply | upstream answers `10.98.0.x` | emptied reply (no address), rcode kept | AC-6, AC-9 | pending | `an_upstream_answer_in_the_mesh_frontend_range_never_reaches_the_guest` |
| W-8 | With the upstream down the guest gets SERVFAIL before its timeout, mesh keeps answering, then it recovers with no restart | upstream gone (its address lands on the node, which drops it), then back | three SERVFAILs each under the guest timeout with one `dns.forward.failed` for the three; mesh frontend returned during the outage; next lookup answered | AC-10, AC-11, AC-12; boundary 3 | pending | `with_the_upstream_down_the_guest_gets_a_servfail_before_its_timeout_and_recovers` |
| W-9 | An absent resolver file is SERVFAIL, not a boot refusal; recovers when restored | resolver file removed, restored | boot proceeds; SERVFAIL; later answered | ruling B; boundary 3 | pending | `an_absent_resolver_file_is_a_servfail_not_a_boot_refusal_and_recovers_when_restored` |
| W-10 | A resolver file naming only the gateway never loops the forward to the node | file lists `100.95.0.1` only | SERVFAIL; no datagram looped | ADR-0358 loop exclusion | pending | `a_resolver_file_naming_only_the_gateway_never_loops_the_forward_to_the_node` |
| W-11 | A query over TCP is forwarded over TCP and answered, and the node keeps serving the connection until the client closes (revised 2026-11-28) | none | framed reply to the first query; the upstream saw one TCP exchange; a second query on the same connection is answered; the client then closes | GPD-5, ADR-0360 and its 2026-11-28 amendment | pending | `a_query_over_tcp_is_forwarded_over_tcp_and_answered_until_the_client_closes` |
| W-12 | The probe refuses when the gateway TCP port is held | TCP port 53 on the gateway already bound | typed `Bind` refusal | boundary 2, gate G-1 | pending | `the_probe_refuses_when_the_gateway_tcp_port_is_held` |
| W-13 | With every permit held a further UDP query is shed at once, a TCP connection is closed unread, mesh over UDP still answers; one `shed_started` | permits held by silent-upstream lookups | SERVFAIL under 500 ms; TCP EOF; mesh frontend returned; one warn | AC-11; boundary 4 | pending | `with_every_permit_held_a_further_query_is_shed_at_once_and_mesh_still_answers` |
| W-14 | A gateway-addressed datagram that is not one IN-class query is dropped and never forwarded | QR set, two questions, opcode STATUS, class CH | no reply to any; upstream sees nothing | DESIGN `WireReject`; open-resolver/reflection guard | **active** | `a_gateway_addressed_datagram_that_is_not_one_in_class_query_is_dropped_and_never_forwarded` |
| W-15 | A TCP connection with malformed framing is closed without a reply | zero length frame; frame over 1500 bytes | closed promptly with no bytes; upstream sees nothing | DESIGN TCP lifecycle (zero length closes without reply; frame over the 1500 byte limit closes) | pending | `a_tcp_connection_with_malformed_framing_is_closed_without_a_reply` |
| W-16 | Two pipelined queries in one write are both answered in order on one connection (added 2026-11-28) | one A and one AAAA query for the same name, framed and written in ONE write as glibc's dual-family lookup does, then the client half-closes its write side | two framed replies, ids and questions matching, in order, on the one connection, received after the half-close; two upstream TCP exchanges | ADR-0360 2026-11-28 amendment; spike `findings-tcp-one-query-per-connection.md` | pending | `two_pipelined_queries_in_one_write_are_both_answered_in_order_on_one_connection` |

W-11 and W-16 are the two test changes the user approved on 2026-11-28 with the ADR-0360 TCP amendment (queries are served until the client closes): W-11 was revised from "one answer, then the connection closes", W-16 is the added regression. Both are pending with `#[should_panic(expected = ...)]` naming the assertion that fails today (W-11: "a second query on the same connection must be answered"; W-16: "both pipelined queries must be answered on the one connection"); the half-close invariant is folded into W-16. The other amendment invariants are assertable through them and add no test. Metal run `d168584d675b01e1f7726e8c`: all 16 tests of the file pass (the two pending ones for the asserted reason). W-15 is unchanged.

The third TCP clause of the DESIGN lifecycle contract (an expired `TCP_READ_DEADLINE` of 5 s closes without a reply) is not given its own scenario: it costs 5 s of wall time per run and shares the same close path as the two framing cases. W-14 is active because today's responder drops every non-mesh datagram; it becomes the guard that the forwarder never answers or relays what is not a query.

### C. Tier 3 real wire: driven adapters (`CP-I/dns_upstream_host.rs`)

| ID | Scenario | Fault | Oracle | Placement | Status | Test |
|---|---|---|---|---|---|---|
| U-1 | Nameserver lines listed in file order; comments, options, scoped and malformed tokens skipped | scoped `fe80::1%eth0`, `not-an-address`, empty `nameserver` | ordered `IpAddr` list (only what DESIGN pins: order, plain addresses, skips; duplicate lines and keyword case are unspecified and not asserted) | host_pure | pending | `nameserver_lines_are_listed_in_file_order_and_everything_else_is_ignored` |
| U-2 | A readable file with no nameserver line is an empty list, not an error | empty file | `Ok(vec![])` | host_pure | pending | `a_readable_file_with_no_nameserver_line_is_an_empty_list_not_an_error` |
| U-3 | A missing file is `Absent`; an unreadable one is a distinct `Unreadable` | missing; a directory in the file's place (not EACCES: tests run as root, which reads any file) | two distinct variants | host_pure | pending | `a_missing_file_is_absent_and_an_unreadable_one_is_a_distinct_error` |
| U-4 | The file is re-read on every call | file replaced between calls | second call reflects the new file | host_pure | pending | `the_file_is_re_read_on_every_call_so_a_replaced_file_takes_effect_at_once` |
| U-5 | A UDP exchange returns the matching reply and ignores a wrong id and a wrong question | spoofed replies | only the matching reply | guest | pending | `a_udp_exchange_returns_the_matching_reply_and_ignores_a_wrong_id_and_a_wrong_question` |
| U-6 | A silent upstream is `Timeout`; a closed port is `Unreachable`, never `Timeout` | silent; ICMP refusal | the two variants | guest | pending | `a_silent_upstream_is_a_timeout_and_a_closed_port_is_unreachable_not_a_timeout` |
| U-7 | A UDP reply that fills the receive buffer is `ReplyTooLarge` | 59 001-byte datagram | `ReplyTooLarge` | guest | pending | `a_udp_reply_that_fills_the_receive_buffer_is_reply_too_large` |
| U-8 | A TCP exchange frames one query per connection and rejects a mismatched reply | mismatched reply | reply or `Io` of kind `InvalidData` | guest | pending | `a_tcp_exchange_frames_one_query_per_connection_and_rejects_a_mismatched_reply` |

### D. Tier 1 default lane: the `Forwarder` contract (`CP-A/dns_forwarder.rs`)

The real `Forwarder` driven through `try_acquire` / `forward`, with scripted
`UpstreamResolvers` / `UpstreamExchange` and `SimClock`. Oracles: outcome, reply
bytes decoded by hickory, what the exchange was asked (server, transport,
timeout, query bytes), elapsed `SimClock` time, captured warn events. All
pending; all `host_pure`.

| ID | Scenario | Contract | Covers | Test |
|---|---|---|---|---|
| F-1 | A public lookup is sent unchanged; the reply is relayed byte for byte | ADR-0356 steps 3, 5 | AC-1, AC-3 | `a_public_lookup_is_sent_upstream_unchanged_and_the_upstream_reply_is_relayed_byte_for_byte` |
| F-2 | A reply of any rcode is final; no failover on an rcode | step 5 | AC-3 | `an_upstream_reply_of_any_rcode_is_final_and_never_triggers_failover` |
| F-3 | Every non-mesh record type is relayed unchanged (AAAA included) | ADR-0361 | GPD-6 | `every_non_mesh_record_type_is_relayed_unchanged` |
| F-4 | A DNSSEC-signed truncated reply keeps every bit (AD, RRSIG, TC) | ADR-0361, V-6 | V-6 | `a_dnssec_signed_truncated_reply_is_relayed_with_every_bit_preserved` |
| F-5 | A TCP guest query is forwarded over TCP | step 3 | GPD-5 | `a_query_that_arrived_over_tcp_is_forwarded_over_tcp` |
| F-6 | A silent first upstream fails over to the second after one attempt timeout (1500 ms) | steps 3, 7 | AC-10 | `a_silent_first_upstream_fails_over_to_the_second_after_one_attempt_timeout` |
| F-7 | A refusing upstream costs no time before the next is tried | `Unreachable` is not a wait | AC-10 | `a_refusing_upstream_costs_no_time_before_the_next_is_tried` |
| F-8 | Every upstream silent: SERVFAIL within the 4000 ms budget, `budget_exhausted` | steps 6, 7 | AC-10, K-GPD-5 | `with_every_upstream_silent_the_guest_gets_a_servfail_within_the_budget` |
| F-9 | Every upstream refusing: SERVFAIL at once, naming the last failure | step 6 | AC-10 | `with_every_upstream_refusing_the_servfail_is_immediate_and_names_the_last_failure` |
| F-10 | An undecodable reply is a failed attempt; the next upstream is tried | step 4 | ADR-0356 | `an_undecodable_reply_is_a_failed_attempt_and_the_next_upstream_is_tried` |
| F-11 | A reply too large for the buffer is a failed attempt with its own cause | `ReplyTooLarge` | ADR-0356 | `a_reply_too_large_for_the_buffer_is_a_failed_attempt_with_its_own_cause` |
| F-12 | (proptest) only the gateway, in IPv4 and mapped form, is excluded; file order kept | pure-function, ADR-0358 | loop exclusion | `only_the_gateway_is_excluded_and_file_order_is_kept` |
| F-13 | A list naming only the gateway is `NoUsableUpstream`; nothing is sent | ADR-0358 | loop exclusion | `a_resolver_list_naming_only_the_gateway_is_no_usable_upstream_and_nothing_is_sent` |
| F-14 | Absent, unreadable, empty source: SERVFAIL with a distinct cause; recovers when restored | ruling B | AC-12; boundary 3 | `an_unusable_resolver_source_is_a_servfail_with_a_distinct_cause_and_recovers_when_restored` |
| F-15 | An answer in the mesh frontend range is emptied whole and marked Blocked (EDE 15, OPT 1232, DO copied) | step 5 | AC-6, AC-9 | `an_upstream_answer_inside_the_mesh_frontend_range_is_emptied_and_marked_blocked` |
| F-16 | A blocked reply to a query without EDNS carries no OPT | step 5 | ADR-0359 | `a_blocked_reply_to_a_query_without_edns_carries_no_opt_record` |
| F-17 | An answer on a reserved VIP address or in the guest prefix is blocked, naming its range (the mesh frontend range is F-15; edges are O-3; random addresses F-33) | step 5 | AC-6, AC-8 | `an_answer_on_a_reserved_vip_or_in_the_guest_prefix_is_blocked_with_its_range` |
| F-18 | A private answer outside the owned ranges is delivered unchanged (`10.20.4.7` and `10.99.0.9`; range edges are O-3) | narrowness | AC-7, K-GPD-4 | `a_private_answer_outside_the_owned_ranges_is_delivered_unchanged` |
| F-19 | *removed*: one owned address in a mixed answer set empties the whole reply is a generated case of F-33 | | US-GPD-2 scenario 3 | covered by F-33 |
| F-20 | AAAA and additional-section addresses are outside the guard, by design | accepted non-coverage | ADR-0359 | `aaaa_and_additional_section_addresses_are_outside_the_guard_by_design` |
| F-21 | An HTTPS record whose `ipv4hint` is owned is blocked | step 5 | ADR-0359 | `an_https_record_whose_ipv4_hint_is_owned_is_blocked` |
| F-22 | A blocked answer is `OwnedRangeBlocked`, emits one `dns.forward.owned_range_blocked` debug event per query and never a warn | ruling C2 | GPD-7 | `blocked_answers_never_raise_a_warn` |
| F-23 | A failure episode warns once at its start and once when it clears; a later failure is a new episode | step 8 | boundary 3 | `a_failure_episode_warns_once_at_its_start_and_once_when_it_clears` |
| F-24 | An accepted reply earlier than `FORWARD_BUDGET` leaves the episode open; no second start | step 8, conditional clear | boundary 7 | `an_early_accepted_reply_leaves_the_failure_episode_open` |
| F-25 | An absent resolver file starts the same failure episode | step 8 | boundary 3 | `an_absent_resolver_file_starts_the_same_failure_episode` |
| F-26 | (proptest) permits = `(soft - reserve) / 2`, saturating at 0 | pure-function, ADR-0362 | GPD-8 | `the_permit_count_is_the_descriptor_budget_over_the_cost_of_one_forward` |
| F-27 | The design's worked arithmetic (333 824 and 71 680; 0 at or below reserve) | pure-function | GPD-8 | `the_designs_worked_permit_examples_hold` |
| F-28 | Forwards beyond the permit count are shed; a released permit is reusable | step 1, ADR-0362 | boundary 4 | `forwards_beyond_the_permit_count_are_shed_and_a_released_permit_is_reusable` |
| F-29 | A shed query touches neither the upstream nor the resolver file | step 1 | boundary 4 | `a_shed_query_touches_neither_the_upstream_nor_the_resolver_file` |
| F-30 | A shed episode warns once; clears only when load has drained to half; conditional clear leaves the flag set; re-saturation is a new episode (walked to GREEN: 4 permits, drain to 2, acquire clears, one more saturates) | step 9 | boundaries 4, 6 | `a_shed_episode_warns_once_and_clears_only_when_the_load_has_drained_to_half` |
| F-31 | A single-permit bound still clears its shed episode | step 9, `permits = 1` | boundary 4 | `a_single_permit_bound_still_clears_its_shed_episode` |
| F-32 | Shedding a thousand queries warns once | ruling G | boundary 4 | `shedding_a_thousand_queries_warns_once` |
| F-34 | Eight threads racing `try_acquire` / drop on a three-permit forwarder never hold more than three at once (atomic high-water mark), and every permit is released at the end: exactly three can be taken, the fourth is shed. Real threads, no socket; user-approved addition (ruling 2026-11-27) | bounded-change, ADR-0362 | GPD-8 | `racing_acquirers_never_hold_more_permits_than_the_bound_and_every_permit_is_released` |
| F-33 | (proptest) a reply is blocked whole iff any answer address is owned (mixed public and owned sets included); otherwise relayed byte for byte | bounded-change over a pure decision | AC-6, AC-7, US-GPD-2 scenario 3 | `a_reply_is_blocked_whole_iff_any_answer_address_is_owned` |

### E. Tier 1 default lane: the owned-range list (`CP-A/dns_owned_ranges.rs`)

| ID | Scenario | Contract | Covers | Test |
|---|---|---|---|---|
| O-1 | (proptest) every address in an owned range is owned and names its range | pure-function | AC-6, AC-8 | `every_address_in_an_owned_range_is_owned_and_names_its_range` |
| O-2 | (proptest) an address in no owned range is not owned | pure-function | AC-7 | `an_address_outside_every_owned_range_is_not_owned` |
| O-3 | The edges of each owned range are owned and the next address is not; `10.99.0.0/16` is not owned | pure-function | AC-6, AC-7 | `the_edges_of_each_owned_range_are_owned_and_the_next_address_is_not` |
| O-4 | Source scan: every address-range constant in production source is named in `OwnedAddressRanges::platform`'s construction or excluded on the record. Selection rule: a `const`/`static` typed `Ipv4Net` or `VipRange` in `crates/*/src/**/*.rs`, comments dropped, each file cut at its first `#[cfg(test)]`; the only exclusion is `WORKLOAD_SUBNET_BASE` (10.99.0.0/16, Grounding 7 / Ruling C), and a stale or contradictory exclusion fails. The VIP pool must arrive through `platform`'s `&VipRange` parameter and be used. Modelled on `required_serve_ports_source_scan.rs`; user-approved addition (ruling 2026-11-27). Red today: `platform` names no constant | pure-function (source text) | AC-6, AC-8; ADR-0359 | `every_address_range_constant_is_in_the_owned_list_or_excluded_on_the_record` |

### F. Tier 3 `serve` boot: forward permit derivation (`CP-I/dns_forward_permit_derivation.rs`)

| ID | Scenario | Contract | Covers | Test |
|---|---|---|---|---|
| P-1 | `serve` boots with a soft descriptor limit of 1,048,576 injected through the host bundle and reports the forward bound once: exactly one `dns.forward.permits` event (keyed on `meta.name()`), whose `reserve` equals `guest_flow_fd_requirement() + NODE_FD_RESERVE`, whose `nofile_soft` equals the injected 1,048,576 and whose `permits` equals `(nofile_soft - reserve) / FDS_PER_PERMIT` (saturating). Drives `run_server` through the shared non-VM host bundle (`tests/common/node_adapters.rs`); user-approved addition (ruling 2026-11-27). The expectation is recomputed in the test from the pinned items, not from `forward_permits` | pure-function | GPD-8, ADR-0362 | `boot_derives_the_forward_permit_bound_from_the_descriptor_limit` |

DELIVER note: the test relies on the common bundle injecting the sim value 1,048,576 as the fourth `NodeAdapters::host` argument (`node_adapters.rs` `host_bundle`, a listed call site); the assertion `nofile_soft == 1_048_576` fails if the helper injects anything else. The test file's own `INJECTED_NOFILE_SOFT` must then stay equal to it.

## Design boundary scenarios 1 to 7

| # | DESIGN boundary scenario | Covered by |
|---|---|---|
| 1 | gate available, boot proceeds | W-1, G-1; existing `dns_responder_bind.rs` set (active, 28-test run) |
| 2 | TCP port held, typed refusal | W-12 |
| 3 | resolver file absent: boot proceeds, SERVFAIL, one episode warn, recovery once an accepted reply arrives at least `FORWARD_BUDGET` after the last failure; a failure inside that interval leaves the episode open | W-9, F-14, F-23, F-24, F-25 |
| 4 | saturation: immediate SERVFAIL on UDP, TCP closed unread, no upstream socket or read, mesh over UDP answered, one `shed_started`, no clear at saturation, one clear at half, new episode on re-saturation | W-13, F-28, F-29, F-30, F-31, F-32 |
| 5 | feature disabled | Does not exist (the forwarder is composed unconditionally, ADR-0138); no scenario by design |
| 6 | conditional clear, shed episode | F-30 |
| 7 | conditional clear, failure episode | F-24 |

## Fault-injection record (`testing.md` § "DISTILL — prove composed system behavior")

| Contract / invariant | Production owner path | Fault stimulus | Outcome oracle | Tier |
|---|---|---|---|---|
| A public lookup gets a prompt, definite reply (AC-1, AC-3, AC-10) | guest → UDP 53 → responder → forwarder → upstream | upstream silent / refusing / NXDOMAIN / resolver file absent | SERVFAIL or relayed rcode inside the 4000 ms budget; guest console elapsed under 5000 ms | F-6..F-9, W-2, W-8, W-9, G-2 (1, 3) |
| No owned-range address reaches the guest (AC-6, AC-9) | forwarder guard | upstream answers `10.98.0.7`, VIP pool, `100.95.x`, mixed set, HTTPS hint | reply carries no address; definite result; benign private answers unchanged | F-15..F-18, F-20..F-22, F-33, O-1..O-3, W-7, G-3 (1, 3) |
| Mesh answers are unaffected by the upstream (AC-2, AC-11) | responder mesh arm | upstream down; every permit held; malformed name under the zone | mesh frontend returned; NXDOMAIN with nothing sent upstream | W-4, W-6, W-8, W-13 (3) |
| The forwarder never loops or answers a foreign datagram (open-resolver gate, V-2) | gateway gate; `UpstreamSet::select` | datagram to a non-gateway address; resolver list of the gateway only | no reply, upstream sees nothing; SERVFAIL, nothing sent | W-3, W-10, W-14, F-12, F-13 (1, 3) |
| In-flight work is bounded and shed honestly (GPD-8) | `try_acquire`, TCP accept | every permit held | immediate SERVFAIL on UDP; connection closed unread on TCP; one warn per episode | F-26..F-32, F-34, P-1, W-13, W-15 (1, 3) |
| Recovery needs no restart (AC-12) | resolver file read per query | file removed then restored; upstream gone then back | next lookup answered; one start and one clear | F-14, F-23, U-4, W-8, W-9 (1, 3) |

Bound of each experiment: a bounded budget (4000 ms) on `SimClock` in tier 1;
tier 3 holds permits with a silent scripted upstream for under 2 s and tears
the guest down. Cleanup is the guest's own disposal; no host state is touched.

## Domain language and shape notes

- Scenario names are domain sentences (a guest, a public name, a definite answer). Protocol and address terms appear only where the contract names them (SERVFAIL, NXDOMAIN, TCP): they are the user-observable vocabulary of DNS, not implementation internals.
- Driving ports only: `serve` + `deploy`, the guest resolver (UDP and TCP 53), `GuestDns::{probe, serve, stop}`, and `Forwarder::{try_acquire, forward}` (the one entry the responder loop uses). The wire codec and internal classifier are exercised only through them.
- Universe / state delta: tier 1 asserts the full observable surface per scenario (outcome, reply, calls the exchange received, warn events); `assert_state_delta` is a Python-pilot port and the repository's Rust equivalent is the explicit before/after on those port-exposed observables. Layers 3+ use traditional assertions (Mandates 8 and 11); sad paths are named examples. PBT appears only at tier 1 (F-12, F-26, F-33, O-1, O-2).
- Tier B: not declared. The journey is not a chained state machine over a rich input space, and the forwarder holds no shared state worth a model (DESIGN: Formal protocol model not applicable).
- Tracing events are keyed on the event's `meta.name()` in every test (`dns.forward.failed`, `.recovered`, `.shed_started`, `.shed_cleared`, `.owned_range_blocked`): production tracing macros MUST set `name:` explicitly for the `dns.forward.*` events (`tracing::warn!(name: "dns.forward.failed", ...)`), or the default `event file:line` name would never match.
- `HostUpstreamExchange` is used in the tests as a unit-struct value, a test-support use of the pinned type: DESIGN pins `ResolvConfUpstreams::from_path` and the `UpstreamExchange` port, not how `HostUpstreamExchange` is constructed. Note for DELIVER: if it needs state, the construction change routes back to the architect, not to an improvised constructor.
- The `dns.forward.owned_range_blocked` debug event is asserted at the forwarder (F-22): the event names the upstream, which only the forwarder knows.
- No cache, so no cache scenario. No allowlist, so none (#383).

## RED classification (pre-DELIVER gate)

Every pending test is `MISSING_FUNCTIONALITY`, none `IMPORT_ERROR`, `FIXTURE_BROKEN`
or `WRONG_ASSERTION`: each passes as a `should_panic` whose `expected` substring
is the failure the body produces today, so a panic from a fixture error would
fail the test.

| Group | Run | Result | Semantic RED |
|---|---|---|---|
| D + E (35, default lane) | `fb50c37a0613b080f2629e85` (earlier run `4b10317ee6173bf17568c122`, 36 before F-19 was removed) | 35 passed | the scaffold's `todo!` message from `forward` / `owning_range` / `forward_permits` (the body reached the production call), or the assertion text for the shed cases (`the 4th concurrent forward must be shed`, `a shed run must warn exactly once`) |
| F-34, O-4 (2, default lane) | `6287f4a5d7989b21f12d127f` | 2 passed | `permit bound violated` (the scaffold's `try_acquire` always grants, so the fourth permit is not shed) and `owned range missing from OwnedAddressRanges::platform` (the scaffold's `platform` names no constant) |
| P-1 (1, Tier 3 guest) | f2416a8f35e1d7effe93f718 | 1 passed | `serve emits exactly one dns.forward.permits event per boot` (the boot emits no such event today; `forward_permits` is the scaffold `0`) |
| B + C (30 incl. the 7 existing `dns_responder_bind`) | `e3ce508b20c17e855d135326` (earlier `efe84168a1d5ec4c550bdef8`) | 30 passed | `resolver file: re-read the file on every call`, `upstream exchange: one fresh socket per exchange`, and assertion texts such as `a gateway-addressed public lookup must be answered` (the responder drops non-mesh queries today) |
| A (4, Tier 3 nested guest) | `3569a6d3cc1eef699cfd9586` (earlier `61e06cdd4272cb5dba165ee5`) | 4 passed (G-1 to G-3 pending-RED, G-4 active) | G-1 to G-4 assertion texts; the underlying RED observed verbatim in `f0db8cd7505ada0a31fcbbff` (`EAI_AGAIN ms=5006`, `Failed` vs `Terminated`) |

Active scenarios W-3, W-6, W-14 and G-4 pass against today's responder (characterisation:
the gate, the mesh arm, the non-query drop and the guest's definite error stay as they are).

Every long pending body was walked to its GREEN end against the pinned contract (F-23, F-24, F-28, F-30, W-1, W-8, W-13, G-1 to G-3). One defect was found and fixed: F-30 took a second extra permit at the end of the re-saturation step and would have panicked at saturation; it now takes one and the next `try_acquire` starts the second episode. The signed reply of F-4 was checked to decode with hickory in a throwaway test (removed).

Note for DELIVER: the tier-1 and `dns_upstream_host` REDs name the scaffold's
`todo!` message. Removing the marker after the body is implemented is the
signal; if the first activated run fails on a different message the fixture, not
the production code, is wrong, and that routes back to the acceptance designer.
