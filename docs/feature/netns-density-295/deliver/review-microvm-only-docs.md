# MicroVM-only documentation review

## Scope and verdict

**NEEDS_REVISION — one bounded architecture-document finding.** The microVM-only product correction is supported by the user's instruction, accepted design, and current implementation. The website no longer advertises the retired host-process workload or host Exec probe. One replacement brief diagram no longer depicts the architectural behavior its accompanying text claims it shows.

Reviewed on 2026-11-02 under the current `AGENTS.md`, website conventions, technical-writer skill, domain-naming rule, and finalization gates. This is a documentation review, not a production defect investigation or a request to change architecture.

The complete fourteen-file author scope was reviewed:

- `docs/product/architecture/brief.md`.
- `website/app/(home)/page.tsx`.
- The historical `what-is-ebpf.mdx` blog clarification.
- Website introduction, comparisons, CLI reference, architecture, microVMs, guest networking, transparent mTLS, dial by name, reconcilers, deploy guide, and reconciler-writing guide.

The editorial/validation ledger is `.context/microvm-only-docs.md`. Only this review artifact was added by the reviewer. User-dirty `AGENTS.md` and all writer changes were preserved. No source, content, policy, model, test, dependency, lockfile, or DES edit was made. No commit, push, deployment, publication, Rust/model/native test, VM, Lima, Metal, BPF, kernel, cgroup, or nftables operation was performed.

## Authority and current behavior

The user's explicit microVM-only instruction supersedes prior website descriptions of process-plus-VM support. It also agrees with accepted ADR-0110 through ADR-0113. Current source confirms the boundary rather than merely containing an unused VM type:

| Product contract | Evidence and assessment |
|---|---|
| Only `[vm]` is a live workload driver table | `crates/overdrive-core/src/aggregate/mod.rs:843` and `aggregate/workload_spec.rs:487` each have a VM-only `DriverInput`. The persisted workload driver at `aggregate/mod.rs:156` is VM-only. ADR-0110 removes the retired adapter while preserving tagged unions and routing. |
| Guest invocation consists of command, args, kernel, and rootfs | The API `VmInput` at `aggregate/mod.rs:858` and parser `VmInput` at `aggregate/workload_spec.rs:456` have precisely those four fields. The two deploy examples and landing example supply exactly those keys, plus resources. Guest command paths and operator-supplied host artifact paths remain distinct. |
| Jobs and services are lifecycle kinds, not host-process versus VM substrates | ADR-0110 retains driver-neutral lifecycle ownership and routing. The corrected introduction, landing, comparisons, and architecture consistently describe services and jobs in Linux microVMs. No new guest runtime is advertised as implemented. |
| Only TCP and HTTP health mechanics remain | `aggregate/probe_descriptor.rs:61` contains only `Tcp` and `Http`. ADR-0113 removes host Exec probes and preserves roles, thresholds, observation, and supervision. The guides do not substitute a host command probe or claim the future guest-command probe exists. |
| Optional VM capability can leave an empty registry, while required capability failure refuses startup | `crates/overdrive-control-plane/src/lib.rs:3173` starts with an empty registry, inserts only a successfully composed VM driver, and branches on typed `VmComposeError`. The capability-dependent probe mapping at `:3406` agrees with ADR-0219. The brief and website describe this without inventing an operator key or a fallback adapter. |
| Driver, registry, allocation routing, and exit-observer ownership remain | ADR-0110 and the current composition preserve these boundaries. The brief's updated driver section retains them rather than collapsing to a scalar VM implementation. |
| Guest capture/vsock and host mTLS remain separate responsibilities | Current guest-network contracts and reviewed implementation retain host identity, transparent listeners, inbound TPROXY, kernel forwarding, and owner-opened guest delivery. The corrections remove the retired workload path without removing valid host-agent/VMM process, cgroup, seccomp, Landlock, socket, or TPROXY mechanisms. |
| TLS forwarding and current limitations remain qualified | The mTLS page retains host TLS endpoints, forwarder ULP refusal, temporary userspace guest-delivery relay, chain-to-bundle authentication, and single-node TCP scope. Guest networking retains the Service-VIP UDP, CID replacement crash-window, non-Linux adapter, and performance-measurement limits. No GA or new benchmark claim is introduced. |

No production contradiction requires a new interface, test seam, code deletion, or runtime remediation. The retired process terminology in historical ADRs and audits is not evidence of a supported current product mode.

