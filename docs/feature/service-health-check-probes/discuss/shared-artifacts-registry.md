# Shared Artifacts Registry — service-health-check-probes

Every variable appearing in TUI mockups or wire events has a documented source of truth and explicit consumers. Drift between these is the primary integration failure mode and is enforced by tests defined in DISTILL.

## Registry

```yaml
shared_artifacts:

  probe_idx:
    source_of_truth: >
      Position of the probe entry in the TOML `[[health_check.{role}]]`
      array, 0-indexed. For inferred default probes (Service has no
      [[health_check.startup]] declared), the inferred probe takes
      probe_idx = 0.
    consumers:
      - ProbeResultRow primary key (alloc_id, probe_idx)
      - Streaming progress line: "Probing startup [<mechanic>] attempt N/M, last: ..."
      - CLI `alloc status` Probes section row label (#0, #1, ...)
      - ServiceSubmitEvent::Stable.witness payload (which probe witnessed Stable)
      - ServiceSubmitEvent::Failed.reason::StartupProbeFailed { probe_idx, ... }
    owner: TOML parser in `overdrive-core` (assigns probe_idx at parse time)
    integration_risk: HIGH
    validation: >
      Acceptance test: submit a Service with two startup probes, observe
      both ProbeResultRows, observe CLI render showing #0 and #1
      consistently across `alloc status` and any failure event.

  settled_in:
    source_of_truth: >
      Computed by ServiceLifecycleReconciler at the deciding tick:
      `tick.now - AllocStatusRow.started_at`. Single computation point;
      no derived caching.
    consumers:
      - ServiceSubmitEvent::Stable.settled_in (Duration on the wire)
      - CLI render: "Service '<id>' is stable\n  settled_in: <duration>"
    owner: ServiceLifecycleReconciler.reconcile()
    integration_risk: MEDIUM
    validation: >
      Acceptance test: submit Service with workload that takes ~1.5s to
      become probe-passing; assert `settled_in` parses as a Duration in
      range [1000ms, 3000ms] (not the literal "live" sentinel).

  witness:
    source_of_truth: >
      ServiceLifecycleReconciler's deciding tick: the (probe_idx, role)
      tuple of the probe whose Pass result moved the reconciler from
      "waiting" to "Stable". For multi-probe startup, witness names the
      LAST probe that crossed its threshold.
    consumers:
      - ServiceSubmitEvent::Stable.witness wire payload
      - CLI render: "  witness: startup probe #0 (tcp 127.0.0.1:8080)"
    owner: ServiceLifecycleReconciler.reconcile()
    integration_risk: MEDIUM
    validation: >
      Acceptance test: assert witness.probe_idx resolves to a real entry
      in the spec's [[health_check.startup]] array or equals 0 for the
      inferred default.

  last_observed_at:
    source_of_truth: >
      Wall-clock instant of the ProbeRunner tick that produced the most
      recent ProbeResultRow for `(alloc_id, probe_idx)`. Recorded once;
      LWW update on the same row PK.
    consumers:
      - CLI `alloc status` Probes section: "at <timestamp>"
    owner: ProbeRunner (worker-side)
    integration_risk: LOW

  last_fail_reason:
    source_of_truth: >
      String captured by ProbeRunner when a probe attempt fails. HTTP
      probe failures format as "HTTP <code>" or "connection refused" or
      "timeout after <duration>"; TCP failures as "connection refused"
      or "timeout"; Exec failures as "exit <code>" or "exec: <error>".
    consumers:
      - ProbeResultRow.last_fail_reason field
      - CLI `alloc status` Probes section last_fail column
      - ServiceSubmitEvent::Failed.reason::StartupProbeFailed.last_fail
    owner: ProbeRunner (worker-side)
    integration_risk: HIGH
    validation: >
      The reason string must be stable enough for operator to ACT.
      Acceptance test: submit Service whose listener never binds;
      assert last_fail_reason == "connection refused" (not "io error"
      or some generic shape).

  startup_deadline:
    source_of_truth: >
      Computed from TOML `[[health_check.startup]].timeout_seconds ×
      .max_attempts` summed across all startup probes; if no explicit
      startup probes, defaults to 60 seconds (platform default for
      inferred TCP probe).
    consumers:
      - ServiceLifecycleReconciler (decides when "Failed" arm fires)
      - CLI render: "elapsed: 60.0s (startup_deadline=60s)"
    owner: ServiceSpec validator in `overdrive-core`
    integration_risk: MEDIUM
    validation: >
      Acceptance test: declare a startup probe with timeout=5s,
      max_attempts=10 → startup_deadline = 50s; assert reconciler emits
      Failed at 50s ± 1 tick, not 60s.

  terminal_condition_bytes:
    source_of_truth: >
      ADR-0037 §3 guarantee: TerminalCondition value is constructed
      ONCE by ServiceLifecycleReconciler.reconcile(), passed in the
      Action, and the action shim writes it to BOTH AllocStatusRow.terminal
      AND LifecycleEvent.terminal. Byte-equal across both surfaces.
    consumers:
      - AllocStatusRow.terminal (durable; ObservationStore)
      - LifecycleEvent.terminal (broadcast bus; streaming subscriber)
      - HTTP alloc_status snapshot handler (reads row)
      - NDJSON streaming subscriber (reads event)
    owner: action_shim crate (single write site for both surfaces)
    integration_risk: HIGH
    validation: >
      Acceptance test: submit Service that reaches Stable; query HTTP
      alloc_status AND read the NDJSON stream's last event; assert the
      TerminalCondition serialised bytes are byte-equal.

  probes_section_present:
    source_of_truth: >
      Compute-on-render predicate: `kind == Service AND probes_present`,
      where `probes_present` means at least one declared OR inferred
      probe.
    consumers:
      - CLI `alloc status` render
    owner: crates/overdrive-cli/src/render.rs (Service-kind handler)
    integration_risk: LOW
    validation: >
      Acceptance test pair: (a) Service with probes → section present;
      (b) Job with terminal Completed → section absent.

  default_probe_descriptor:
    source_of_truth: >
      ServiceSpec validator in `overdrive-core`: when ServiceSpec has
      zero [[health_check.startup]] entries, synthesises a single
      ProbeDescriptor { mechanic: Tcp { host: "0.0.0.0", port:
      listeners[0].port }, timeout: 5s, interval: 2s, max_attempts: 30 }
      with probe_idx = 0.
    consumers:
      - ServiceSpec post-validation aggregate (single source for the
        rest of the system; reconciler and ProbeRunner read from here)
      - CLI streaming progress line "(probe inferred: tcp 0.0.0.0:<port>)"
    owner: ServiceSpec validator
    integration_risk: HIGH
    validation: >
      Acceptance test: submit Service with no probes; assert exactly
      one ProbeResultRow gets written with probe_idx=0 and mechanic
      matching the listener; assert renderer marks it as "inferred".
```

