# Walking Skeleton — phase-1-control-plane-core

**Strategy**: C (real local adapters; no paid externals; no mocks). See
`wave-decisions.md` DWD-01.

> **Amendment 2026-04-26.** `overdrive cluster init` was removed from
> Phase 1 in commit `d294fb8`. The walking-skeleton sequence is now
> `serve` → `job submit` → `alloc status` (the dual-tempdir / Phase-0
> `cluster init` step is gone). `serve` is the sole Phase 1
> CA-minting site; it writes the trust triple to `~/.overdrive/config`
> on every start. The verb returns in Phase 5 with the persistent CA
> + operator-cert ceremony per ADR-0010 §Amendment 2026-04-26 and
> GH #81. RCA:
> `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`.
>
> **Amendment 2026-04-26 (ADR-0020) — `commit_index` dropped from
> Phase 1.** WS-1 / WS-2 / WS-3 are revised. WS-1's per-write witness
> is the spec-digest round-trip (no `commit_index` printed by the
> CLI; the submit output prints `spec_digest` + `outcome`). WS-2's
> wiring witness is `broker.dispatched > 0` plus the `reconcilers`
> registry list (the cluster-status output narrows from five lines to
> four — no Commit-index line). WS-3's idempotency witness is
> `outcome == "unchanged"` + `spec_digest` equality (no
> `commit index 17` magic-number). The demo script's step-2 line
> ("the commit index, and a 'Next' line") and step-5 line ("the
> server returns the same commit index — byte-identical content is
> idempotent") are revised. The traceability section's
> `shared-artifacts-registry.md` reference to `commit_index` is
> dropped. Source: ADR-0020;
> `docs/feature/redesign-drop-commit-index/design/wave-decisions.md`.

## User goal the skeleton proves

Ana, the Overdrive platform engineer, can clone the repository onto a
laptop, start the control plane, submit a real job spec through the
CLI, observe the reconciler primitive registered and idle, and read
back the spec digest byte-identical to what she can compute locally
from the same TOML file. That is the §J-OPS-002 claim in one engineer
session: *"Submit a job and trust what the CLI tells me."*

## Demo script for a non-technical stakeholder

1. "Ana has a fresh clone and a TOML file with her payment service's
   job description. She runs `overdrive serve` in one terminal. The
   control plane mints a fresh trust triple, writes it to
   `~/.overdrive/config`, and starts listening on the engineer's
   laptop, over TLS, on the default address. (No separate
   `cluster init` step in Phase 1 — that verb returns in Phase 5
   with the persistent-CA + operator-cert ceremony it actually
   needs. See ADR-0010 §Amendment 2026-04-26.)"
2. "In a second terminal, Ana runs `overdrive job submit payments.toml`.
   The CLI prints the job ID, the canonical intent key, the spec
   digest, the outcome (`created` for first submit), and a 'Next'
   line pointing her at the status command. ~~the commit index, and a 'Next' line pointing her at the status command.~~ (Revised 2026-04-26 (ADR-0020) — no commit index in Phase 1; spec digest + outcome replace it on the per-write surface.)"
3. "Ana runs `overdrive alloc status --job payments`. The CLI prints
   a spec digest — a short hash — that exactly matches what she can
   compute locally from the same TOML file. The CLI also tells her
   honestly that zero allocations are placed, because the scheduler
   lands in the next feature."
4. "Ana runs `overdrive cluster status`. The CLI lists the
   reconciler primitive registered at boot — `noop-heartbeat` — and
   shows the evaluation broker's counters."
5. "Ana changes a whitespace character in `payments.toml` that doesn't
   affect the semantic content. She resubmits. The server returns
   the same spec digest with `outcome: unchanged` — byte-identical
   content is idempotent. ~~the same commit index — byte-identical content is idempotent.~~ (Revised 2026-04-26 (ADR-0020) — no commit index; the witness is the typed outcome enum plus the digest equality.) She changes a real field. She resubmits. The server returns a conflict error — same key, different spec, different story."

Every noun in the demo script names an observable operator outcome.
The stakeholder can confirm "yes, that is what engineers need" without
reading any Rust.

## The walking-skeleton scenarios

### WS-1 — Clean-clone end-to-end submit round-trip

`test-scenarios.md` §1.1. Tags:
`@walking_skeleton @real-io @adapter-integration @driving_adapter @us-01 @us-02 @us-03 @us-05 @journey:submit-a-job @kpi K1`.

Enters through the `overdrive` CLI subprocess. The scenario:

1. Starts `overdrive serve` as a child process pointed at a scratch
   `tempfile::TempDir`. `serve` mints the CA + trust triple
   in-process and writes the triple to the configured
   `<dir>/.overdrive/config` (ADR-0010 §R1 as amended 2026-04-26 —
   `serve` is the sole Phase 1 minter). Waits for the HTTPS listener
   to be ready on `127.0.0.1:7001`.
2. Runs `overdrive job submit payments.toml`. Asserts exit 0, that
   stdout names the Job ID, the canonical intent key, the spec
   digest, and the outcome (`created`), and ends with a "Next" line
   pointing at `alloc status`. ~~the Job ID, the canonical intent key, and the commit index, and ends with a "Next" line pointing at `alloc status`.~~ (Revised 2026-04-26 (ADR-0020).)
3. Runs `overdrive alloc status --job payments`. Asserts the printed
   spec digest equals the digest Ana can compute locally via
   `ContentHash::of(rkyv_archived_bytes)`.
4. Stops the server cleanly via SIGINT. Asserts in-flight drain.

Exercises every adapter named in DWD-09: `rcgen`, `rustls`, `axum`,
`reqwest`, `LocalStore` (real redb in a TempDir), `SimObservationStore`
(as the Phase 1 production impl — ADR-0012), `libsql` (provisioned by
the reconciler runtime for `noop-heartbeat`).

### WS-2 — Reconciler primitive alive and observable

§1.2. Tags:
`@walking_skeleton @real-io @adapter-integration @driving_adapter @us-04 @us-05 @journey:submit-a-job @kpi K4 @kpi K5`.

Starts the control plane, runs `overdrive cluster status`, asserts:

- Exit 0.
- The reconcilers section lists `noop-heartbeat`.
- The broker counters (queued / cancelled / dispatched) render as
  non-negative integers.
- The mode is reported as `single`, the region is the default. ~~and the commit_index equals whatever LocalStore is at.~~ (Revised 2026-04-26 (ADR-0020) — no commit_index field on the cluster-status response; the wiring witness for "the reconciler primitive ran" is `broker.dispatched > 0` after the broker has had a tick to drain. Cluster-status output is four fields total: `{mode, region, reconcilers, broker}`.)
- The output does not contain a Commit-index line.

This is the operator-visible proof that the §18 primitive is wired.
WS-1 creates the IntentStore state and tests round-trip; WS-2 tests
that the reconciler layer is alive and visible to the operator.

### WS-3 — Byte-identical re-submit is idempotent; different spec at same key is a conflict

§1.3. Tags:
`@walking_skeleton @real-io @adapter-integration @driving_adapter @error-path @us-03 @us-05 @journey:submit-a-job @kpi K1 @kpi K6`.

Two submits with the same spec bytes → both return 200 OK with
`outcome == "unchanged"` (on the second) and the same `spec_digest`
(on both). ~~commit_index (200 OK both times).~~ Then a submit with a
*different* spec at the *same* intent key → 409 Conflict with an
actionable error body. Then a submit of the original spec again →
still 200 OK with `outcome == "unchanged"` and the same
`spec_digest` (idempotency does not depend on intervening conflicts).
~~original commit_index (idempotency does not depend on intervening conflicts).~~ (Revised 2026-04-26 (ADR-0020) — no commit index; the witness is the typed outcome enum + digest equality.)

This is the scenario that proves the error-path claim: operators can
re-submit safely, and the server distinguishes "same thing again"
from "you're overwriting someone else's work."

## Why three walking skeletons, not one

Three WS scenarios cover three distinct engineer outcomes:

- **WS-1** — "the platform takes my input and gives it back exactly" —
  the round-trip hypothesis from the whitepaper §4 claim.
- **WS-2** — "the §18 reconciler primitive is visible and alive" —
  the storm-proof-reconciler hypothesis.
- **WS-3** — "the platform distinguishes safe retry from real
  conflict" — the operator-hygiene hypothesis.

Consolidating into one would violate bdd-methodology Rule 1
("one scenario, one behavior"). A green-only WS would skip the
conflict/idempotency behaviour that is an ADR-0015 load-bearing
decision. WS-3 without the earlier two would be unanchored — there
would be no previously-committed spec to test conflict against.

## Strategy-C litmus test

> "If I deleted the real local adapters, would these walking skeletons
> still pass?"

**Answer**: No.

- Delete `redb` → the LocalStore can't commit the submit → §1.1/§1.3
  fail at the submit step.
- Delete `rcgen` → `serve` can't mint the CA → every WS fails at the
  bootstrap step (per ADR-0010 §R1 as amended 2026-04-26, `serve` is
  the sole Phase 1 cert-minting site).
- Delete `axum`+`rustls` → the server never binds → every WS times
  out on the "wait for listener" step.
- Delete `reqwest` → the CLI can't call the server → every WS fails at
  the first subcommand invocation.
- Delete `libsql` → the reconciler runtime can't provision
  `noop-heartbeat`'s DB → the server fails to register the reconciler
  → WS-2 fails at the cluster-status assertion.

> "Could these walking skeletons pass without the `overdrive` CLI
> subprocess wrapper?"

**Answer**: No. All three WS enter through `overdrive` subprocess
invocations and assert on observable subprocess outcomes (exit code,
stdout format, stderr actionable messages). Calling the server's
`submit_job` handler directly would skip the CLI's spec-digest
computation, the endpoint-precedence logic (`--endpoint` flag >
`OVERDRIVE_ENDPOINT` env > default), and the actionable-error
rendering — the exact behaviours US-05 is designed to prove.

## What is NOT part of the walking skeleton

- **No scheduler**. `alloc status` shows an explicit empty state
  naming `phase-1-first-workload` as the next feature. Per whitepaper
  §3 and §4, placement lands with the next feature.
- **No real node agent / driver**. `node list` shows an explicit
  empty state. Same reason.
- **No Corrosion**. The `SimObservationStore` wrapped by
  `wire_single_node_observation` (ADR-0012) IS the Phase 1 production
  observation-store impl. Phase 2+ swaps in `CorrosionStore` via
  `Box<dyn ObservationStore>` with no handler changes.
- **No Raft**. `LocalStore` (single-mode, redb direct) is the Phase 1
  intent store. HA lands later per whitepaper §4.
- **No operator auth**. The endpoint accepts any connection that
  trusts the ephemeral CA. Operator SPIFFE IDs / RBAC land Phase 5
  per ADR-0010.
- **No ACME / public-trust certs**. Phase 3+ work per whitepaper §11.
- **No external HTTP services, paid APIs, cloud credentials**.
  Strategy C posture — every adapter is local.

## Traceability

- **Journey**: `docs/product/journeys/submit-a-job.yaml` (Steps 1–4).
- **Feature-level journey**:
  `docs/feature/phase-1-control-plane-core/discuss/journey-submit-a-job.yaml`.
- **User stories**: US-01 (aggregates), US-02 (REST surface), US-03
  (handlers), US-04 (reconciler primitive + cluster status), US-05
  (CLI handlers).
- **KPIs**: K1 (WS-1), K4 + K5 (WS-2), K1 + K6 (WS-3). K2/K3/K7
  covered by focused scenarios §4 and §6.
- **Shared artifacts**: `spec_digest`, ~~`commit_index`~~ (revised
  2026-04-26 (ADR-0020) — no commit_index in Phase 1; replaced by
  `outcome: IdempotencyOutcome` on `SubmitJobResponse`), `intent_key`,
  `rest_endpoint`, `openapi_schema` (see
  `discuss/shared-artifacts-registry.md`).
