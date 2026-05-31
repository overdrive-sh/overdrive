# ADR-0058 — Default-probe inference: probe-less Service inherits a TCP-connect startup probe against `listener[0]`; "honest by default" divergence from K8s/Nomad

## Status

Accepted. 2026-05-24. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

Tags: phase-1, service-kind, operator-config, default-policy,
rca-a-closure.

**Companion ADRs**: ADR-0054 (ProbeRunner), ADR-0055
(ServiceLifecycleReconciler), ADR-0057 (TOML spec). **Closes**: RCA
root cause A from
`docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`
for the most common operator workflow (no probes declared).

## Context

The RCA root cause A states: *kernel-accepted exec is NOT
operator-meaningful liveness*. The DISCUSS wave decision (US-01,
walking skeleton Slice 01) is to **infer a default TCP-connect
startup probe** when the operator declares no `[[health_check.*]]`
sections.

This diverges from Kubernetes and Nomad, both of which default to
"no probe" semantics — a container is considered Ready as soon as it
starts. The K8s rationale is "we cannot know what 'ready' means for
your application"; Overdrive's rationale is "kernel-accepted exec
has been demonstrated to be a worthless signal; the platform must
do better by default."

This ADR pins the inference rule and the SemVer surface.

## Decision

### 1. Inference rule (parser-side, in `ServiceSpec` validation)

```
IF parsed_service_spec.health_check.startup is ABSENT
   AND parsed_service_spec.listeners is non-empty
THEN
   service_spec.startup_probes = vec![ProbeDescriptor {
       idx: ProbeIdx(0),
       role: ProbeRole::Startup,
       mechanic: ProbeMechanic::Tcp {
           addr: SocketAddrV4::new(
               Ipv4Addr::new(0, 0, 0, 0),
               listeners[0].port,
           ),
       },
       timeout: Duration::from_secs(5),
       interval: Duration::from_secs(2),
       max_attempts: Some(30),       // → startup_deadline = 60s
       failure_threshold: None,
       success_threshold: None,
       inferred: true,
   }];
ELSE IF parsed_service_spec.health_check.startup is PRESENT BUT EMPTY (`[[health_check.startup]] = []`)
   service_spec.startup_probes = vec![];   // explicit opt-out
ELSE
   service_spec.startup_probes = vec![<parsed probes per ADR-0057>];
```

The inference target is **the first listener** by parse order
(`listeners[0]`). For multi-listener Services, the operator is
strongly encouraged to declare explicit probes; the first-listener
heuristic is a "best guess" intentionally narrow.

### 2. Inferred descriptor flag — `inferred: bool`

Per `feature-delta.md` shared-artifacts-registry,
`ProbeDescriptor.inferred: bool` is `true` for the synthesised
default; `false` for every parser-derived descriptor. The flag
flows through to:

- `ProbeWitness.inferred` on the wire (operator sees `(inferred)`
  marker in CLI render).
- `ProbeResultRow.mechanic` is identical to explicit-TCP probes;
  the `inferred` flag is carried separately on the descriptor and
  re-derived on render.
- `alloc status` Probes section per US-06 marks the inferred probe
  explicitly (`startup #0 (tcp 0.0.0.0:8080) (inferred)`).

The `inferred` flag is **NOT used by the reconciler for any
decision** — the inferred probe and an explicitly-declared
equivalent TCP probe produce identical reconciler behaviour. The
flag is for operator visibility only.

### 3. SemVer surface — the default IS a contract

Once shipped, the inference rule's behaviour becomes operator-relied-on:

- An operator submits a Service with no probes; expects Stable based
  on listener-bind.
- A future Phase 2+ change to "infer HTTP `/health` probe instead of
  TCP" silently changes the success criterion for every
  no-probe-declared Service.

Per the project's additive-only schema-migration discipline, the
inference rule is **frozen** as TCP-connect-on-`listener[0]`. Future
defaults (e.g. HTTPS-bound listener inference, multi-listener
fan-out) require:

- An operator-configurable knob (e.g.
  `[platform.defaults].startup_probe_inference = "tcp" | "http" | "none"`).
- The knob defaults to `"tcp"` to preserve current behaviour.

This rule is documented in operator changelog and pinned by an
integration test that asserts "submitted Service with no probes
gets exactly one TCP-connect probe against listener[0]".

### 4. Why not align with K8s/Nomad "no default" semantic?