## Cross-step consistency contracts

1. **probe_idx is a closed integer across steps 1, 2, 4.** Parse-time assignment IS the source; everywhere else reads. If a future change introduces a `probe_id: String` field, the closed integer must coexist or every consumer's display logic shifts.
2. **terminal byte-equality is structural (ADR-0037 §3).** Both consumer surfaces (snapshot + streaming) read action-derived state; the action shim is the SINGLE write site. The validation acceptance test catches drift the moment it happens.
3. **default_probe_descriptor is computed once.** ServiceSpec validation is the single point; ProbeRunner and ServiceLifecycleReconciler read from the validated aggregate, never re-infer.

## CLI vocabulary consistency

| Term | Definition | Used in |
|---|---|---|
| probe | A single declared or inferred health check (one entry in `[[health_check.{role}]]`) | TOML key, CLI render, ADR text |
| role | One of `startup` / `readiness` / `liveness` | TOML section name, ProbeResultRow field, CLI render |
| mechanic | One of `http` / `tcp` / `exec` | TOML probe-body field, CLI render summary |
| Stable | Terminal condition: Service has reached operator-meaningful liveness (startup probes pass) | TerminalCondition variant, ServiceSubmitEvent variant, CLI render |
| StartupProbeFailed | Failure reason: startup probe never passed within deadline | ServiceSubmitEvent::Failed.reason variant, CLI render |
| EarlyExit | Failure reason: workload exited before any startup probe could pass | ServiceSubmitEvent::Failed.reason variant, CLI render |
| settled_in | Real Duration from `started_at` to deciding-tick wall-clock | ServiceSubmitEvent::Stable field, CLI render |
| witness | The probe (probe_idx + role) whose Pass moved the reconciler to Stable | ServiceSubmitEvent::Stable field, CLI render |

No `"live"` literal in any operator-facing string (RCA-A solution D).
