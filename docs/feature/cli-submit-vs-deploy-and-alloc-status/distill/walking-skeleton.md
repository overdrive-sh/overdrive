# Walking Skeleton — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISTILL
**Status**: WAIVED.

---

## Decision

DISCUSS [D8] and DESIGN [C7] explicitly **waived** the formal walking
skeleton for this feature. This document records the rationale and
the structural-end-to-end coverage that nonetheless ships in its
place.

---

## Rationale

This feature is a brownfield extension. The end-to-end already exists:

- The inner-loop `overdrive job submit` verb already commits intent
  and returns a one-line JSON ack (shipped in `phase-1-control-plane-core`).
- The lifecycle reconciler already runs and converges allocations
  (shipped in `phase-1-first-workload`).
- The `ExecDriver` already starts real workloads against real cgroups
  (shipped in `phase-1-first-workload`).
- The `alloc status` command already returns an
  `AllocStatusResponse`, even if today's content is sparse.

There is no thinnest-vertical-slice to ship because the slice
already ships. The remaining work is **enriching observation surfaces
that already exist** (snapshot field expansion, streaming wire
shape).

---

## What ships in place of a formal WS

The driving-adapter verification mandate (Mandate 1 / Quinn's gate
under `nw-distill`) requires at least one Tier-3 scenario per driving
adapter that invokes the system through the operator's real
protocol. This feature has two driving adapters:

1. **CLI subprocess** (`overdrive job submit`, `overdrive alloc
   status`).
2. **HTTP API** (`POST /v1/jobs` with content negotiation;
   `GET /v1/allocs?job=<id>`).

Both are exercised end-to-end by two Tier-3 scenarios in
`test-scenarios.md`:

- **`S-WS-01` — Operator submits a healthy spec and the verb tells
  the truth on success.** Covers the happy path through both driving
  adapters: real `overdrive job submit` subprocess against a real
  spawned control plane; real reqwest streaming over
  `application/x-ndjson`; real exit code; real summary line; real
  `LocalIntentStore`, `LocalObservationStore`, `ExecDriver`.

- **`S-WS-02` — Operator submits a broken-binary spec and the verb
  names the cause (REGRESSION TARGET).** Covers the failure path
  through both driving adapters: real subprocess invocation pointing
  at a non-existent binary; real ENOENT; real reconciler restart
  budget exhaustion; real exit-code-1; real `Error:` block; second
  subprocess invocation of `overdrive alloc status` against the
  same observation store; assertion that the failure reason is
  byte-equal across both surfaces.

These scenarios carry the conventional walking-skeleton tag triple
`@walking_skeleton @driving_adapter @real-io` so the catalogue audit
picks them up.

---

## Demo-ability

`S-WS-02` is the demo-able scenario for stakeholders. The literal
session a stakeholder watches:

```
$ overdrive job submit ./payments.toml
Accepted (spec_digest sha256:..., outcome inserted)
Pending → Pending  reason: scheduling
Pending → Failed   reason: driver start failed
                   detail: stat /usr/local/bin/no-such-binary: no such file or directory
... [4 more retry transitions] ...
Pending → Failed   reason: driver start failed
                   detail: stat /usr/local/bin/no-such-binary: no such file or directory

Error: job 'payments-v2' did not converge to running.
  reason: driver start failed (binary not found)
  last-event: stat /usr/local/bin/no-such-binary: no such file or directory
  reproducer: overdrive alloc status --job payments-v2

Hint: fix the spec's `exec.command` path and re-run.
$ echo $?
1

$ overdrive alloc status --job payments-v2
Job:         payments-v2
Spec digest: sha256:7f3a9b12...
Replicas:    1 desired / 0 running

ALLOC ID   STATE     RESOURCES        STARTED  EXIT
a1b2c3     Failed    100mCPU/256 MiB  -        -

Last transition: 2026-04-30T10:18:22Z
  Pending → Failed    reason: driver start failed
  source:  driver(exec)
  error:   stat /usr/local/bin/no-such-binary: no such file or directory
Restart budget: 5 / 5 used (backoff exhausted)
```

This is the "told the truth" moment the journey YAML's emotional arc
calls out. A stakeholder can confirm "yes, that is what an operator
needs" without reading any code.

---

## What this is NOT

- This is NOT a walking skeleton that ships in DELIVER as the first
  enabled test. The crafter enables scenarios in slice order (slice
  01 first, then slice 02, then slice 03 conditionally). `S-WS-01`
  and `S-WS-02` enable when slice 02 enables (they require streaming
  + snapshot together to fully exercise).
- This is NOT a substitute for the per-slice acceptance scenarios
  that exercise individual handler paths at Tier 1. The bulk of the
  catalogue is Tier 1; `S-WS-*` are the structural-end-to-end
  bookends.

---

## References

- `discuss/wave-decisions.md` [D8] — WS waiver rationale.
- `design/wave-decisions.md` [C7] — carryover.
- `distill/test-scenarios.md` § 3.1 — full `S-WS-01` and `S-WS-02`
  specifications.
- `distill/wave-decisions.md` DWD-01 — driving-adapter verification
  fulfils the structural intent.