| Argument for K8s-style | Argument against (chosen) |
|---|---|
| "We can't know what ready means for the operator." | The RCA proves the platform's current "ready = kernel accepted exec" claim is wrong. Doing nothing is not neutral; it's the bug. |
| "Operators can declare probes if they want." | Operators DON'T declare probes for fast iteration (US-01 problem statement). The platform's default must work for the iteration loop. |
| "Inference is magic; magic is bad." | Inference is OPT-OUT-able (`[[health_check.startup]] = []`); the inference rule is one paragraph in operator docs; the `(inferred)` marker makes it visible everywhere. |

The "honest by default" choice IS the structural fix for RCA-A.
Operators who genuinely want first-Running-IS-Stable semantics opt
out explicitly via the empty array. The default is the trustworthy
path.

### 5. Inference rule when `listeners` is empty

`ServiceSpec` requires `listeners` to be non-empty per ADR-0047 §1
(`ParseError::NoListeners`). The inference rule is therefore
unreachable on a zero-listener spec because parse fails before
inference runs. This is the structural reason the inference rule
can safely use `listeners[0]` without guarding for empty.

### 6. Inferred probe and the `0.0.0.0` bind host

The inferred probe targets `0.0.0.0:<port>` because:

- Workload listener bind addresses are not currently surfaced on
  `Listener` (per ADR-0047 §4a `ListenerRow.port` + `protocol` +
  `vip`; no bind-host field).
- Phase 1 single-node makes "bind to all interfaces and the
  loopback `0.0.0.0` resolves" the safe assumption.
- The probe runs from the worker process which is local-host; the
  TCP connect succeeds if the workload binds any of `0.0.0.0`,
  `127.0.0.1`, or the host's primary interface address.

If a future spec gains an explicit `bind_host` field on `Listener`,
the inferred probe uses that value instead. Additive change.

## Considered alternatives

### Alternative A — K8s/Nomad default: no probe, Stable on Running

Rejected per §4 — this IS the bug. The RCA proves that
kernel-accepted exec is not operator-meaningful liveness; the
platform default must be better.

### Alternative B — Infer HTTP `/health` probe instead of TCP

Heuristic: if `listener[0].port == 80 || 8080 || 443 || 8443`, infer
HTTP probe to `/health`. Rejected: heuristic produces false
expectations (operator's app may not expose `/health`; probe
returns 404; alloc never reaches Stable). TCP-connect is the safest
"is anything listening?" claim that does not require operator
cooperation.

### Alternative C — Infer multi-listener probe (all listeners)

For Services with N listeners, infer N TCP-connect probes. Rejected:
the AND-of-all gate (per ADR-0055 §5) would delay Stable until
every listener binds. Phase 1 operators typically have one
public listener + maybe an admin listener; requiring both before
Stable is overly conservative. First-listener default is more
forgiving; operators who want multi-listener gates declare them.

### Alternative D — Make inference operator-opt-in (not default)

Require `[health_check].infer_default = true` for inference to fire.
Rejected: opt-in defeats the "honest by default" goal. Operators
must remember to opt in to get the trustworthy signal; the silent
default reverts to RCA-A behaviour. Opt-out (the chosen shape) makes
the trustworthy path the default.

## Consequences

### Positive

- **RCA-A closed structurally** for the most common case
  (no-probes-declared Service): the platform's default does the
  right thing.
- **Operator iteration loop is honest by default**: submit → Stable
  reflects real listener-bind, not kernel-accepted exec.
- **`(inferred)` marker makes the rule visible** to operators
  inspecting `alloc status`; no operator is surprised by
  invisible behaviour.
- **Explicit opt-out preserves Phase 1 compatibility** for
  operators who genuinely want first-Running semantics.

### Negative

- **Divergence from K8s/Nomad default** is a new operator surface
  to document. Mitigation: operator changelog entry; first-launch
  docs explicitly call out the "honest by default" choice and the
  opt-out shape.
- **Inference rule is a SemVer contract**: any future change
  requires a configurable knob with the current rule as default.
  Bounded; cost is one Phase 2+ ADR if/when the rule evolves.

### Quality-attribute impact

| Attribute | Impact |
|---|---|
| Functional correctness | RCA-A closed for default workflow |
| Usability — operator config | Trustworthy default; opt-out documented |
| Reliability — fault tolerance | Workload that never binds gets `Failed`, not silent `Running` |
| Compatibility — evolvability | Opt-out preserves spec compatibility |

## Cross-references

- RCA `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`
  root cause A
- ADR-0054 — ProbeRunner
- ADR-0055 — ServiceLifecycleReconciler consumes inferred probe
  identically to explicit probes
- ADR-0057 — TOML spec; defines `ProbeDescriptor.inferred` flag
- `feature-delta.md` US-01, C4 (default policy)

## Changelog

- 2026-05-24 — Initial accepted version. Closes RCA-A for default
  operator workflow. Sets "honest by default" as Overdrive convention.
