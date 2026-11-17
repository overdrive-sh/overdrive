# DESIGN review (subtractive) — native-benchmark-program

Reviewer: nw-solution-architect-reviewer (Sonnet), 2026-11-17, per `.claude/rules/design.md` § "No unrequested scope". Reads DESIGN sections against `intake.md` rulings and #287 ACs. DDD-1..14 not argued away.

**Verdict: CHANGES_REQUESTED** — 3 MEDIUM, 5 LOW; all removals or narrowings inside approved decisions.

## Medium
- **S-1. Drop `expected_families` from `run_main` and the exit-70 refusal** (`feature-delta.md` DDD-3 rule 2). Rule 1 (`use <crate> as _;`) plus "no benchmark declares family X" for the selected `--family` already catches V-5's dropped-crate case; a hard-coded count must change per family (against K1).
- **S-2. Thread the observer through `DnsResponder::new` only**, not `GuestDnsDeps` / the factory (`GuestDnsDeps` is built at `lib.rs:1209,4674,4864`, `overdrive-sim/src/adapters/guest_dns.rs:191`, `shared_network_test_ports.rs`). `HostGuestDnsFactory` passes `NoQueryObserver`.
- **S-3. Cut `probe()` checks that duplicate the kernel or other checks:** `osrelease` vs `uname`; non-empty online CPU list; the `HostClock` monotonicity probe and its DDD-10 refusal; `StatsEnabler`'s "first read shows runs" (covered by the per-replicate `run_count > 0` failure). Keep absent-vs-unreadable in DDD-7 and `RecordSink` create-new.

## Low
- **S-4.** Remove `better = <smaller|larger>` (no consumer); trim the `unit` list to used units and add the missing `client_throughput_qps` unit; drop `statistic = <delta|median>` (only `delta` used).
- **S-5.** Drop the `--seed` override from the Metal form; keep the recorded, entropy-seeded seed.
- **S-6.** Record `debug_assertions` only; drop `opt_level`/`profile` (would need a `build.rs`); the closed `cargo bench` command already prevents a debug build.
- **S-7.** Remove recorded-only ASLR and `perf_event_paranoid` fields (no reader); keep clocksource and `CONFIG_IRQ_TIME_ACCOUNTING`.
- **S-8.** (a) drop the cgroup-scoped CPU figure from mTLS rows; (b) remove the `exec` refusal of `overdrive` (bypassable; the command log suffices for US-01(d)); (c) drop catalogue columns substrate/stats flag/caveats (record header carries them); (d) collapse the `CanaryWorkload` port into a pure function over samples.

## Retained after tracing
Mann-Whitney U; `bench-report validate/render`; the declaration echo in `run_started`; VM profiles and stage table; both mTLS directions and ingress "absent" rows; two crates, `linkme`, native-only registry, the `testing.md` exemption; `smoke(none)`.

## Resolution

**Resolved 2026-11-17.** All eight findings applied as removals or narrowings inside DDD-1..14; ADRs are **Accepted**.

| # | Change |
|---|---|
| S-1 | DDD-1 and DDD-6: `run_main(evidence)` takes no family count; DDD-3 rule 2 is the `--family`-not-declared refusal (`Error::FamilyNotDeclared`, exit 64) with the `use <crate> as _;` rule kept. ADR-0298. |
| S-2 | DDD-12: the observer enters through `DnsResponder::new` only; `GuestDnsDeps` and the factory are unchanged; `HostGuestDnsFactory` passes `NoQueryObserver`. ADR-0307. |
| S-3 | Components and ports (driven-port table): `osrelease` vs `uname`, non-empty CPU list, the `HostClock` probe, and `StatsEnabler`'s first-read check are cut; DDD-10 no longer refuses on a clock probe; Lifecycle Gate Ownership table drops the `HostClock` probe. ADR-0302 and ADR-0305. |
| S-4 | DDD-2 grammar: `better` removed; unit list is `ns, bytes, bytes_per_second, qps, ratio, count`; `statistic` removed from `reference` (a reference is compared with the metric's A/B delta). Connect-hook family text matches. |
| S-5 | DDD-5, DDD-9: no seed override in the runner or the Metal form; the entropy seed is recorded. ADR-0300, ADR-0304. |
| S-6 | DDD-6 / DDD-7: `debug_assertions` only. ADR-0301. |
| S-7 | DDD-7: ASLR and `perf_event_paranoid` removed from the recorded-only row; clocksource and `CONFIG_IRQ_TIME_ACCOUNTING` kept. ADR-0302. |
| S-8 | (a) DDD-13 and ADR-0308: no cgroup CPU figure; CA-9 records the US-BENCH-09 criterion change. (b) DDD-6 contract notes: `exec` no longer refuses `overdrive`; ruling 3 is enforced by the families' code and review, not a runtime guard. (c) DDD-6 retention: catalogue columns reduced to family, boundary and arms, workload matrix, clock boundary, result owner. (d) Components and ports: `CanaryWorkload` port removed; the canary verdict is a pure function over the start and end samples. |