## Website coverage and writing assessment

An independent search of the entire website docs tree and landing page found no remaining `[exec]`, `ExecDriver`, `ExecProber`, process-backed service, or supported process-workload invocation. The remaining host-process references correctly describe the agent and VMM or explicitly exclude another workload execution mode. Competitor raw-exec descriptions are not Overdrive claims.

The examples remain usable artifact-based VM examples. The deploy guide says paths are placeholders supplied by the operator, documents matching guest-kit init/module requirements, and preserves reflink, KVM, kernel-vhost-vsock, and source-integration requirements. It invents no image registry, prebuilt fork release, guest probe API, or config key.

Landing changes affect authored copy, supported-type labels, and the displayed TOML example. No component control flow was rewritten. The eBPF post receives one short current-boundary callout. It keeps the original technical discussion as history rather than pretending it was originally written about a different implementation. The cgroup and other historical posts do not require an unrelated rewrite.

The changes use precise host-versus-guest objects and keep the distinction between an OS process and a supported host-process workload. No new public delivery coordinates were introduced. The changed explanations assume a technical reader but give enough context for the VM, artifact, networking, and probe boundaries.

## Brief scope and invariant audit

The 672-line brief diff is larger than a vocabulary replacement because it removes whole obsolete Exec grammar, placement, probe, composition, and diagram blocks. The corresponding accepted supersession is real: ADR-0110 governs the live driver surface, ADR-0111/0112 govern forward-only envelopes and lifecycle vocabulary, ADR-0113 removes only the host Exec probe, and ADR-0219 governs capability composition.

The current-boundary preface and dated source/reuse labels make the retained historical Exec comparisons inapplicable as current APIs. The actual changelog content is byte-identical to HEAD, independently verified. No ADR, roadmap, or historical evidence file was rewritten. Current VM supervision, distinct allocation identity, resource/confinement ownership, typed capability handling, guest readiness/exit, and network activation/teardown contracts retain their governing references.

No new normative permission to weaken a security, error, lifecycle, cleanup, or isolation invariant was identified. The documented replacements point to current approved contracts rather than prescribing new architectural mechanisms. However, the broad diagram simplification produces the concrete documentation defect below.

## Finding

### F-01 — Medium — convergence diagram no longer shows its stated loop and execution boundaries

**Location:** `docs/product/architecture/brief.md:3645`, section “C4 Level 3 — Convergence-loop closure”.

The revised introduction says the diagram shows the components and async/sync boundaries explicitly. The text after it says the diagram makes the convergence loop's architectural property visually explicit. The replacement diagram is instead the same VM execution/guest-network summary used by the container view. It combines the runtime and action shim into one box, omits the pure reconciler and scheduler, omits hydration and view persistence, and ends at an observation box without an edge back to hydration. It shows no async/sync labels or persistence-before-dispatch ordering.

This mismatch is directly visible in the changed document. It is not a hypothetical production defect. The current production path still has these distinctions: `crates/overdrive-control-plane/src/reconciler_runtime.rs` hydrates state, calls the pure reconcile function, and at `:1563` onward awaits `persist_view` before dispatch. The source comment identifies ADR-0035's durability ordering as load-bearing. Removing the retired Exec adapter does not remove these boundaries or turn the convergence view into a guest-networking view.

**Required bounded correction:** make this section's diagram and accompanying description agree while preserving the relevant convergence architecture. Retain the pure reconcile/scheduler boundary, hydration, view persistence before dispatch, driver-registry/VM dispatch, observation, and next-iteration relationship in the convergence view. Replace the obsolete Exec endpoint with the current VM route. Alternatively, preserve the detailed convergence view as an explicitly applicable separate diagram and accurately label the new picture as a summary. Do not imply that an unlabeled execution chain displays async boundaries or loop closure. This requires documentation only, with no new owner, operation, API, or architecture decision.

**Disposition:** OPEN. The original writer should remediate this section and return it for the same reviewer's focused re-review. No production, model, test, or unrelated website change is requested.

## Validation and limits

Independent checks performed:

| Check | Result |
|---|---|
| Complete changed-file diff, plus whole website docs/landing workload vocabulary audit | PASS for the VM-only product boundary |
| Local MDX links and heading fragments | PASS, 75 targets/fragments checked |
| Two deploy TOML examples and landing example | PASS, all parse and contain exactly command/args/kernel/rootfs in the VM table, with resources and no Exec table |
| Brief actual changelog against HEAD | PASS, byte-identical |
| Existing one-index verifier against retained build | PASS, 28 published pages including six blogs, one draft excluded |
| `git diff --check` | PASS |

The writer records passing current typecheck, lint, production Next build, local OpenNext build, and HTTP/Markdown/discovery smoke. Those expensive builds were not redundantly rerun here. The writer's added-prose punctuation audit reports zero semicolons, with literal TypeScript punctuation preserved. Earlier untouched prose is not claimed to have received a site-wide punctuation migration.

The supplemental Mermaid parse under Bun failed because its browser DOMPurify interface was unavailable. This is not evidence of invalid Mermaid syntax. The modified flowcharts use ordinary supported syntax on static inspection, but no browser-rendered diagram check was performed or claimed. No browser tool is available in this review session, and no package or sanitizer was added to manufacture such a check. F-01 is about the diagram's architectural meaning, independently visible without rendering, not the Bun environment failure.

The pre-existing search-ranking failure remains qualified by the earlier current/pristine-baseline reproduction. No full runtime-suite or analytics-branch pass is claimed, and no search fix is required for this bounded correction. Website checks establish content/build behavior, not VM or kernel correctness.

## Final verdict

**NEEDS_REVISION.** The requested VM-only correction is otherwise supported and materially complete. Close F-01's diagram/prose mismatch without expanding implementation or changing approved architectural contracts, then return for focused re-review. No publication or commit is authorized by this artifact.

## Review iteration 2 — convergence view remediation

### Scope and evidence

Reviewed only the corrected convergence-loop section and ledger update. The writer's website changes and all prior factual, scope, and validation dispositions remain unchanged. Only this review artifact was edited by the reviewer, with user-owned `AGENTS.md` and writer changes preserved.

The revised diagram now depicts the workload loop rather than substituting guest-networking detail. Deploy/stop handlers commit intent and enqueue evaluation. The runtime drains evaluation, hydrates desired/observed state, reads the cached view, invokes the pure reconciler and scheduler, persists the next view, and dispatches actions through the separate shim. VM-only registry/allocation routing reaches the driver. Allocation state and the per-driver exit observer feed the ObservationStore, which supplies actual state for the next evaluation.

These labels match the existing implementation and approved architecture:

- `reconciler_runtime.rs` hydrates both state inputs, reads `get_view` synchronously, and calls pure `reconcile` before awaiting `persist_view` and dispatch. Persistence failure propagates before dispatch, matching ADR-0035's ordering.
- `WorkloadLifecycle` uses the existing synchronous scheduling helper. The diagram keeps that decision inside the pure boundary rather than introducing a second runtime or owner.
- The existing action shim, driver registry, allocation routing, and per-driver exit observer remain the effect and observation paths. The picture does not invent direct reconciler I/O, a host-process workload driver, or guest-network ownership inside the convergence loop.

The accompanying prose now accurately distinguishes the pure decision boundary from async hydration, durable view persistence, lifecycle effects, and observation writes. It removes the earlier contradictory claim that only the shim was async and expressly scopes the picture to the workload path rather than every action. The feedback edge and next-evaluation explanation close the loop that was absent from the rejected summary.

### Verification and disposition

Read the corrected section against the current runtime and scheduling paths. Static checks confirm the required components, ordering labels, observation feedback, balanced code fences, and zero semicolons in this revised section. `git diff --check` passes. Website content did not change during remediation, so no website build was repeated. The earlier browser-rendering and runtime-ranking qualifications remain intact. No Rust, model, native, dependency, VM, or deployment operation was performed.

| Finding | Remediation | Disposition |
|---|---|---|
| F-01, convergence diagram did not show the loop or boundaries claimed by its prose | Restored hydration/cached-view input, pure planning and scheduling, persistence before dispatch, separate shim/VM routing, exit observation, and next-evaluation feedback, with matching prose | **CLOSED** |

No new finding or architectural requirement is introduced.

### Iteration 2 final verdict

**APPROVED — microVM-only website and brief correction.** F-01 is closed by the bounded documentation remediation. The VM-only product boundary, supported probe/configuration shapes, current host networking mechanisms, historical applicability labels, and prior validation qualifications remain accepted. This content approval does not authorize publication, deployment, or feature finalization.
