# RED Classification — unconnected-udp-sendmsg4

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Wave:** DISTILL (this) → DELIVER (next)

> Per `.claude/rules/testing.md` § "RED scaffolds": this project's RED
> convention makes the bar **green under nextest by construction** — a
> `#[should_panic(expected = "RED scaffold")]` test PASSES, and the scaffold
> IS the specification. "RED" here means "the behavior is unimplemented and
> the test/invariant will flip to a real assertion in DELIVER", NOT "the test
> binary is red". DELIVER reads this file at its RED-phase entry gate to
> confirm each scaffold fails/asserts for the right reason once unskipped.

## Tier-1 DST invariant (default lane — the load-bearing reply-path defense)

| Test / invariant | File | RED mechanism | Right-reason classification |
|---|---|---|---|
| `reply-source-rewrite-lockstep` | `crates/overdrive-sim/src/invariants/reply_source_rewrite_lockstep.rs` | Evaluator body is `todo!("RED scaffold: …")` (panic), gated `#[expect(clippy::todo, clippy::unused_async)]`. The downstream full-invariant-walk tests carry `#[should_panic(expected = "RED scaffold")]`. | **MISSING_FUNCTIONALITY.** The evaluator's GREEN target (in its fn docstring) drives `register_local_backend` then asserts `reply_source_for(BackendKey) == Some(vip)` plus the S-02-02 forward-only-mutation asymmetry. Today the body `todo!()`s with the "RED scaffold" message because `SimDataplane::register_local_backend` does not yet write the reply mirror. Flips GREEN the moment DELIVER adds the mirror write (Slice 01/02): the `todo!()` is replaced by the real assertions and the `#[should_panic]` attributes are removed in the same commit. This is the structural defense BELOW Tier-3 (no Tier-2 backstop). |

Note: the invariant is in `Invariant::ALL`. A panic-based RED scaffold — NOT
an `InvariantResult::Fail` — is the convention-correct shape per
`.claude/rules/testing.md` § "RED scaffolds" + § "Downstream fallout on
pre-existing tests": returning `InvariantResult::Fail` reds the green bar and
forces every full-invariant-walk test (`run_boots_…`,
`default_harness_run_passes_…`, `full_default_catalogue_…`,
`harness_run_is_deterministic_under_fixed_seed`) to fail, which the project
convention forbids (the bar stays green; lefthook passes without
`--no-verify`). Those four walk tests are therefore armed with
`#[should_panic(expected = "RED scaffold")]` until the GREEN write lands; the
attribute self-trips (a different panic / no panic) the moment DELIVER unskips,
flagging each test for review at the GREEN transition. The
`invariant_roundtrip.rs` proptest covers the `FromStr`↔`Display` roundtrip
automatically (iterates `ALL`) and does NOT exercise the panicking evaluator.

The integration-tests-gated, Lima-only subprocess DST tests
(`dst_clean_clone_green.rs`, `dst_harness_smoke.rs`, `dst_seeded_reproduction.rs`)
assert the `dst` binary exits green on the full catalogue. They were already
RED-pending-GREEN before this slice (the invariant is in `Invariant::ALL` but
absent from their `EXPECTED_INVARIANTS` lists, and the harness was not green),
and DELIVER unskips them when the reply-mirror write lands. They are outside
the default nextest lane the pre-commit gate runs.

## Tier-3 acceptance (integration lane, Lima — THE gate; no Tier-2 backstop)

| Test | File | RED mechanism | Right-reason classification |
|---|---|---|---|
| S-01-01 WS round-trip | `unconnected_udp_roundtrip.rs::unconnected_sendto_recvfrom_reads_vip_sourced_reply` | `#[should_panic(expected = "RED scaffold")]` | **MISSING_FUNCTIONALITY.** Body panics naming the blocker (sendmsg4+recvmsg4 + dual-write). Compiles (no dependency on unimplemented `EbpfDataplane` methods). |
| S-01-02 both maps present | `…::forward_and_reverse_map_entries_present_after_one_register` | `#[should_panic]` | MISSING_FUNCTIONALITY (REVERSE_LOCAL_MAP handle + `reverse_local_map_entries`). |
| S-01-03 stateless reuse | `…::second_unconnected_query_reuses_same_mapping_statelessly` | `#[should_panic]` | MISSING_FUNCTIONALITY (Slice 01 round-trip). |
| S-02-03 Tier-3 meets Tier-1 | `…::kernel_reply_source_meets_tier1_reply_mirror_at_backend_identity` | `#[should_panic]` | MISSING_FUNCTIONALITY (reply rewrite + reply mirror, Slice 01+02). |
| S-03-01 no-op-on-miss | `unconnected_udp_reply_hardening.rs::non_service_unconnected_udp_reads_real_source_recvmsg4_noop_on_miss` | `#[should_panic]` | MISSING_FUNCTIONALITY (recvmsg4 hit-rewrite + no-op-miss branch + miss counter). Three assertions: non-service UDP reads real source (no-op); service reply always hits → VIP; miss counter inert. App-sockaddr, not wire. Corrected per CA-3 / UI-1 (was sentinel-on-miss). |
| S-03-02 below-floor refusal | `…::below_floor_kernel_refuses_at_attach_preflight_observably` | `#[should_panic]` | MISSING_FUNCTIONALITY (probe attach both hooks + typed `DataplaneBootError` variants). |
| S-03-03 fixture collision | `…::stub_resolver_binds_off_5353_and_asserts_clean_bind` | `#[should_panic]` | MISSING_FUNCTIONALITY (Tier-3 stub-resolver fixture). |

All seven are RED-pending-real-kernel: they compile and pass under nextest by
construction (the `#[should_panic]` convention), and DELIVER will unskip them
(swap each `panic!` for the real `EbpfDataplane`-driven assertion) one slice at
a time. They are gated behind the `integration-tests` feature and run only via
`cargo xtask lima run --` (Lima-only).

## Production-side RED scaffolds (so imports resolve — Mandate 7)

| Module | File | RED mechanism |
|---|---|---|
| `REVERSE_LOCAL_MAP` kernel map | `crates/overdrive-bpf/src/maps/reverse_local_map.rs` | `__SCAFFOLD__` — `#[map]` attribute ABSENT (kernel-side RED convention); POD key struct is real. |
| `REVERSE_LOCAL_MISS_COUNTER` | `crates/overdrive-bpf/src/maps/reverse_local_miss_counter.rs` | `__SCAFFOLD__` — `#[map]` attribute absent. |
| `build_local_service_key` helper | `crates/overdrive-bpf/src/shared/build_local_service_key.rs` | `__SCAFFOLD__` — body `todo!("RED scaffold: …")` + `#[expect(clippy::todo, …)]`. |
| `cgroup_sendmsg4_service` program | `crates/overdrive-bpf/src/programs/cgroup_sendmsg4_service.rs` | `__SCAFFOLD__` — `#[cgroup_sock_addr(sendmsg4)]` attribute absent (kernel-side RED); returns the non-denying default verdict 1. |
| `cgroup_recvmsg4_service` program | `crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs` | `__SCAFFOLD__` — `#[cgroup_sock_addr(recvmsg4)]` attribute absent; returns 1 (the only verifier-legal verdict). |
| `ReverseLocalMapHandle` userspace handle | `crates/overdrive-dataplane/src/maps/reverse_local_map_handle.rs` | `__SCAFFOLD__` — method bodies `todo!("RED scaffold: …")` + `#[expect(clippy::todo, …)]`. |
| Sim reply mirror + `reply_source_for()` | `crates/overdrive-sim/src/adapters/dataplane.rs` | Field + accessor REAL; the reply-mirror WRITE in `register_local_backend` is the GREEN target (left as a commented scaffold so existing forward-path tests stay green; the RED signal is carried by the Tier-1 invariant, not a panicking write). |

Production-side scaffolds use the project convention — `__SCAFFOLD__` marker,
`todo!("RED scaffold: …")` + `#[expect(clippy::todo)]` for fillable bodies, and
attribute-absence for kernel-side `#[map]`/`#[cgroup_sock_addr]` (panic cannot
expand under the `loop {}` panic_handler). Discoverable via
`grep -rn '__SCAFFOLD__\|RED scaffold' crates/overdrive-bpf crates/overdrive-dataplane crates/overdrive-sim`.

## Deferred to DELIVER (not scaffolded here — would shape production prematurely)

- `EbpfDataplane::register_local_backend` dual-write + `reverse_local_map_entries`
  accessor + probe attach of both hooks + `DataplaneError`/`DataplaneBootError`
  `#[from]` variants (`CgroupSendRecvAttach`, `ReverseLocalProbe`). These are
  EXTEND-on-shipped-code (the host adapter); DISTILL scaffolds the NEW modules
  the tests import, and the Tier-3 `#[should_panic]` tests name these as their
  blockers without depending on them at compile time (keeping the bar RED-not-
  BROKEN). DELIVER lands them slice by slice.
- `cgroup_connect4_service` helper-refactor (EXTEND — D4/CA-1). Behavior-
  preserving; re-verified by re-running the shipped `local_backend_proto_connect.rs`
  Tier-3 acceptance against the helper-backed connect4. No new DISTILL scaffold —
  the existing acceptance IS the regression gate (the one item DISCUSS called
  "UNCHANGED", now EXTEND).
