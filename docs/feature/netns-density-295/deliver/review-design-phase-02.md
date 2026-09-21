# DESIGN Review — `netns-density-295` phase-02 remediation

## Metadata

- Review type: independent solution-architecture review
- Iteration: 1
- Reviewed scope: the 2026-09-20 additions to
  `docs/feature/netns-density-295/feature-delta.md` and the corresponding
  phase-02 remap in `docs/feature/netns-density-295/deliver/roadmap.json`
- Trigger: `docs/feature/netns-density-295/deliver/review-02-01.md`, including
  its confirmed DESIGN dependency gap and DISTILL completeness gap
- Authoritative architecture: the accepted #295 feature delta, architecture
  brief, ADR-0114/0115/0116/0117/0118/0120/0121/0122/0123/0124/0125/0126,
  DISTILL scenario and RED-classification artifacts, spike decisions, and the
  affected production owner/adapter paths
- Repository state: existing commits `c60cdd3b` and `7ec987a8` remain rejected
  `Step-Id: 02-01` history; no commit, production code, test, DES event, design
  artifact, or roadmap field was changed by this review
- Authorized write: this review artifact only

## Review summary

The remap correctly identifies the original cycle and chooses the right
delivery grouping under the active-step/history constraints: the one
shared-switch owner step must own the classifier, the semantic bridge guard,
and the TCX effects it consumes; the IP-family constant-rule replacement is a
separate later slice; listeners remain later still. The exact D6 names, D9
semantic surface, single owner, single-cut legacy removal, scenario moves, and
rejected-commit history are stated accurately.

The remediation is nevertheless not executable yet. Two concrete DESIGN gaps
remain after revalidation of the production paths:

1. the accepted crate split has no exact cross-crate production boundary by
   which the control-plane-owned host owner can ask the dataplane-owned aya
   adapter to load, pin, adopt, insert, or attach TCX state; and
2. the current production boot composition has no injectable/observable
   reclamation boundary with which DISTILL can author the mandatory seeded
   S-ND295-13 invariant before changing source order.

Both gaps are explicitly protected by the repository's no-invented-API rule.
They cannot be delegated to the `02-01` crafter or silently filled by DISTILL.

## Iteration 1 evidence

### Observed production facts

| Fact | Evidence | Assessment |
| --- | --- | --- |
| The original `02-01` consumes TCX and bridge-guard effects that the old roadmap assigned to later `02-02`. | `review-02-01.md` iteration 2; `guest_tcx.rs:110-154` remains five RED panics; the rejected `02-01` diff did not change `overdrive-dataplane` or `overdrive-bpf`. | Confirmed dependency cycle. |
| The ordinary production action path still selects the legacy network owner while the new provisioner is reached only through test-gated helpers. | `action_shim/mod.rs` still composes `HostNetworkProvisioner`, `NetSlotAllocator`, netns/veth teardown, and the test-only guest-network helper; `AppState` still has `net_slot_allocator`. | Confirmed bounded `02-01` implementation work after upstream approval. |
| The current host scratch binding and allocation owner do not perform real TCX effects. | `guest_network.rs:579-669` defines the private D5 seam, while `RealSharedGuestNetworkScratchIo::apply_tcx`, counts, exercises, and handle transitions are no-op/zero scaffolds; provision/teardown remain incomplete as recorded by `review-02-01.md`. | Confirmed implementation state, not authorization to invent a boundary. |
| The current boot path calls shared-switch sweep before VM reclamation. | `run_server_with_obs_and_drivers` calls `probe_startup`, `sweep_stale`, and `converge_shared` at `lib.rs:2545-2573`; `vm_reclamation_boot::converge` is later at `lib.rs:3267-3275`. | Confirmed source-order mismatch; production failure remains unproved. |
| No complete executable S-ND295-10/11/12 bodies exist, and the current S13 body proves only stale-TAP removal. | Test inventory and `review-02-01.md` iteration 2; `shared_guest_network_startup.rs:324` creates a TAP/guard but no prior VMM, endpoint, link, pin, or full complement. | Confirmed DISTILL blocker. |
| The two rejected commits and DES events already belong to `02-01`. | Both commits contain `Step-Id: 02-01`; `execution-log.json` retains the existing `02-01` RED/GREEN/COMMIT/COMMIT events. | History must remain untouched; the same-step remediation rule applies. |

### Accepted architecture facts

- One private `HostSharedGuestNetworkOwner` owns both the inherited
  `GuestNetworkProvisioner` operations and the five shared-owner operations.
  No boot/allocation owner split is allowed.
- Raw aya and the private endpoint/counter ABI terminate in
  `overdrive-dataplane::guest_tcx`; control-plane has no direct aya dependency.
- D-295-DISTILL-6 exposes exactly `GuestTcxAttachment`, `GuestTcxCounter`,
  `TcxAttachPoint`, `GuestTcxError`, and the five doc-hidden functions
  `query_attachment`, `detach_pinned_link`, `endpoint_present`,
  `remove_endpoint`, and `read_counter`.
- D-295-DISTILL-5 is a module-private control-plane scratch-algorithm seam. Its
  closed actions include TCX load/map/link lifecycle, but the seam itself is
  not a cross-crate dataplane API and cannot be named by the dataplane crate.
- D-295-DISTILL-9 already pins the semantic bridge-family spec/fact/inventory/
  outcome/error types and the exact observe/converge/member/delete operations
  over one private family-aware codec.
- Boot ordering remains scratch probe, VM reclamation, stale shared-attachment
  sweep/read-back, shared convergence, then admission. A source-order mismatch
  is not sufficient proof of a production defect; the seeded Sim invariant is
  the mandatory first reproduction.

## Option and dependency analysis

| Option | Dependency direction | Atomicity, rollback, and evidence | Existing `Step-Id` effect | Assessment |
| --- | --- | --- | --- | --- |
| Consolidate classifier, D6, D5 lifecycle binding, and D9 bridge guard into revised `02-01`; leave IP-family replacement in `02-02`. | `01-01 -> 02-01 -> 02-02 -> 02-03`; no later-step behavior is consumed by an earlier step. | One shared-switch owner is reviewed and rolled back as one attachment slice; IP target replacement retains its own rollback boundary. S08/09 and S10..13 follow their production effects. | Preserves the rejected `02-01` commits and permits same-step remediation by the original crafter after upstream gates. | Correct selected decomposition, subject to the two DESIGN gaps below. |
| Add a fresh adapter prerequisite step. | Could be acyclic in a clean roadmap. | Gives a smaller adapter slice, but the active `02-01` cannot advance to another step while unapproved. | Would require advancing past the failed step or changing established history/sequence. | Correctly rejected for this active run. |
| Run old `02-02` before `02-01`. | Pulls unrelated IP replacement ahead while the owner still lacks its prerequisites. | Mixes S14..19 with the prerequisite adapter problem and leaves S10..13 incomplete. | Cannot reinterpret the existing `02-01` commits as later-step work. | Correctly rejected. |
| Merge all old `02-01` and `02-02`. | Acyclic. | Unnecessarily joins bridge/TCX ownership to independent IP replacement and rollback, increasing failure and review blast radius. | Preserves history but makes the active step larger than required. | Correctly rejected. |
| Approve an interim no-op, process-local, legacy, or dual-owner path. | Superficially unblocks sequencing by weakening the step. | Produces green evidence over a non-production composition and breaks the single-cut contract. | Would misrepresent the rejected commits as complete. | Correctly rejected. |

The selected `02-01` is large at 48 hours, but its size follows the atomic
owner boundary and the already-started step constraint. The remap does not
arbitrarily absorb the independent IP-family rollback slice.

## API and visibility analysis

### Confirmed correct fences

- The revised documents replace the old inaccurate
  “attach/pin/adopt/query/remove” description with the exact five D6
  operations.
- They do not add a public TCX attach, pin, adopt, generic remove, family
  parameter, owner method, fault hook, persistence store, recovery protocol,
  second owner, or compatibility branch.
- D9 bridge operations and semantic values match the accepted feature-delta
  fence. `02-02` is explicitly forbidden from owning D6 or D9 behavior.
- The existing `GuestNetworkProvisioner`, `SharedGuestNetworkOwner`,
  `GuestNetworkPlan`, grouped assignment, and VMM attachment shapes remain
  unchanged.

### Finding DESIGN-P02-01 — Blocking: no exact dataplane lifecycle boundary exists for TCX load/attach/pin/adopt

**Severity:** Critical

**Evidence:**

- D-295-DISTILL-4 assigns raw aya, lifecycle translation, and private map ABI
  to `overdrive-dataplane::guest_tcx`, while explicitly forbidding a direct aya
  dependency in control-plane (`feature-delta.md:657-667`).
- D-295-DISTILL-6 exposes only the five query/detach/endpoint/counter functions
  (`feature-delta.md:735-799`). The current module contains exactly those five
  public functions and no load, insert, attach, pin, or adopt production
  operation (`guest_tcx.rs:110-154`).
- D-295-DISTILL-5's `SharedGuestNetworkScratchIo` and its action/resource enums
  are module-private to control-plane (`feature-delta.md:1902-2049`; current
  `guest_network.rs:579-620`). A module-private control-plane trait cannot be
  implemented or named by the sibling dataplane crate, and a control-plane
  implementation cannot invoke private dataplane functions across a Rust crate
  boundary.
- The remediation states that lifecycle must bind “only through D5's exact
  existing scratch action/resource seam,” but it does not name the exact
  dataplane boundary that the production D5 implementation calls. Current
  `RealSharedGuestNetworkScratchIo::apply_tcx` is consequently a no-op
  (`guest_network.rs:629-667`).

**Reachability:** Revised `02-01` requires both startup probe and ordinary
provision to load the production program/maps, insert an endpoint, attach TCX
ingress, pin/adopt maps and links, then read them back. The control-plane owner
cannot perform those required effects through any accepted existing dataplane
surface. A crafter must otherwise add an unspecified public/doc-hidden
dataplane operation or type, add direct aya to control-plane, move ownership,
or fake the effect. Each option violates an existing normative fence.

**Bounded disposition:** Return this as a DESIGN/API gap. The design architect
must obtain explicit approval for the exact cross-crate production lifecycle
shape, or for a different ownership boundary, before DISTILL or the `02-01`
crafter proceeds. This review does not prescribe a method, type, trait, or
visibility. At minimum the feature-delta API SSOT must become exact; if the
resolution changes component ownership or dependency direction, the brief and
relevant ADRs must also be amended and independently reviewed.

### Finding DESIGN-P02-02 — Blocking: the mandatory S13 seeded invariant is not drivable through the existing production composition

**Severity:** Critical

**Evidence:**

- The remediation requires a seeded `overdrive-sim` invariant through the
  current production boot entry and existing `VmReclamation`/
  `SharedGuestNetworkOwner` ports before any source reorder.
- `run_server_with_obs_and_drivers` accepts the shared owner but no
  `VmHostState`, reclamation adapter, or shared ordering observer
  (`lib.rs:2538-2544`). It invokes the shared owner's probe/sweep/converge before
  constructing and driving the boot-reclamation state (`lib.rs:2545-2573` and
  `3267-3275`).
- `SimSharedGuestNetworkOwner` can script owner-port results and record only
  its own `GuestNetworkOperation` calls (`overdrive-sim/.../guest_network.rs:
  19-83,155-209`). It has no prior-VMM state, reclamation callback, or common
  trace with `VmReclamation` from which to prove the required happens-before
  relation.
- The current S13 integration body uses real TAP/guard setup only; it cannot
  substitute for the mandated seeded production-owner invariant and does not
  establish that a real prior VMM was reclaimed.

**Reachability:** The invariant must fail against today's order and print its
seed. With the current entry point and ports, a Sim test can observe that
`sweep_stale` was called, but cannot create a deterministic prior VMM owned by
the production reclamation path or observe whether reclamation completed
before that call. A native-metal spike may prove real reachability, but the
repository rule explicitly says it does not replace the seeded invariant.

**Bounded disposition:** The remediation's conditional testability warning is
now a concrete blocker. Record that DISTILL cannot author executable S13 at
the required boundary and return the exact missing testability decision to the
user. Do not add a Sim seam, production API, or alternate evidence contract
without explicit approval. After an approved DESIGN resolution and independent
review, DISTILL may author the seeded invariant and native-metal body.

## Scenario and evidence analysis

| Scenario group | Revised owner | Assessment |
| --- | --- | --- |
| S-ND295-00, 02..04, 06..07 | `02-01` | Correct: pool, grouped handoff, private scratch algorithm, and production action-owner behavior are part of the shared-switch owner slice. |
| S-ND295-05 | `03-01` | Correct: it is the pure fixed-placement-cap boundary in the core scheduler acceptance file, not a classifier or shared-switch adapter behavior. The revised `02-01` no longer claims it. |
| S-ND295-08/09 | `02-01` | Correct: these scenarios prove the classifier/maps now owned by the consolidated shared-switch step. Leaving them in `02-02` would recreate the dependency mismatch. |
| S-ND295-10 | `02-01`, pending DISTILL | Correct evidence split: typed D6 detach/query, real D9 guard/counter observation, and Lima real-kernel no-escape/audit. No fixture-owned production effect is permitted. |
| S-ND295-11/12 | `02-01`, pending DISTILL | Correct paired layers: source-local D5 owner-order/failure proof plus ordinary production-composition Lima evidence and real complements. They remain blocked by DESIGN-P02-01. |
| S-ND295-13 | `02-01`, incomplete DISTILL | Correct requirement of seeded ordering reproduction first and native-metal VMM/kernel complement second, but the current ports cannot drive the first layer; DESIGN-P02-02 blocks handoff. |
| S-ND295-14..19 | `02-02` | Correct: IP-family constant-rule replacement, refusal, exact rollback, and runtime no-rewrite remain independent of classifier/bridge ownership. |
| S-ND295-20..26 | `02-03` | Correct and unchanged: listener/capability registry ownership remains after IP target convergence. |

The revised phase-02 activation sets are unique. The full roadmap still has the
pre-existing duplicate prerequisite metadata for S-ND295-27 and S-ND295-28,
already recorded by `review-roadmap.md`; that is outside this bounded phase-02
remediation and is one reason the roadmap remains pending rather than globally
approved.

Rust tests remain in-process; real-kernel Lima and native-metal tests retain
their substrate authority; no expectation is asked to invoke Rust tests or
recreate an implementation. The S10/S11/S12 handoff is otherwise specific
enough for DISTILL once DESIGN-P02-01 is resolved. S13 is not.

## Brief and ADR impact

The consolidation itself does not change component ownership, public behavior,
kernel policy, lifecycle order, persistence, recovery, or deployment. Moving
already-approved D5/D6/D9 work into the step that consumes it therefore does
not by itself warrant a brief or ADR amendment.

That conclusion cannot be used to bypass DESIGN-P02-01. If closing the missing
TCX lifecycle boundary changes the accepted ownership or dependency graph, the
brief and the affected ADRs must change. If it only makes an already-approved
internal boundary exact, the feature delta may be sufficient, but that choice
belongs to the design architect and user rather than this reviewer.

## Roadmap checks

| Check | Result | Evidence |
| --- | --- | --- |
| JSON/schema | PASS | `python3 -m des.cli.roadmap validate` reports `VALID: 4 phases, 11 steps`. Length/count warnings are the repository's documented non-blocking precision warnings. |
| DES roadmap integrity | PASS | `des-verify-integrity --roadmap-only` reports roadmap format OK and no validator errors. |
| Dependency graph | PASS for the remap | `01-01 -> 02-01 -> 02-02 -> 02-03 -> 03-01` is acyclic and no revised phase-02 step consumes later-step behavior. |
| Reference resolution | PASS | Every revised `roadmap.design_refs` path exists and names the active accepted brief/ADR set. |
| Phase-02 traceability | PASS | S00/02..04/06..13 map to `02-01`; S14..19 to `02-02`; S20..26 to `02-03`; S05 remains only in `03-01`. |
| Exact public API | FAIL | D6 and D9 declarations are accurate, but the production TCX lifecycle cross-crate shape required by D5 is absent (DESIGN-P02-01). |
| Independent executability | FAIL | `02-01` cannot implement its TCX lifecycle without inventing API, and its mandatory S13 RED cannot be authored through the existing production composition (DESIGN-P02-01/02). |
| Diff hygiene | PASS | `git diff --check` and `jq empty` pass for the two reviewed files. The review did not touch unrelated dirty work. |
| Mutation testing | NOT RUN | Correctly excluded from this DESIGN review. |

The earlier whole-roadmap findings concerning benchmark entry shape, global
concision, S27/S28 duplicate prerequisite metadata, S36 command linkage, and
the CLI driving-adapter lane remain pending in `review-roadmap.md`. This review
does not silently approve or remediate them.

## Findings and remediation disposition

| Finding | Status | Required bounded disposition |
| --- | --- | --- |
| DESIGN-P02-01 — missing exact dataplane TCX load/attach/pin/adopt lifecycle boundary | OPEN, blocking | Return to DESIGN/user decision; pin the exact cross-crate shape without inventing it in DELIVER. Amend the appropriate SSOT and re-review. |
| DESIGN-P02-02 — existing production composition cannot drive the mandatory seeded S13 reclamation-before-sweep invariant | OPEN, blocking | Record the concrete testability gap and obtain explicit approval for an exact boundary or changed evidence contract; then rerun DESIGN and DISTILL review. |

## Final verdict

# CHANGES_REQUIRED

The phase-02 consolidation and scenario remap are directionally correct, but
two unresolved DESIGN blockers prevent an executable DISTILL handoff and an
honest revised `02-01`. The roadmap must remain `validation.status = pending`.
No DELIVER remediation or later roadmap step may begin from this review.

## Iteration 2 — D-295-DISTILL-12/13 remediation review

### Scope and changed claim

Iteration 2 preserves the iteration-1 record and reviews only the subsequent
D-295-DISTILL-12/D-295-DISTILL-13 feature-delta amendments and their matching
roadmap changes. D12 proposes an exact opaque dataplane lifecycle boundary for
TCX load/map/link ownership. D13 makes the existing `VmHostState` port a
mandatory server-helper dependency and adds one structured production boot-
phase trace for the required seeded S-ND295-13 invariant. Step `03-03` now
correctly treats both the real owner composition and the cumulative helper
signature as completed by revised `02-01`.

The updated design was compared with the locked aya 0.13.1 API, the accepted
D4/D5/D6/D9 ownership fences, `SimVmHostState`,
`vm_reclamation_boot::converge`, the existing server composition, all current
helper callers, the DISTILL scenario catalogue, the rejected `02-01` commits,
and the unchanged DES history.

### Prior-finding dispositions

| Prior finding | Iteration-2 disposition | Evidence |
| --- | --- | --- |
| DESIGN-P02-01 — no exact dataplane lifecycle boundary | **PARTIALLY RESOLVED; remains blocking through DESIGN-P02-03/04 below.** | D12 now pins a coherent stateful mutation boundary: opaque non-`Clone` `GuestTcxProgram`, `GuestTcxLink`, and `GuestTcxAdoptedState`; exact semantic values; exact methods; one owner constructor; D5 action mapping; D6 unchanged; and explicit Drop behavior. It still does not supply the accepted D5 cleanup-inventory reads, and one mismatch value cannot represent the observation it promises. |
| DESIGN-P02-02 — S13 cannot be driven through production composition | **RESOLVED.** | D13 injects the already-existing `VmHostState` production dependency, preserves the real boot drive, supplies an exact non-persisted production trace, and defines a reachable seeded prior-host state plus a failing current-order oracle. No new boot owner, Sim model, public state accessor, or fabricated consequence is introduced. |

### D-295-DISTILL-12 boundary analysis

#### Ownership, visibility, and Rust implementability

The selected opaque-state option is structurally preferable to the rejected
alternatives. It preserves the accepted control-plane → dataplane dependency,
keeps raw aya handles and the endpoint ABI inside dataplane, and avoids a
second lifecycle trait, a generic command API, direct aya use in control-plane,
or a coarse transaction that would take D5's ordering and cleanup authority
away from the host owner.

The state transitions are implementable against aya 0.13.1:

- `SchedClassifier::attach_with_options` supports TCX ingress with
  `LinkOrder::first`, and its owned link can be taken and converted to an
  `FdLink`;
- consuming link pin/detach operations align with the non-`Clone` opaque link;
- pinned links can be reopened and explicitly unpinned into an owned FD link;
- map info exposes ID, type, key size, value size, and maximum entries; and
- endpoint map conversion and reads/writes can remain wholly inside
  `overdrive-dataplane::guest_tcx`.

The exact methods are necessary for the accepted D5 leaf ordering. Loader
state is held once in the one private production owner, scratch state remains
isolated inside its private real D5 I/O, and Sim implements only the existing
high-level application owner. No second production owner or generic public test
seam was found.

#### Drop, rollback, and cancellation ownership

D12's Drop discipline is correct: dropping `GuestTcxProgram` closes loader
handles without unpinning, and dropping `GuestTcxAdoptedState` closes adopted
FDs without removing pins. Normal-path effect completion therefore cannot be
mistaken for task submission or hidden by best-effort Drop. A failed consuming
link pin drops the unpinned link and detaches it; successful pinning survives
loader close until explicit unpin/detach. D5 remains the owner of reverse
cleanup, first-source retention, continuation after failure, and complete
post-cleanup observation.

The new dataplane methods are synchronous, so no Rust future can be cancelled
inside an individual aya mutation. The production convergence owner drains
already-admitted evaluation futures on normal shutdown, and boot directly
awaits its owner operations. Abrupt process death leaves only the deliberately
pinned residue that the next boot's post-reclamation sweep owns. No detached
future, runtime discovery, or hidden Drop repair is introduced by D12.

#### Finding DESIGN-P02-03 — Blocking: D12 does not implement D5's exact eight-family TCX inventory boundary

**Severity:** Critical

**Evidence:**

- The accepted D5 contract requires `count_tcx` for `EndpointMap`,
  `CounterMap`, `EndpointEntry`, `TcxProgram`, `TcxLink`, `EndpointMapPin`,
  `CounterMapPin`, and `TcxLinkPin`, after explicit cleanup and handle release.
  Every family must become `Observed(n)` or `Unavailable`; retained objects may
  not be inferred away or reported as fabricated zero
  (`feature-delta.md:1990-2069, 2115-2143`).
- The iteration-2 prose still says D12 is the exact cross-crate boundary for
  D5 actions *and inventory* (`feature-delta.md:4398-4409`), but D12's
  exhaustive mapping at `feature-delta.md:4638-4657` maps only mutation
  actions. It names no dataplane observation for any `count_tcx` resource.
- D6 can query an interface attachment, one endpoint through an existing map
  pin, and one counter through an existing map pin. It cannot count an
  unattached loaded program or unpinned endpoint/counter map after D5 has
  removed the pins and dropped loader/adopted handles.
- The private real D5 implementation lives in control-plane. Under D4 it cannot
  call aya's `loaded_programs`, `loaded_maps`, or `loaded_links` directly, and
  it cannot inspect dataplane-private handles/ABI. Therefore the omitted
  counts cannot be implemented honestly from the accepted existing surface.

**Reachability:** Every successful and failed scratch probe reaches D5 step 7
after cleanup. The required oracle must distinguish a genuine empty kernel
complement from an accidentally retained program/map FD or kernel object. Pin
absence plus local handle Drop is not the required observation and cannot
prove those three object families are zero. A crafter following D12 must invent
another doc-hidden dataplane surface, report `Unavailable`, or fabricate zero;
none is authorized by the claimed exhaustive fence.

**Bounded disposition:** Return D12 to the design architect. Pin the exact
dataplane-owned, source-honest observation boundary for all eight existing D5
TCX resource families, including the post-unpin/post-Drop program and map
counts, or explicitly revise the already-approved D5 oracle through a separate
authorized DESIGN decision. This review does not choose a function, method,
type, or enumeration mechanism. The roadmap must then name the corrected
closed surface and evidence.

#### Finding DESIGN-P02-04 — Blocking: `GuestTcxMapKind` cannot represent every wrong observed map kind

**Severity:** High, blocking under the repository's exact-API and
source-honesty rules

**Evidence:**

- D12 requires every successfully opened pin with a wrong semantic schema to
  return the source-less `MapSchemaMismatch { expected, observed }`
  (`feature-delta.md:4583-4590`).
- `GuestTcxMapFact.observed.schema.kind` can contain only
  `EndpointHash` or `CounterArray` (`feature-delta.md:4459-4479`).
- The locked aya 0.13.1 `MapInfo::map_type` returns a much larger valid
  `MapType` space: ordinary/per-CPU/LRU hashes and arrays, program/perf arrays,
  LPM trie, maps-of-maps, sock maps, and other kernel map types. A privileged
  actor or stale/corrupt prior process can place any valid wrong map type at an
  expected bpffs path; that is precisely the foreign/conflicting read-back case
  the boot adopter must classify.

**Reachability and consequence:** If the endpoint pin opens an LRU hash, or the
counter pin opens a per-CPU array, the adapter has an actual map ID and actual
sizes but no semantic kind value with which to populate `observed`. The crafter
must mislabel the object as one of the two accepted kinds, route a semantic
mismatch through a different sourced error, or add an unapproved variant. All
three diverge from D12's exact source-honest contract.

**Bounded disposition:** Extend or otherwise correct the exact semantic
observation/error shape so every successfully observed wrong kernel map kind
has an honest representation without exposing raw aya or numeric ABI. The
architect owns that exact shape; the crafter must not invent it.

### D-295-DISTILL-13 boundary analysis

D13 closes the earlier testability gap without manufacturing an unreachable
state:

- `VmHostState` is already the production port consumed by `AppState` and
  `vm_reclamation_boot::converge`; moving its construction to the outer
  composition and passing it as a mandatory argument exposes a real dependency,
  not a `cfg(test)` override.
- Ordinary `run_server` still constructs and passes `RealVmHostState`.
  Existing injected-driver callers are bounded and can supply the existing
  `SimVmHostState`; the single-driver helper forwards the same object to the
  registry helper. There is no default or no-op fallback.
- `SimVmHostState` already seeds/removes scopes, run directories, and clones,
  and exposes the exact existing absence predicates. An empty supervision set
  is the real fresh-process condition that authorizes boot reclamation. The
  seeded run-directory/clone residue is therefore reachable production state,
  not a test-only impossible state.
- `guest_network.shared_owner_boot_phase` is a non-persisted structured tracing
  record inside the already-approved `guest_network.shared_owner_*`
  operational telemetry family. It creates no observation row, repository,
  domain event, state accessor, or recovery authority.
- The event boundaries are exact: completion is emitted only after the awaited
  production effect returns successfully. Against current source order,
  `stale_sweep/started` is observed while seeded host residue remains and before
  `vm_reclamation/completed`, so the named invariant has a genuine failing RED
  with its replay seed. After the bounded reorder, the same invariant observes
  production reclamation effects before the real owner-port sweep call.

The seeded Sim proof and native-metal proof remain independent: Sim proves
application ordering and convergence through production composition; native
metal proves actual VMM/cgroup/TAP/map/link/pin/guard/dynamic-element effects.
D13 neither weakens nor replaces the Tier-3 layer.

### Roadmap, history, and handoff checks

| Check | Iteration-2 result | Evidence |
| --- | --- | --- |
| Schema and JSON | PASS | Roadmap validator reports `VALID: 4 phases, 11 steps`; `jq empty` and `git diff --check` pass. Repository-documented length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS | `des-verify-integrity --roadmap-only` reports roadmap format OK with no errors. |
| Dependency graph | PASS | Revised `01-01 -> 02-01 -> 02-02 -> 02-03 -> 03-01` remains acyclic; `03-03` now correctly consumes, rather than redefines, D13 composition. |
| Phase-02 scenario ownership | PASS | S00/02..04/06..13 are unique to `02-01`; S14..19 to `02-02`; S20..26 to `02-03`; S05 remains at `03-01`. |
| Evidence layers | PASS except D12 inventory | S08/09 keep BPF_PROG_TEST_RUN plus real-kernel evidence; S10–12 retain source-local/Lima boundaries; S13 now has an executable seeded-production-composition design plus native metal. D5's native TCX complement remains unimplementable through the proposed D12 surface. |
| Helper caller fallout | PASS | The updated signature inventory covers the actual production and test call sites plus source-scanning expectations; file lists remain guidance rather than restrictive allowlists. |
| Existing `Step-Id` history | PASS | `c60cdd3b` and `7ec987a8` remain rejected `02-01` commits; no DES event is relabelled or backfilled. The original crafter must execute a fresh same-step RED/GREEN/COMMIT cycle after upstream approval. |
| Global scenario uniqueness | Pending outside this remediation | The unchanged S27/S28 prerequisite duplicates remain recorded in `review-roadmap.md`; no new duplicate was introduced by D12/D13. |
| Mutation testing | NOT RUN | Correctly reserved for the final DELIVER gate. |

The DISTILL handoff for S10/S11/S12 and the D13 portion of S13 is now specific
enough to author executable RED evidence. DISTILL still cannot complete S00's
real D5 complement or hand revised `02-01` to DELIVER until
DESIGN-P02-03/04 are resolved and independently re-reviewed.

### Brief and ADR disposition

D13 changes neither the reclamation owner nor boot policy; it makes an existing
port dependency explicit and observes an already-approved order. D12 likewise
keeps the accepted control-plane/dataplane ownership and dependency direction.
For those bounded changes, keeping exact signatures in the feature-delta SSOT
without editing the brief or ADRs is justified.

If remediation of DESIGN-P02-03 or DESIGN-P02-04 moves ownership, adds a new
port/owner, changes the D5 evidence contract, or exposes raw aya/ABI, that would
invalidate this non-amendment disposition and require the affected architecture
SSOTs to change. A correction that completes the same dataplane semantic
boundary may remain feature-delta-only.

### Iteration-2 findings and dispositions

| Finding | Status | Required bounded disposition |
| --- | --- | --- |
| DESIGN-P02-01 — missing TCX lifecycle boundary | PARTIALLY CLOSED | D12 closes mutation/state ownership, but cannot be considered complete until DESIGN-P02-03/04 close. |
| DESIGN-P02-02 — missing S13 production-composed testability boundary | CLOSED | D13 is exact, production-owned, reachable, and sufficient for DISTILL. |
| DESIGN-P02-03 — D12 omits the D5 eight-family TCX cleanup-inventory observation boundary | OPEN, blocking | Make the existing D5 counts implementable through an exact dataplane-owned semantic boundary, or separately amend D5 with explicit approval; no DELIVER invention. |
| DESIGN-P02-04 — wrong observed map kinds are not representable in `GuestTcxMapKind` | OPEN, blocking | Correct the exact semantic mismatch shape so every valid wrong kernel map kind is represented honestly without raw aya leakage. |

## Iteration 2 final verdict

# CHANGES_REQUIRED

D-295-DISTILL-13 resolves the S13 production-composition blocker, and D12
correctly establishes opaque lifecycle mutation ownership. D12 is still not a
complete executable boundary: it omits D5's mandatory TCX inventory reads and
cannot honestly encode every wrong observed map kind. The roadmap remains
`validation.status = pending`; DISTILL/DELIVER may not resume revised `02-01`
until these two bounded DESIGN findings are resolved and independently
re-reviewed.

## Iteration 3 — handle-free TCX inventory and opaque-schema remediation review

### Scope and prior-finding disposition

Iteration 3 preserves iterations 1 and 2 and reviews the corrected D12
inventory/schema contract plus its roadmap handoff. D13 is unchanged and was
rechecked only for drift.

| Prior finding | Iteration-3 disposition | Evidence |
| --- | --- | --- |
| DESIGN-P02-03 — no D5 TCX inventory boundary | **PARTIALLY RESOLVED; remains blocking through DESIGN-P02-05/06.** | D12 now names a handle-free inventory identity, eight exact observation methods, the D5 resource-to-method mapping, complete enumeration/by-ID/pin-path mechanisms, and source/Unavailable behavior. Two failure/attribution branches remain impossible or dishonest under the exact signatures. |
| DESIGN-P02-04 — wrong map kinds not representable | **CLOSED.** | `Hash`, `Array`, and `Unsupported(opaque token)` distinguish all valid kinds without exporting aya/numeric ABI. Key, value, and capacity mismatches use analogous equality-preserving opaque properties; decode failures remain sourced `Map` errors. |

### Locked aya/kernel feasibility

The proposed observation mechanisms exist in locked aya 0.13.1 and the pinned
kernel contract:

- `loaded_maps` and `loaded_programs` expose complete iterator results with
  map/program IDs and semantic metadata;
- aya's doc-hidden `loaded_links` exposes the kernel link information needed
  for an internal TCX link projection;
- map objects can be reopened by ID after pin removal while any kernel
  reference remains;
- program metadata exposes tag, name, type, and map relationships; and
- exact pin paths can be opened and compared to private object identity inside
  dataplane.

The handle-free `GuestTcxInventoryIdentity` is a sound ownership carrier after
successful capture. Its private mutex-shared receipts do not retain kernel
objects, its clones do not create a second owner, and its Drop has no effect.
Recorded IDs allow an unpinned object retained by a leaked FD to remain visible
after loader/adopted handles are released. The eight methods map one-to-one to
the eight accepted D5 fields; no raw iterator record, FD, kernel ID, map
layout, or ELF name crosses into control-plane.

The opaque schema tokens are deterministic and implementable. A private
discriminant can retain the locked aya/kernel identity for equality while the
public type exposes neither a constructor nor accessor and prints only
`Unsupported`. Distinct unexpected kinds or numeric properties remain unequal,
accepted schemas remain named semantically, and future/undecodable kinds do not
masquerade as Hash or Array. This closes DESIGN-P02-04 without adding an
adapter mirror of aya's non-exhaustive enum.

Once a valid inventory identity exists, source handling is also coherent:
iterator/open/read failures retain `Map`, `Program`, `Link`, or `Io`; semantic
schema/ambiguity/ownership disagreements remain typed source-less errors; D5
projects every `Err` to that family's `Unavailable`, retains the first direct
typed cleanup error, and continues all later families. Pin absence and local
Drop are explicitly insufficient to claim object absence.

### Finding DESIGN-P02-05 — Blocking: all-or-nothing `capture` loses the inventory object required after capture failure

**Severity:** Critical

**Evidence:**

- The exact signature is
  `GuestTcxInventoryIdentity::capture(...) -> Result<Self, GuestTcxError>`
  (`feature-delta.md:4651-4655`).
- The prose makes capture the first part of D5's
  `LoadProgramAndMaps` action and states that a capture failure returns the
  `TcxLoad` primary error (`feature-delta.md:4805-4810`). Therefore the `Err`
  branch returns no `GuestTcxInventoryIdentity` to the real D5 I/O.
- The accepted D5 algorithm still runs explicit cleanup and then all eight
  `count_tcx` observations after *every* primary setup failure, including the
  first setup action. The only exact observation methods are inherent methods
  requiring `&GuestTcxInventoryIdentity` (`feature-delta.md:4657-4671`).
- Capture itself performs the global map/program/link enumerations that may
  fail. Its source-bearing `GuestTcxError` is moved into the owner as the
  primary error and is not cloneable. No exact dataplane method or returned
  partial identity remains from which later families can produce their own
  honest `Unavailable` results.

**Reachability and consequence:** A real `loaded_maps`, `loaded_programs`, or
`loaded_links` enumeration error occurs before any BPF load effect. D5 must
still observe all eight families and distinguish observed zero from
unavailability. Under the proposed signature, the crafter must invent a retry,
construct a placeholder identity, manufacture family errors in control-plane,
skip the observations, or infer zero because load did not start. None is named
by D12, and the last two directly contradict D5's complete/no-fabricated-zero
contract.

**Bounded disposition:** Pin the exact dataplane-owned capture-failure
disposition that leaves D5 able to execute all eight observations and retain
the genuine enumeration cause without fabricating or duplicating it. This
review does not choose whether that is represented by a partial capture result,
an identity-contained unavailable state, or another exact shape. The public
surface and roadmap must be updated before DELIVER; the crafter may not infer
the mechanism.

### Finding DESIGN-P02-06 — Blocking: the pre-ID fallback counts an indistinguishable unrelated object as owned residue

**Severity:** High, blocking under source-honest ownership and complement rules

**Evidence:**

- For a load failure before exact object IDs are recorded, D12 compares global
  post-state to the pre-load ID baseline and filters by accepted
  name/schema/relationship. It explicitly says one unique new match is counted
  as retained owned residue (`feature-delta.md:4843-4852`).
- A second privileged loader can create the same production object after the
  baseline. Its program tag/name/type, map name/schema/program relationship,
  and absence from the baseline are byte-for-byte indistinguishable from a
  partial object leaked by this probe. No pin path, FD, or recorded object ID
  associates that candidate with the inventory identity.
- `InventoryAmbiguous` is used only for multiple/concurrently ambiguous
  candidates. A single indistinguishable foreign candidate is therefore
  reported as an owned count rather than `Unavailable`.

**Reachability and consequence:** The accepted security model already treats
privileged kernel-object mutation as observable drift, and the review request
requires global enumeration to attribute only this probe/owner. Concurrent
same-object loading is not made impossible by the kernel API; the single-serve
deployment rule prevents a legitimate second Overdrive owner but cannot turn a
foreign privileged object into this probe's receipt. The proposed fallback can
therefore produce a false non-zero owned complement. It fails closed, but its
evidence is not source-honest and it violates the exact owner/object scope of
the D5 count.

**Bounded disposition:** A candidate without an exact recorded ownership
receipt must not be counted as this probe merely because it is the sole
baseline delta with matching semantics. Make the no-receipt attribution rule
exact and conservative—returning a typed unavailable/ambiguity disposition
unless ownership is actually proved—or identify an implementable ownership
receipt that distinguishes the object. The architect owns the exact choice;
do not add a public kernel ID, FD, generic enumeration record, or new owner.

### Eight-family and evidence-layer assessment

| D5 family | Proposed D12 observation | Iteration-3 assessment |
| --- | --- | --- |
| Endpoint map | `observe_endpoint_maps` | Exact after a successful ownership receipt; retained unpinned map is visible by recorded ID/global enumeration. |
| Counter map | `observe_counter_maps` | Same; accepted schema and unsupported schema are now honest. |
| Endpoint entry | `observe_endpoint_entries` | Recorded ifindices are read through the private ABI from the exact owned map; map-family residue independently prevents a false empty total complement. |
| TCX program | `observe_tcx_programs` | Exact ID/tag/name/type/map relationship is implementable after receipt publication. |
| TCX link | `observe_tcx_links` | Exact link/program/ifindex/TCX-ingress relationship is available through the dataplane-private link projection. |
| Endpoint-map pin | `observe_endpoint_map_pins` | Exact path and object identity; ENOENT alone is zero, wrong object is `OwnershipMismatch`. |
| Counter-map pin | `observe_counter_map_pins` | Same. |
| TCX-link pin | `observe_tcx_link_pins` | Same, using the recorded planned path and private link identity. |

The updated roadmap correctly adds dataplane source-local and Lima real-kernel
inventory tests and extends S-ND295-00 to cover clean complements plus retained
unpinned program/map/link objects. Those tests can prove the successful-capture
path, opaque equality, source propagation, and all eight mappings. They cannot
choose the missing capture-failure contract or make an indistinguishable
foreign object owned, so DESIGN-P02-05/06 remain upstream blockers rather than
test-authoring tasks.

### D13 and architecture-drift audit

D13 remains unchanged and intact. `VmHostState` is still an existing mandatory
production dependency; the phase event remains non-persisted operational
telemetry in the accepted family; the seeded state is production-reachable;
and native metal remains the independent substrate layer. No D12 change adds a
second shared-network owner, lifecycle trait, generic command API, Sim raw
adapter, persistence/recovery mechanism, or new deployment boundary.

The corrected D12 still fits the accepted dataplane ownership and dependency
direction, so brief/ADR non-amendment remains justified if the two remaining
corrections stay inside this same semantic adapter boundary. Any resolution
that changes owner/dependency/D5 evidence policy would require the affected
architecture SSOTs to change.

### Mechanical and traceability checks

| Check | Iteration-3 result |
| --- | --- |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; repository-documented length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no errors. |
| JSON and diff hygiene | PASS — `jq empty` and `git diff --check`. |
| Design references | PASS — every revised reference resolves. |
| Dependencies | PASS — the phase-02/03 graph remains acyclic and D13 ownership is unchanged. |
| Phase-02 scenario ownership | PASS — S00/02..04/06..13, S14..19, and S20..26 remain uniquely owned by 02-01/02-02/02-03; S05 remains at 03-01. |
| Global scenario uniqueness | Unchanged pending state — pre-existing S27/S28 prerequisite duplicates remain outside this remediation. |
| Step history | PASS — `c60cdd3b` and `7ec987a8` remain rejected `02-01` history; no DES event is relabelled or backfilled. |
| Mutation testing | NOT RUN — correctly deferred to the final DELIVER gate. |

### Iteration-3 findings and dispositions

| Finding | Status | Required bounded disposition |
| --- | --- | --- |
| DESIGN-P02-03 — missing D5 inventory boundary | PARTIALLY CLOSED | Eight exact observation methods now exist, but their capture-failure and no-receipt attribution branches remain incomplete through DESIGN-P02-05/06. |
| DESIGN-P02-04 — unrepresentable wrong map kinds | CLOSED | Opaque equality-preserving unsupported tokens are sufficient, deterministic, and do not leak raw ABI. |
| DESIGN-P02-05 — capture failure leaves no identity for mandatory continued observations | OPEN, blocking | Pin an exact source-honest dataplane disposition that still permits all eight D5 observations. |
| DESIGN-P02-06 — a unique unreceipted baseline-delta match is falsely counted as owned | OPEN, blocking | Require proof of ownership or a typed unavailable/ambiguity result; do not expose raw identity or invent it in DELIVER. |

## Iteration 3 final verdict

# CHANGES_REQUIRED

The opaque schema correction is approved and the successful-capture inventory
path is implementable, complete, and source-honest. D12 still has two exact
failure-path gaps: `capture` can return no identity while D5 must continue all
eight observations, and the pre-ID fallback attributes a unique but
indistinguishable foreign object to this owner. D13 remains approved. The
roadmap stays `validation.status = pending`; DISTILL/DELIVER may not resume
revised `02-01` until DESIGN-P02-05/06 are resolved and independently
re-reviewed.

## Iteration 4 — capture carrier and receipt-only attribution review

### Scope and prior-finding dispositions

Iteration 4 preserves all prior iterations and reviews only the corrected D12
capture/error-movement and attribution rules plus their roadmap/DISTILL
handoff. The approved D13 and opaque schema contracts are unchanged.

| Prior finding | Iteration-4 disposition | Evidence |
| --- | --- | --- |
| DESIGN-P02-05 — capture failure leaves no identity for continued observation | **CLOSED.** | `capture` now always returns a non-`Clone` carrier. `into_parts(self)` moves an observation-capable identity and the one disposition separately; D5 stores the identity before moving the genuine first enumeration error into the existing `TcxLoad` primary. |
| DESIGN-P02-06 — unique unreceipted candidate counted as owned | **CLOSED.** | Only an exact private ownership receipt may produce a nonzero owned count. With a complete baseline and no receipt, an empty candidate set is zero and every nonempty set—including a single semantic match—is `InventoryAmbiguous`/`Unavailable`. |

### Rust ownership and first-source analysis

The corrected carrier is implementable without cloning, wrapping, flattening,
or hiding the genuine lower source:

1. `capture` constructs one pure-data `GuestTcxInventoryIdentity` up front.
2. It exhausts maps, programs, and links independently in fixed order, storing
   each successful baseline and only an availability flag for a failed domain.
3. `GuestTcxInventoryCapture` owns that identity plus
   `Result<(), GuestTcxError>`; its fields are private and the carrier is not
   cloneable.
4. `into_parts(self)` consumes the carrier. The real D5 I/O moves the identity
   into its state first, then moves the disposition. On `Err`, that exact
   `Map` or `Program` value—with its original aya source—is moved once into
   `GuestNetworkError::Tcx { operation: TcxLoad, source }`.
5. The identity retains no error, `Arc<Error>`, string, or fabricated clone.
   Later observations consult only domain availability and return typed
   source-less `CaptureUnavailable { family }` where necessary.

This gives D5 the required primary error and an independently usable identity.
Later capture-domain failures are intentionally represented by availability
flags because D5's aggregate retains only the first direct primary/cleanup
source; they are not silently converted into successful observations. No
second lower source is fabricated or hidden behind a replacement error.

Skipping `GuestTcxProgram::load` when the capture disposition is `Err` is also
correct. It prevents new lifecycle effects after an incomplete baseline while
still allowing all eight post-cleanup observations to execute. Independent
successfully captured domains and exact pin paths remain observable; affected
domains are `Unavailable`, never inferred zero.

### Eight-family continuation and genuine-zero analysis

| D5 family | Capture dependency and result rule | Assessment |
| --- | --- | --- |
| Endpoint map | Maps baseline or exact later receipt | Failed baseline without receipt → `CaptureUnavailable`; receipt → exact-ID observation; complete baseline with no candidates → zero; any unreceipted candidate → `InventoryAmbiguous`. |
| Counter map | Maps baseline or exact later receipt | Same. |
| Endpoint entry | Maps baseline plus recorded endpoint-map receipt/ifindices | Same domain rule; by-ID private-ABI reads occur only when the owned map is identified. |
| TCX program | Programs baseline or exact later receipt | Failed baseline → `CaptureUnavailable`; receipt uses exact ID/tag/name/type/map relation; unreceipted candidate never counts. |
| TCX link | Links baseline or exact later receipt | Failed baseline → `CaptureUnavailable`; receipt uses exact link/program/target/TCX-ingress identity. |
| Endpoint-map pin | Exact planned path | Direct path observation remains possible even if global map capture failed; ENOENT is zero, unreceipted presence is ambiguous, receipted wrong identity is `OwnershipMismatch`. |
| Counter-map pin | Exact planned path | Same. |
| TCX-link pin | Exact planned path registered before mutation | Same. |

The rules are conservative and source-honest. `Ok(0)` requires a completed
observation and an empty owner-attributable candidate set. Local Drop, pin
absence alone, or missing bookkeeping never proves global object absence. A
retained unpinned object remains visible through global enumeration/by-ID
inspection after loader and adopted handles are released. Conversely, a
foreign object is not counted merely because its name/schema/tag resembles the
production object.

The pre-receipt partial-load rule is now exact: until dataplane reads and
commits an ownership receipt, a surviving object is unavailable evidence, not
an owned count. This is the only honest result because the kernel offers no
causal owner token that distinguishes one otherwise-identical unpinned loader
object from another privileged loader. `InventoryAmbiguous` fails closed
without corrupting the complement's owner/object semantics.

### Boundary and regression audit

The cumulative D12 surface is now necessary and sufficient:

- opaque non-`Clone` lifecycle handles own mutation state;
- the pure-data cloneable inventory identity owns only private receipts,
  baselines, planned paths, and availability state;
- eight closed observation methods supply exactly D5's eight TCX fields;
- unsupported map-kind/property tokens retain stable equality without raw aya
  or numeric ABI exposure;
- only the existing private real D5 I/O consumes the lifecycle/inventory
  boundary; and
- the public Sim owner remains limited to the high-level
  `SharedGuestNetworkOwner` port.

No new production owner, generic command/inspection API, persistence record,
recovery protocol, test-only host hook, or alternate cleanup owner was added.
Drop remains non-mutating with respect to pins; explicit owner-authored cleanup
and read-back remain authoritative. D6's five free operations and their source
mapping are unchanged.

D13 remains intact: the existing `VmHostState` dependency is mandatory in the
real helper signatures, the structured boot-phase event is non-persisted
operational telemetry, the seeded invariant fails against the current real
composition order, and native metal remains the independent VMM/kernel layer.
No revised D12 rule affects that boundary.

### DISTILL and roadmap handoff

The updated handoff now covers the previously missing failure branches:

- S-ND295-00 must prove clean eight-zero observation, exact positive counts
  for deliberately retained receipted objects, capture failure with once-moved
  first source and later `CaptureUnavailable`, continued independent-domain
  and pin observation, exact-path wrong ownership/schema, and unique/multiple
  unreceipted ambiguity;
- dataplane source-local tests prove opaque equality and every map-kind/
  property projection without exposing the private discriminants;
- Lima real-kernel inventory proves retained unpinned map/program/link
  visibility after handle release; and
- S10/S11/S12 and D13's seeded-plus-native S13 layers remain unchanged.

This is sufficient for the acceptance designer to author executable RED tests
without choosing an API or inventing an owner. Revised `02-01` remains blocked
until those DISTILL bodies are authored and independently approved.

### Mechanical checks

| Check | Iteration-4 result |
| --- | --- |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; documented length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK with no errors. |
| JSON/diff hygiene | PASS — `jq empty` and `git diff --check`. |
| References | PASS — every design reference resolves. |
| Dependency graph | PASS — phase-02/03 remains acyclic; no later-step behavior is consumed early. |
| Phase-02 traceability | PASS — scenario ownership and evidence layers remain aligned. |
| Step history | PASS — rejected commits and existing DES events remain historical `02-01` evidence with no relabel/backfill. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

The pre-existing whole-roadmap findings in `review-roadmap.md`, including the
S27/S28 prerequisite duplicates, remain outside this bounded DESIGN approval.
They, the still-missing DISTILL bodies, and `validation.status = pending`
continue to prevent DELIVER execution.

### Iteration-4 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 — exact TCX lifecycle boundary | CLOSED by cumulative D12. |
| DESIGN-P02-02 — S13 production-composed boundary | CLOSED by D13. |
| DESIGN-P02-03 — eight-family inventory boundary | CLOSED by cumulative D12. |
| DESIGN-P02-04 — unsupported map schema representation | CLOSED by opaque equality-preserving tokens. |
| DESIGN-P02-05 — capture-failure continuation/source movement | CLOSED by `GuestTcxInventoryCapture::into_parts`. |
| DESIGN-P02-06 — unreceipted ownership attribution | CLOSED by receipt-only nonzero and conservative ambiguity. |

No unresolved DESIGN finding remains in the bounded phase-02 remediation.
Brief/ADR non-amendment is justified because component ownership, dependency
direction, boot/recovery policy, persistence, and deployment remain unchanged;
the feature delta remains the exact implementation-facing API SSOT.

## Iteration 4 final verdict

# APPROVED

D-295-DISTILL-12 and D-295-DISTILL-13 now provide exact, implementable,
source-honest lifecycle, inventory, and production-composition boundaries.
DISTILL may proceed with the required S-ND295-00/10/11/12/13 work. The roadmap
must remain `validation.status = pending`, and no DELIVER step may resume until
DISTILL approval and the separate outstanding whole-roadmap review findings are
resolved.

## Iteration 5 — DISTILL testability remediation review

### Scope and disposition of previously approved boundaries

Iteration 5 preserves all prior iterations and reviews D12's private inventory
source, D-295-DISTILL-12A's allocation leaf-effect boundary, and D13's Sim
sweep-call RED oracle. The cumulative public D12 lifecycle/inventory contract
and D13 helper/telemetry contract approved in iteration 4 remain unchanged.

- D12's private `GuestTcxInventorySource`, real aya implementation,
  `capture_with_source`, and raw-to-semantic projections are **APPROVED**.
- D13's `SimSharedGuestNetworkOwner::with_sweep_host_state` and
  `SimSharedGuestNetworkSweepCall` RED oracle are **APPROVED**.
- D12A is correctly shaped as a private leaf adapter rather than a second
  workflow or owner, but its TAP postcondition surface is not yet sufficient
  for the accepted production identity. Two bounded DESIGN findings remain.

### Dataplane-private inventory source

The inventory injection preserves one production implementation and one
projection path. `GuestTcxInventoryIdentity::capture` delegates to private
`capture_with_source` with `AyaGuestTcxInventorySource`; later observations
retain that same stateless source privately. Source-local tests can inject raw
aya/numeric observations only inside `overdrive-dataplane::guest_tcx`, and both
tests and production use the same private `project_map_kind` and
`project_map_schema` functions.

No raw observation, source trait, kernel ID, FD, numeric schema, or opaque-token
constructor crosses the crate boundary. The source does not retain handles
between calls and therefore does not become an owner. It is a read-only leaf
adapter beneath the already-approved handle-free identity, not a test-only
production branch. This closes the DISTILL concern without changing D12's
public/doc-hidden contract or source semantics.

### D12A ownership, async, rollback, and cancellation analysis

D12A's placement and responsibility are otherwise correct:

- `GuestNetworkAllocationIo` is module-private, contains exact leaf effects and
  observations, and returns no completed provision/teardown result;
- `HostSharedGuestNetworkOwner` alone owns ordering, postcondition comparison,
  rollback choice, allocation publication, complement construction, and lease
  release;
- `HostGuestNetworkAllocationIo` is the sole real implementation over existing
  netlink, D9, D12, and D6 adapters;
- the pending unpinned `GuestTcxLink` lives only in the real leaf adapter
  between synchronous attach and pin calls, while the one shared TCX state
  remains the owner's existing state;
- `with_allocation_io` is module-private and `cfg(test)`-only; source-local
  tests invoke the real owner algorithm and inject only leaf results/call
  recording; and
- Sim continues to implement only the high-level application owner.

No mutex crosses an await. TCX/guard mutations are synchronous; TAP mutations
are awaited to completion. The production convergence task drains admitted
owner futures during normal shutdown, so ordinary cancellation does not abandon
an in-flight provision/teardown sequence. Abrupt process death leaves transient
or pinned residue for the accepted next-boot reclamation/sweep boundary. Normal
primary failure invokes owner-authored rollback; cleanup continues after a
leaf failure, retains the first typed cleanup error, holds the lease, and
permits an absence-idempotent retry. Drop is not treated as effect completion.

The provision and teardown order matches the accepted security boundary:
guard membership precedes endpoint/link publication; endpoint and link are
removed before guarded TAP deletion; member removal occurs only after TAP
deletion; exact complement precedes release-last. D12A does not introduce a
second action sequence, recovery policy, persistence owner, or compatibility
path.

### Finding DESIGN-P02-07 — Blocking: the TAP observation cannot prove persistence or the required VMM owner

**Severity:** High, blocking under exact postcondition and source-honesty rules

**Evidence:**

- The accepted `GuestNetworkFact::Tap` identity includes `persistent: bool`
  and `owner_uid: Option<u32>` in addition to name, ifindex, link kind, and up
  state (`feature-delta.md:2668-2677`).
- The existing real netlink observer intentionally distinguishes `Absent`,
  `Incompatible`, and `Persistent { up, owner_uid }`; a TUN, dummy, veth, or
  non-persistent same-name TAP is incompatible, and the owner UID is observable
  (`overdrive-netlink/src/client.rs:43-58,312-328,905-943`).
- D12A's exact `GuestNetworkAllocationTapObservation` contains only `present`,
  `ifindex`, `kind`, `master_ifindex`, and `up`
  (`feature-delta.md:5095-5104`). It cannot represent either persistence or
  actual owner UID.
- D12A nevertheless requires the host owner—not the leaf adapter—to compare
  semantic postconditions and return `PostconditionMismatch`
  (`feature-delta.md:5203-5210,5269-5282`).

**Reachability and consequence:** The production TAP is reopened by the
confined Cloud Hypervisor identity. A persistent TAP owned by the wrong UID, or
a non-persistent/same-name incompatible link, can have the same represented
ifindex/kind/master/up tuple as the desired attachment. The owner can therefore
return provision success without proving the accepted persistent/reopenable
identity, moving the failure after the network gate into VMM start or accepting
an unrelated device. Mapping the discrepancy to a leaf `NetlinkError` would
also violate the stated owner-authored semantic mismatch boundary.

**Bounded disposition:** Make D12A's exact leaf observation preserve enough
typed actual state for `HostSharedGuestNetworkOwner` to distinguish absence,
incompatible/non-persistent state, persistence, and the exact VMM owner and to
construct the existing expected/observed fact honestly. The architect owns the
exact private shape; do not invent it in DISTILL or DELIVER.

### Finding DESIGN-P02-08 — Blocking: the owner has no expected bridge ifindex for the promised exact-master comparison

**Severity:** High, blocking under implementation feasibility and testability

**Evidence:**

- D12A promises that the real owner observes the TAP's **exact** bridge master
  while down and again before final success (`feature-delta.md:5269-5278`).
- The leaf observation returns only the TAP's actual
  `master_ifindex: Option<u32>` (`feature-delta.md:5097-5104`).
- `GuestNetworkPlan` carries the desired bridge name, not its ifindex. The exact
  D12A trait has no bridge-identity observation/resolution method, and the
  exact `HostSharedGuestNetworkOwner` state added by D12A stores only the TCX
  state, allocation I/O, and per-allocation `{ ifindex, program_id }`
  (`feature-delta.md:5214-5239`).
- The design states that the leaf interface contains the host effects and the
  owner alone compares postconditions. Resolving the bridge directly from the
  owner would bypass the injected leaf boundary and make the source-local S11
  owner tests unable to drive the same comparison.

**Reachability and consequence:** A TAP enslaved to a different bridge still
returns `Some(master_ifindex)`. With no expected bridge ifindex/fact available,
the owner must accept any master, perform unsanctioned host I/O outside D12A,
or ask the scripted test leaf to pre-decide whether the master is correct. The
first breaks the topology contract; the second breaks the exact testability
boundary; the third transfers semantic comparison from the owner to the leaf
adapter.

**Bounded disposition:** Pin the exact private fact or leaf observation needed
for the owner to compare actual TAP master identity with the accepted shared
bridge identity at both read-back points. Keep the comparison in the one host
owner and the host lookup in the leaf adapter; do not add a public bridge port,
second owner, or test-authored success result.

### D13 RED-oracle analysis

The refined S13 RED is ordering-honest:

- `SimVmHostState::clone` shares the same Arc-backed state, and the invariant
  passes that same state to both the production helper and
  `with_sweep_host_state`;
- the snapshot is taken inside the actual `SharedGuestNetworkOwner::sweep_stale`
  port call, not in a parallel test task or callback;
- the default Sim owner behavior and existing `calls()` API remain unchanged;
- the recorded call index and host snapshot share one trace lock, preserving a
  deterministic relation to `CleanupComplement`;
- seeded run-directory/clone/scope residue with empty supervision is a real
  fresh-process reclamation input; and
- with only parameter plumbing plus the Sim observation binding, current
  sweep-before-reclamation order reaches the sweep call and observes residue,
  so the test fails on its host-state assertion and prints the seed.

Structured production events are no longer needed to establish RED. They are
a separate GREEN operational-telemetry obligation after the ordering fix, so a
missing event cannot masquerade as the original defect. Native metal remains
the independent proof of real VMM and kernel complements. The Sim addition is
an explicit adapter-sim evidence API, not a product hook or alternate boot
owner.

### Roadmap and mechanical checks

| Check | Iteration-5 result |
| --- | --- |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; repository-documented length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no errors. |
| JSON/diff/reference hygiene | PASS — `jq empty`, `git diff --check`, and every design reference resolves. |
| Dependency graph | PASS — phase-02/03 remains acyclic and no D12A/D13 behavior depends on a later step. |
| Scenario ownership | PASS for the bounded remap — S00/02..04/06..13, S14..19, and S20..26 retain unique phase-02 ownership; global S27/S28 prerequisite duplicates remain a separate pending roadmap finding. |
| Evidence boundaries | PASS except D12A TAP identity — private dataplane projection, source-local real-owner tests, Lima actual effects, Sim ordering, GREEN telemetry, and native metal remain distinct. |
| Step history | PASS — rejected `02-01` commits/DES events remain untouched and historical. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

### Iteration-5 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-06 | Remain CLOSED by approved cumulative D12/D13. |
| D12 private inventory source | APPROVED — production-used, crate-private, source-honest, and non-owning. |
| D13 Sim sweep-call RED oracle | APPROVED — same state, real port call, reachable residue, seed-bearing, telemetry-independent. |
| DESIGN-P02-07 — persistence/VMM-owner facts absent from D12A TAP observation | OPEN, blocking. |
| DESIGN-P02-08 — exact shared-bridge master identity unavailable to the owner | OPEN, blocking. |

Brief/ADR non-amendment remains justified if these corrections stay inside the
same private allocation leaf boundary. A change to owner, dependency direction,
public product API, recovery policy, or persistence would require the affected
architecture SSOTs to change.

## Iteration 5 final verdict

# CHANGES_REQUIRED

The private inventory source and ordering-honest S13 RED oracle are approved,
and D12A is correctly a leaf-effect adapter beneath the one real owner. Its TAP
observation is still insufficient: the owner cannot prove persistent/exact-UID
identity or compare the actual master ifindex with the accepted shared bridge.
The roadmap remains `validation.status = pending`; DISTILL/DELIVER may not
resume revised `02-01` until DESIGN-P02-07/08 are resolved and independently
re-reviewed.

## Iteration 6 — exact TAP and shared-bridge identity review

### Scope and prior-finding dispositions

Iteration 6 preserves all prior iterations and reviews only the D12A TAP/
bridge observation correction and its S11 roadmap/DISTILL handoff. All
previously approved D12, private inventory-source, D12A ownership/ordering,
D13 helper/telemetry, and D13 Sim RED-oracle decisions were rechecked for
regression.

| Prior finding | Iteration-6 disposition | Evidence |
| --- | --- | --- |
| DESIGN-P02-07 — persistence/VMM-owner facts absent | **CLOSED.** | The TAP observation now distinguishes Absent, Incompatible, and Persistent and preserves actual name, ifindex, semantic kind, persistence, up state, owner UID, and master. The owner compares the existing Tap fact against exact persistence and `OVERDRIVE_VMM_UID`. |
| DESIGN-P02-08 — expected shared-bridge master unavailable | **CLOSED.** | A private bridge observation returns actual semantic name/ifindex/kind. The owner refreshes it before both master checks, accepts only current Bridge-kind identity, and compares the TAP's actual master through existing structured facts. |

### Netlink capability and semantic projection

The supporting netlink surface is implementable from the existing
`RTM_GETLINK` reply and existing persistent-TAP parser:

- name and ifindex come from the link message identity;
- semantic kind comes from `IFLA_LINKINFO`;
- TAP/TUN type, persistence, and owner UID come from the existing typed TUN
  attributes;
- administrative state comes from link flags;
- master ifindex and MAC come from existing link attributes; and
- absence remains the typed no-link result while transport/decode failures
  retain `NetlinkError`.

`ObservedLinkIdentity`, `ObservedLinkKind`, and `PersistentTapIdentity` are
doc-hidden semantic adapter values. They expose no `LinkMessage`, NLA bytes,
raw kind string, ioctl record, or desired-state verdict. The richer persistent
TAP observation reuses the current parser rather than creating a second
classification truth; the current `TapLinkState` behavior need not change.
The generic link observation is an adapter read used by the private allocation
leaf, not a product driving port or ownership API.

### Owner/leaf responsibility and source honesty

The real leaf now returns actual state only:

- `Absent { name }` remains absence;
- `Incompatible` retains the actual kind, persistence when meaningful, owner,
  ifindex, up state, and master; and
- `Persistent` is produced only by the exact persistent-IFF_TAP classifier and
  retains actual owner/up/master identity.

`HostSharedGuestNetworkOwner` alone constructs expected and observed
`GuestNetworkFact::Tap` and `LinkMaster` values. It requires the plan's name,
TAP kind, persistence, exact `OVERDRIVE_VMM_UID`, stable ifindex, requested
down/up state, and exact current bridge master. Semantic discrepancies are
source-less `PostconditionMismatch`; only genuine observation transport/decode
failure is sourced `Netlink { operation: TapObserve, ... }`. The leaf does not
pre-author a boolean success result.

The bridge lookup follows the same discipline. The leaf returns Absent or the
actual name/ifindex/kind. The owner maps this through the new exact
`BridgeLinkIdentity` fact, rejects absent or non-Bridge state semantically, and
uses the current Bridge-kind ifindex as the following `LinkMaster` expectation.
Lookup failure alone is sourced `BridgeObserve`. This supplies the missing
expected identity without moving comparison or workflow ownership into the
adapter.

The additional `BridgeLinkIdentity` fact is a necessary implementation-facing
error projection because the existing full Bridge fact would require
fabricating MAC/address fields for a wrong-kind link. It remains within the
feature-delta error SSOT and adds no product command, wire field, persistence,
or operator surface.

### Bridge replacement race analysis

The expected bridge identity is refreshed immediately before each master
comparison and is stack-local. It is not cached or published as another owner:

1. after attach, the owner observes the current bridge and then the down TAP;
2. before final success, it observes the current bridge again and then the up
   TAP; and
3. each comparison requires the TAP's actual master to equal the immediately
   preceding Bridge-kind ifindex.

Deletion/recreation between the two checks changes the second expected
ifindex; a TAP still attached to the predecessor, or detached by predecessor
deletion, fails final comparison. Replacement between a bridge lookup and its
immediately following TAP lookup likewise produces a mismatching or absent
master unless the TAP has already been attached to the current semantically
Bridge-kind object. Mutation after the final read-back is the existing runtime
audit/recovery domain, not an excuse to cache the boot identity.

Full shared-bridge MAC/up/gateway convergence remains owned by
`converge_shared`/`audit_shared`; D12A uses bridge identity only to validate
per-allocation master membership. S11's fixed bridge-MAC attachment proof is
also supplied by the exact endpoint-value read-back before TCX publication.
The correction therefore closes the master race without duplicating shared-
bridge convergence inside the allocation leaf.

### DISTILL completeness

The S11 handoff is now complete enough to author RED tests without designing
surface:

- table Absent, non-TAP TUN, dummy/veth/other, non-persistent TAP, wrong owner
  UID, and exact persistent TAP observations;
- table absent bridge, wrong-kind same-name bridge, correct Bridge identity,
  and replacement to a different ifindex;
- exercise both the down-TAP and final up-TAP master comparisons;
- assert stable TAP ifindex, exact owner UID, and owner-authored Tap,
  BridgeLinkIdentity, and LinkMaster mismatches; and
- pair source-local real-owner ordering with Lima real-netlink/D9/D12 read-back
  before VMM start.

Scripted leaves return actual typed observations, never a completed result or
test-authored verdict. The existing S06 rollback and S12 continuation/
release-last matrices remain valid and unchanged.

### Regression and architecture audit

- D12's opaque lifecycle, capture carrier, receipt-only inventory, unsupported
  tokens, and private projection source are unchanged.
- D12A remains one module-private production-used leaf adapter under the same
  `HostSharedGuestNetworkOwner`; no second action owner or workflow appears.
- D13 still uses the same `SimVmHostState` at the real sweep port call for RED,
  separate structured GREEN telemetry, and independent native-metal evidence.
- No raw aya/netlink ABI crosses crate boundaries, no mutex crosses await, and
  no Drop path is promoted to normal effect completion.
- No persistence, recovery protocol, compatibility path, feature flag, daemon,
  or deployment boundary is added.

The doc-hidden semantic netlink values and `BridgeLinkIdentity` error fact are
exactly pinned in the feature delta. Component ownership, dependency direction,
boot/recovery policy, and product behavior remain those already recorded in
the brief and ADRs, so no brief/ADR amendment is warranted.

### Mechanical checks

| Check | Iteration-6 result |
| --- | --- |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; documented length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no errors. |
| JSON/diff/reference hygiene | PASS — `jq empty`, `git diff --check`, and every design reference resolves. |
| Dependency graph | PASS — phase-02/03 remains acyclic. |
| Phase-02 traceability | PASS — scenario owners and evidence layers remain aligned; S11 now enumerates the corrected TAP/bridge cases. |
| Step history | PASS — rejected `02-01` commits and DES events remain untouched historical evidence. |
| Mutation testing | NOT RUN — correctly deferred to the final DELIVER gate. |

The separate whole-roadmap findings, global S27/S28 prerequisite duplicates,
and still-required DISTILL bodies keep `validation.status = pending` and
continue to block DELIVER.

### Iteration-6 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-06 | Remain CLOSED. |
| D12 private inventory source | Remains APPROVED. |
| D12A ownership/order/rollback boundary | APPROVED. |
| D13 Sim sweep-call RED oracle | Remains APPROVED. |
| DESIGN-P02-07 — persistent TAP/VMM owner identity | CLOSED by exact typed TAP observation. |
| DESIGN-P02-08 — current shared-bridge master identity | CLOSED by refreshed semantic bridge observation and owner comparison. |

No unresolved DESIGN finding remains in the bounded phase-02 remediation.

## Iteration 6 final verdict

# APPROVED

The corrected D12A TAP/bridge observations are exact, implementable,
source-honest, and keep all semantic comparison in the one production owner.
All previously approved D12/D12A/D13 boundaries remain intact. DISTILL may
proceed with S-ND295-00/10/11/12/13; the roadmap must remain pending and no
DELIVER step may resume until DISTILL approval and the separate roadmap-level
findings are resolved.

## Iteration 7 — D-295-DISTILL-14 startup packet-probe review

### Scope and prior-finding disposition

Iteration 7 preserves iterations 1–6 and reviews only the proposed
D-295-DISTILL-14 amendment: one closed semantic TCP probe on the existing D12
opaque program, the amended D5 pre-close exercise order, and the distinct
D9/TAP detached-link oracle. D12's lifecycle/inventory contract, D12A's
allocation leaf, and D13's production-composed ordering evidence remain
unchanged.

F-16 is production-reachable and bounded exactly as D14 states. Ordinary
`run_server_with_obs_and_drivers` awaits the one composed owner's
`probe_startup` before publishing the supervisor or admitting work
(`overdrive-control-plane/src/lib.rs:2558-2582`). The real D5 binding currently
checks only attachment/endpoint/counter presence for `Classifier` and
`OriginalDestination`, and only attachment absence for `DetachedLinkGuard`
(`overdrive-control-plane/src/guest_network.rs:1020-1066`). It can therefore
return startup success without any packet-level classifier or D9 guard
observation, exactly matching implementation-review F-16. D14 changes only
that proven path; it does not absorb F-22/F-23/F-24 or any later supervisor
work.

### Exact API necessity and sufficiency

The proposed additive surface is necessary because the existing values divide
the needed authority deliberately:

- `GuestTcxProgram` alone simultaneously owns the loaded classifier and typed
  endpoint/counter maps (`guest_tcx.rs:689-697,718-860`). D6 exposes only
  semantic attachment/counter operations and no live program handle.
- the existing production `sys::prog_test_run` helper has a context-free raw
  packet/FD boundary, while the context-aware `__sk_buff` runner is confined to
  the BPF integration test (`overdrive-dataplane/src/sys/prog_test_run.rs:43-132`;
  `overdrive-bpf/tests/integration/guest_tcx_classifier_test_run.rs:51-146`);
- TAP traffic cannot return the TC action, output mark, and rewritten packet,
  while BPF test-run cannot traverse the bridge-family nft hook after link
  detach.

The selected method is also sufficient. Its six outcome accessors cover every
D5 semantic comparison without exporting a raw representation:

| Required observation | D14 semantic result |
| --- | --- |
| TC verdict | `verdict()` → `Accept`/`Drop`/`Unexpected` |
| proof mark | `mark()` → `None`/`Intercept`/`Accepted`/`Unexpected` |
| source-MAC preservation | `source_mac()` |
| bridge destination rewrite | `destination_mac()` |
| preserved destination IPv4 and TCP port | `original_destination()` |
| exact one-of-eight transition and wrap/decrease detection | `counters()` over eight semantic before/after records |

`GuestTcxTcpProbeInput` and the counter records are intentionally semantic data
that the sibling control-plane adapter can construct or inspect. The successful
`GuestTcxTcpProbeOutcome` itself has private fields and no constructor; the
control-plane cannot synthesize or replace a projected outcome. `Clone` permits
no fabrication because cloning still requires an existing dataplane-produced
value. The surface exposes no packet bytes, context field, action/mark/counter
number, FD, `bpf_attr`, command, repeat count, program lookup, protocol selector,
or arbitrary payload.

No smaller existing method can provide these observations, and none of the
rejected alternatives is needed. Adding the method to the already-owning
opaque program avoids a second loader, free packet operation, injectable
dataplane trait, or control-plane raw-BPF adapter. D6's five free operations,
`GuestTcxError`, `GuestTcxLink`, and `GuestTcxAdoptedState` remain exact.

### Aya and kernel implementability

The boundary is implementable with the locked substrate and no dependency
change:

- Aya 0.13.1 exposes immutable program-FD borrowing from a loaded `Program` and
  immutable typed array reads. `GuestTcxProgram` retains both the loaded
  `aya::Ebpf` and the typed counter array, so `&self` is the correct receiver;
  no mutable public handle or FD accessor is required.
- the repository already carries the complete `BPF_PROG_TEST_RUN` attribute
  layout and syscall error handling in production, and the existing TC
  integration runner proves the `ctx_in`/`ctx_out` layout needed to set
  `ingress_ifindex` and recover `mark`. D14 can keep the new context-aware raw
  helper and packet parser private to `guest_tcx`; it need not widen the
  pre-existing context-free `sys` helper.
- the production classifier reads `ingress_ifindex`, validates the endpoint's
  source IPv4/MAC, preserves the destination tuple, rewrites the destination
  MAC, writes the intercept mark, and advances only `Intercept`
  (`overdrive-bpf/src/programs/guest_tcx.rs:41-127`). One fixed valid TCP SYN is
  therefore a complete narrow positive boot probe; S08/09 retain the full
  positive and negative classifier partitions.
- the workspace already enables nix's `ioctl` and socket features, and
  control-plane already depends on nix. Reopening the named persistent TAP with
  `TUNSETIFF(IFF_TAP | IFF_NO_PI)` and binding the local UDP socket with
  `SO_BINDTODEVICE` requires no new crate, public netlink operation, or owner.

The production use of `BPF_PROG_TEST_RUN` is acceptable at this boot boundary.
The caller already has the privilege required to load and attach the same BPF
program; the synthetic run injects no host-network packet, retains no buffer or
FD, and the current classifier's only persistent mutation is one isolated
counter increment. The method executes once per semantic input, never resets a
counter, and returns the actual eight before/after pairs. `checked_sub` in D5
makes decrease/wrap fail closed. This adds bounded startup work only and no
steady-state or audit cost.

### Same-program identity, source, and cleanup

The amended order preserves one identity chain rather than substituting test
execution for attachment proof:

1. D12 loads one classifier/maps object, inserts the scratch endpoint, attaches
   first-ingress, retains `GuestTcxLink::program_id`, and pins that link.
2. Both peer-MAC and gateway-MAC inputs run through the still-live
   `GuestTcxProgram`; the method obtains the FD only from that same object and
   reads that object's counter map.
3. D5 closes the loader, adopts the exact recorded pins, queries the scratch
   interface/ingress attachment, and requires the complete returned program-id
   vector to equal the retained id. Probe success cannot satisfy adoption or
   query, and query cannot fabricate classifier semantics.

Fresh peer/gateway calls in both `Classifier` and `OriginalDestination` stages
also prevent one cached receipt from satisfying both stages. Each call has an
independent exact counter bracket and distinct destination facts.

The error chain remains source-honest. Counter reads remain `GuestTcxError::Map`;
missing owned objects remain `ObjectMissing`; raw syscall/context/output
transport remains `GuestTcxError::Io`. The real D5 adapter wraps a typed
`GuestTcxError` or `BridgeGuardError` with `std::io::Error::other(error)`, so
`GuestNetworkError::Io { operation: StartupProbe }` retains the typed error and
its lower source rather than a string. Semantic disagreement remains the
existing stage-specific `PostconditionMismatch`; no error variant is added.

Moving the two classifier stages before normal loader close does not create an
unreachable cleanup branch. D12 already specifies that
`GuestTcxAdoptedState::unpin_link` unpins the exact planned link and is
absence-idempotent even when the state began with `for_inventory`
(`feature-delta.md:4909-4928`). D14 requires the real I/O to create that one
handle-free state lazily at the first unpin when normal adoption was not
reached. The existing public method can privately open/validate/unpin the
receipted planned path and return the opaque unpinned link for the existing
detach action; no `adopt_link` success claim or new action is needed.

The current partial implementation's `unpin_link` returns `Ok(None)` when its
optional opened handle is empty (`guest_tcx.rs:1000-1014`), so GREEN must bring
that body into compliance with its already-approved D12 semantics for this new
reachable branch. This is implementation work within the exact existing
method, not an API or DESIGN gap. Normal and cleanup-phase
`close_loader_handles` remain infallible and absence-idempotent; complete
reverse cleanup and all fifteen observations still run after every returned
error or false result.

Both new packet operations are synchronous once their async `exercise` future
is polled. They contain no `.await`, spawn, callback, retained future, or lock
across an await, so cancellation cannot bisect one BPF test run or the bounded
TAP/counter/socket observation. Cancellation before the first poll or after a
ready return does not invent a partial packet-operation receipt. The broader
already-approved D5 setup/cleanup ownership is unchanged.

### Detached-link oracle and evidence-layer separation

The detached-link mechanism is non-vacuous and distinct from the classifier
probe:

- D6 first proves the scratch ingress attachment vector is empty and snapshots
  all eight classifier counters.
- D9's synchronous generation-bracketed `observe` returns the actual exact
  bridge-family inventory and its anonymous `DefaultDrop` packet/byte counter
  (`overdrive-netlink/src/nft.rs:2963-3015,3510-3609`). Requiring one exact
  owned occurrence prevents a different rule/counter from supplying evidence.
- writing one valid marker-bearing Ethernet/IPv4/UDP datagram through the
  reopened persistent TAP enters the host as traffic from the exact managed
  member. The bridge-bound UDP socket uses the kernel-selected port, eliminating
  a stale fixed-port dependency.
- exact packet delta one and positive byte delta prove arrival at the guard;
  unchanged classifier counters prove the detached classifier did not run; and
  continuous nonblocking `WouldBlock` with no received datagram proves no host
  delivery. Any extra scratch traffic, duplicate/missing counter, wrap/decrease,
  guard conflict, reappeared link, or delivery fails closed.

The scratch bridge/TAP has no VMM or other producer, and the loop uses a
monotonic deadline capped at 250 ms. It waits for the asynchronous kernel
counter transition instead of relying on an immediate post-write sample, while
the exact-one requirement prevents ambient traffic from being ignored. The
poll is boot-only and owns local RAII socket/TAP FDs; it adds no detached task or
runtime flake-prone timer assertion.

There is no evidence collapse:

- D14 boot probes only the two narrow positive TCP semantics needed for source-
  honest production admission. S08/09 still own the complete TCP/gateway/ARP,
  malformed, spoof, miss, truncation, direct-bypass, exact-counter, and external
  no-escape partitions.
- D14's scratch detached stage proves one real D9 counter transition plus host
  no-delivery before admission. S10 still owns an ordinary provisioned
  attachment, external link loss, peer-TAP and host no-escape, and the exact
  production audit cause.
- source-local raw-result/projection tables and D5 call-order/error tables are
  necessary but cannot stand in for the named Lima ordinary-boot body. Public
  Sim, the test-only BPF runner, an ignored placeholder, a fabricated outcome,
  or a direct private-runner call is explicitly excluded.

The D14 handoff is sufficiently exact for revised DISTILL: it names the pure
projection body and required `/// CONTRACT_SHAPE: pure-function.` marker; the
real-owner order/source/cleanup body; the active Lima ordinary-boot body through
`HostSharedGuestNetworkOwner::probe_startup`; the semantic mismatch and lower-
source refusal matrix; post-close exact adoption/query; real detached D9/TAP/
UDP observation; empty complement; BootClosed/no-publication; and the retained
independent S08/09/S10 evidence. DISTILL may author those bodies without
choosing a new API or owner.

### Architecture, roadmap, and mechanical checks

D14 adds no component, owner, port, dependency edge, technology, persistence,
system of record, recovery policy, supervisor behavior, or product-facing API.
It is one doc-hidden method and five semantic values on the existing D12
dataplane boundary plus implementation inside the existing private D5 adapter.
Raw classifier packet/context/syscall/FD/numeric ABI remains below that
boundary. Brief, C4, and ADR-0114/0115/0122/0124 remain correct; the exact API
belongs in the feature-delta SSOT.

| Check | Iteration-7 result |
| --- | --- |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; existing length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no errors. |
| JSON/diff/reference hygiene | PASS — `jq empty`, `git diff --check`, balanced Markdown fences, and all local feature-delta links resolve. |
| Locked dependency feasibility | PASS — `cargo metadata --format-version 1 --locked --no-deps`; Aya remains `^0.13`, aya-ebpf `^0.1.1`, nix `^0.30` with required existing features. |
| Dependency graph | PASS — 11 known steps, no missing dependency and no cycle. The existing S27/S28 prerequisite duplication remains the separately recorded whole-roadmap issue; D14 introduces no scenario duplicate. |
| External validity | PASS — ordinary boot invokes the real owner and the revised verification list names the active real-adapter Lima body. |
| API/implementation coupling | PASS for this bounded amendment — the exact doc-hidden signature is the accepted feature-delta contract required by repository policy; behavioral kernel evidence remains independently specified. |
| Unit/integration boundary | PASS — pure private projection, real owner algorithm, Lima kernel boot, S08/09 classifier partitions, and S10 ordinary detached-link behavior remain separate. |
| Step history | PASS — rejected `02-01` commits and DES events remain untouched historical evidence; the original crafter must run a fresh same-step cycle only after all upstream gates approve. |
| Roadmap execution status | Correctly BLOCKED — `validation.status = pending` until revised DISTILL and roadmap review approve the new evidence. DESIGN approval alone grants no DELIVER authority. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

### Iteration-7 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-08 | Remain CLOSED. |
| D12 opaque lifecycle/private source/inventory | Remains APPROVED. |
| D12A allocation leaf and exact TAP/bridge identity | Remains APPROVED. |
| D13 same-host sweep RED oracle and independent evidence layers | Remains APPROVED. |
| DELIVER-02-01-F16 startup packet exercise | CLOSED at DESIGN by D14's exact semantic program method, amended D5 order, and separate D9/TAP oracle; implementation and executable proof remain pending downstream. |

No unresolved DESIGN finding remains in the bounded D14 remediation. The noted
current `unpin_link` body is a directly reachable GREEN obligation under the
already-approved D12 method semantics, not authorization for another method,
variant, action, loader, or owner.

## Iteration 7 final verdict

# APPROVED

D-295-DISTILL-14 is necessary, sufficient, Rust/kernel-implementable, and
source-honest. It closes F-16 at the DESIGN boundary without reopening or
changing D12/D12A/D13, and it preserves the S08/09/S10 evidence partitions.
Revised DISTILL and independent roadmap validation remain mandatory; step
`02-01` is not executable while `validation.status = pending`.

## Iteration 8 — D14 DESIGN-to-DISTILL translation review

### Scope and translation inventory

Iteration 8 preserves the iteration-7 DESIGN approval and reviews only its
translation into the three newly authored D14 bodies, their RED scaffolds, the
revised DISTILL documents, and the matching `02-01` roadmap handoff. No
production behavior, rejected `02-01` history, DES event, or prior D12/D12A/D13
decision is re-reviewed.

The translation correctly preserves the exact public/doc-hidden shape:

- `GuestTcxProbeVerdict`, `GuestTcxProbeMark`,
  `GuestTcxProbeCounterObservation`, `GuestTcxTcpProbeInput`, and
  `GuestTcxTcpProbeOutcome` match the approved derives, fields, and type homes;
- the outcome fields remain private and expose exactly the six approved
  accessors;
- `GuestTcxProgram::probe_tcp_intercept(&self, GuestTcxTcpProbeInput)` has the
  exact approved return type and is an explicit RED panic, not an invented
  implementation;
- `RawGuestTcxTcpProbeResult` and `project_tcp_probe_result` are private to
  `guest_tcx.rs`; no raw packet, skb, FD, action/mark number, counter index,
  syscall attribute, trait, free operation, constructor, or error variant is
  exported; and
- the control-plane additions are confined to `#[cfg(test)]` private fixtures.
  They do not add another owner, loader, production seam, or public packet
  surface.

The source-local owner body also captures the approved coarse D5 order: setup;
`Classifier`; `OriginalDestination`; normal loader close; adoption/query;
unpin/detach; distinct detached-guard stage; then unconditional reverse cleanup
and all fifteen observations. Its pre-close failure suffix places the one lazy
handle-free adopted-state preparation inside the first unpin action and retains
the typed `GuestTcxError` beneath `StartupProbe` I/O. Those are valid RED
obligations.

Three blocking translation findings remain.

### Finding DESIGN-P02-09 — Blocking: no body forces the real D5 adapter to validate the six semantic outcome fields

**Evidence:**

- Approved D14 requires D5 to refuse every wrong verdict, mark, source MAC,
  destination MAC, original destination, non-Intercept counter delta, missing
  Intercept delta, and counter decrease/wrap. The approved Lima body was also
  required to prove BootClosed refusal for each semantic mismatch or lower
  failure (`feature-delta.md:5242-5252,5268-5277,5360-5385`).
- The dataplane body calls only the private pure projection
  (`guest_tcx.rs:1576-1733`). It proves that raw fields are represented in an
  outcome, but never drives the control-plane predicate that decides whether
  that outcome passes a D5 stage.
- The control-plane body's `PacketProbeIo::exercise` fabricates its own
  peer/gateway call records and then returns one generic `Ok(false)` or typed
  error per stage (`guest_network.rs:3488-3507`). It cannot construct a
  `GuestTcxTcpProbeOutcome`, does not invoke
  `RealSharedGuestNetworkScratchIo`, and therefore proves only that the owner
  reacts to a pre-decided stage boolean.
- The Lima body is healthy-path only (`shared_guest_network_startup.rs:479-545`).
  It never makes any one semantic field wrong and never observes a real-adapter
  semantic refusal. The older composed Sim body proves only caller reaction to
  an already-assembled owner-port error.

**Reachability and consequence:** A DELIVER implementation may call the real
opaque probe but omit any one of the mark/MAC/original-destination/counter
comparisons. The pure projection body, scripted owner-order body, healthy Lima
body, and existing Sim refusal body can all still pass. The acceptance set
therefore does not enforce the exact source-honest boundary it claims to hand
to the crafter.

**Required bounded disposition:** Add RED evidence that reaches the actual D5
semantic decision for every finite mismatch class and the exact lower-source
class already specified by D14. Do not add a public outcome constructor, raw
runner, FD accessor, injectable dataplane trait, second adapter, or test-only
product hook. If the approved surface cannot make those failures reachable,
return that testability gap to DESIGN rather than inventing API in DISTILL.

### Finding DESIGN-P02-10 — Blocking: the pure projection table does not cover malformed complete output

**Evidence:**

- D14 and the roadmap require both short **and malformed** returned-packet
  projection, with `None` for the affected semantic destination
  (`feature-delta.md:5236-5240,5360-5366`; roadmap `02-01` criterion 14).
- `raw_tcp_probe_output` always builds the same IPv4/TCP-shaped 38-byte value.
  The negative rows truncate it only to lengths 0, 6, 12, and 34
  (`guest_tcx.rs:1537-1553,1698-1715`).
- The wrong-source row is still a structurally valid IPv4/TCP output. There is
  no complete-length row for a wrong EtherType, IPv4 version/IHL, non-TCP
  protocol, or unavailable destination-port boundary.

**Reachability and consequence:** A projection that reads fixed offsets from
every buffer of length at least 38, without validating the declared packet
shape, satisfies the authored table while violating D14's malformed-output
contract. The revised DISTILL documents nevertheless mark C6a and malformed
output complete.

**Required bounded disposition:** Extend the existing private pure table with
the finite malformed complete-output and destination-boundary rows required by
the approved projection contract. This needs no API, owner, real-I/O fixture,
or architecture change.

### Finding DESIGN-P02-11 — Blocking: the Lima witness races transient scratch state and is not serialized with the selected host-kernel tests

**Evidence:**

- The ordinary-boot body starts an unsynchronised monitor thread, polls every
  1 ms, calls `run_server`, and stops the monitor immediately when `run_server`
  returns (`shared_guest_network_startup.rs:114-159,488-496`).
- It then requires the monitor to have sampled the short-lived counter pin,
  observed classifier maximum exactly four, and completed D9 observations both
  before and after the one packet (`shared_guest_network_startup.rs:498-516`).
  There is no handshake that the monitor started before setup, no receipt that
  it observed either transition before cleanup, and no post-boot wait because
  the scratch resources are correctly gone by then.
- The `02-01` verification command selects this body together with S10/S11/S12/
  S13 real-host bodies in the same nextest run. The repository's cross-process
  `host-kernel-shared` group is the required mechanism for fixed global kernel
  names, but `.config/nextest.toml` has no override covering
  `shared_guest_network_startup`. In-source `serial_test` would not serialize
  nextest processes.

**Reachability and consequence:** A correct production probe can create,
increment, and clean its isolated pins/guard before the monitor is scheduled or
between two samples, producing a false test failure. Conversely, another
selected real-host test can collide with the fixed scratch/production bpffs,
bridge, TAP, or nft names. This makes the mandatory Lima gate scheduler- and
suite-order-dependent rather than repeatable kernel evidence.

**Required bounded disposition:** Make the ordinary-production-composition
oracle deterministic using only the already-approved semantic/read-only
surfaces, and place every selected fixed-name shared-kernel body in the existing
cross-process serialization domain. Do not add a product event, timing hook,
public state accessor, sleep in production, or second owner to coordinate the
test.

### Evidence-layer and documentation disposition

The intended layer split remains correct and must be preserved during
remediation:

- the dataplane private table owns raw-to-semantic projection only;
- the private D5 table owns owner order, source retention, cleanup continuation,
  and the fifteen-family complement;
- ordinary Lima boot owns the real opaque-program call, post-close same-program
  adoption/query, real D9/TAP/UDP guard traversal, and cleanup;
- S08/09 retain the complete classifier partition and external no-escape
  evidence; and
- S10 retains ordinary provision, external detach, peer/host no-escape, and
  exact audit cause.

No authored body improperly substitutes Sim or the test-only BPF runner for
Linux, and no production effect is installed by the new Lima fixture. The
findings concern missing enforcement and determinism inside that otherwise
correct partition, not a request to merge layers or broaden scope.

Until the three findings close, the revised prose overstates its evidence:
`test-scenarios.md` cannot mark C6a/C6b and the D14 executable audit complete,
and `red-classification.md` cannot describe the Lima witness as exact. The
roadmap is correct to retain `validation.status = pending`; step `02-01` remains
non-executable.

### Mechanical checks

| Check | Iteration-8 result |
| --- | --- |
| Exact D14 API/visibility | PASS — only the approved five semantic values, six outcome accessors, and one opaque-program method were added; raw scaffolds remain private. |
| Production implementation prohibition | PASS — D14 production methods are explicit RED panics; test-only fixtures are `#[cfg(test)]` or integration-crate local. |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; existing count/length warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no validator errors. |
| JSON/diff hygiene | PASS — `jq empty` and `git diff --check`. |
| Contract Shape markers | PASS — the live pure projection body carries the exact `/// CONTRACT_SHAPE: pure-function.` line; the two stateful bodies declare bounded-change. |
| D5 cleanup order | PASS at the scripted owner layer — normal and pre-close-failure expected sequences include exact cleanup continuation and all fifteen observations. |
| Semantic refusal coverage | FAIL — DESIGN-P02-09. |
| Malformed projection coverage | FAIL — DESIGN-P02-10. |
| Lima determinism/serialization | FAIL — DESIGN-P02-11. |
| Roadmap execution status | Correctly pending; no DELIVER authority. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

### Iteration-8 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-08 | Remain CLOSED. |
| D14 architecture/API decision | Remains APPROVED; no API or ownership defect was introduced by the translation. |
| DESIGN-P02-09 — actual D5 semantic mismatch enforcement is not driven | OPEN, blocking. |
| DESIGN-P02-10 — malformed complete-output projection is absent | OPEN, blocking. |
| DESIGN-P02-11 — Lima witness is timing-racy and cross-process-unserialized | OPEN, blocking. |

## Iteration 8 final verdict

# CHANGES_REQUIRED

The translation preserves D14's exact API, ownership, raw-ABI fence, cleanup
order, and evidence-layer separation, but it does not yet enforce every
semantic refusal and its mandatory Lima proof is not deterministic. Remediate
the three bounded findings and re-review before independent roadmap approval or
DELIVER step `02-01`.

## Iteration 9 — D-295-DISTILL-14A validation and deterministic-observation review

### Scope and prior-finding dispositions

Iteration 9 preserves D14's iteration-7 public/doc-hidden API approval and
reviews only proposed D-295-DISTILL-14A: the production-used module-private
semantic validator, the four-name/five-event non-persisted startup trace, D5
order supersession, malformed projection handoff, and deterministic Lima/
nextest requirements. No code, test, DES log, prior D12/D12A/D13 decision, or
later supervisor behavior is reviewed or authorized here.

| Iteration-8 finding | Iteration-9 disposition | Evidence |
| --- | --- | --- |
| DESIGN-P02-09 — actual D5 semantic mismatch enforcement is not drivable | **CLOSED by D14A.** | `GuestNetworkTcpProbeObservation` is constructible only inside the control-plane module, while `from_outcome` is the sole production mapping from opaque accessors. The exact production-used validator makes every finite mismatch and one source-bearing lower error directly testable without making the successful outcome constructible downstream. |
| DESIGN-P02-10 — malformed complete-output projection is absent | **CLOSED at DESIGN handoff.** | D14A explicitly requires complete-length wrong EtherType, IPv4 version/IHL, non-TCP protocol, and unavailable destination-port rows in the existing dataplane-private pure table. No API or owner change is introduced. |
| DESIGN-P02-11 — transient Lima monitor and serialization | **CLOSED by D14A.** | The monitor/polls are rejected and replaced with synchronous production completion events captured across awaited ordinary boot. The whole `overdrive-control-plane` integration binary already has a committed `host-kernel-shared` override in `.config/nextest.toml`; D14A correctly requires preserving and mechanically verifying it rather than inventing source-level serialization. |

The iteration-8 statement that the whole integration binary lacked a nextest
assignment was factually incomplete: `.config/nextest.toml` already contains
`package(overdrive-control-plane) & binary(integration)` →
`host-kernel-shared`. The monitor race finding remains valid; D14A removes that
race and pins verification of the existing cross-process assignment.

### Private validator exactness and source semantics

The proposed boundary is necessary and no broader than the translation gap:

```text
validate_guest_tcx_tcp_probe(
    requirement,
    expected,
    observed Result<private semantic observation, GuestTcxError>,
) -> io::Result<Passed(actual Intercept bracket) | Mismatch(exact private cause)>
```

All supporting values and the function remain module-private. The observation
contains only D14's already-approved semantic outcome fields. It carries no raw
packet, skb context, FD, program handle, action/mark number, counter slot,
syscall attribute, owner authority, or persistence identity. Source-local tests
may construct it, but downstream crates still cannot construct a
`GuestTcxTcpProbeOutcome`; production obtains an observation only through the
six approved outcome accessors.

The validation order is deterministic and sufficient:

1. `Accept` verdict;
2. `Intercept` mark;
3. exact present source MAC;
4. exact present bridge destination MAC;
5. exact present original destination for `OriginalDestination` only;
6. each of the eight counter identities at its canonical array index;
7. checked non-decreasing before/after bracket; and
8. `Intercept +1`, every other counter `+0`.

The mismatch taxonomy retains precisely the information needed to distinguish
those failures: observed verdict/mark, expected and optional observed MAC/
destination, counter index and semantic identities, actual decreasing bracket,
or expected/observed delta. `CounterDecrease` honestly represents both an
ordinary decrease and u64 wrap; `CounterDelta` is evaluated only after
`checked_sub` succeeds. `Passed` returns the actual Intercept bracket used by
the later trace and does not normalize, reset, or invent a count.

`Classifier` intentionally does not compare original destination; D14 assigns
that additional check to the fresh `OriginalDestination` stage. Both stages
still validate verdict, mark, MACs, and all counters for peer and gateway
inputs. This preserves the approved division rather than weakening a field.

An observed `GuestTcxError` becomes
`std::io::Error::other(error)`. The typed `GuestTcxError` and its lower source
therefore remain downcastable beneath D5's existing
`GuestNetworkError::Io { operation: StartupProbe }`. A semantic mismatch is not
fabricated as I/O: the real adapter converts `Mismatch` to `Ok(false)`, leaving
the owner to construct the existing stage-specific `PostconditionMismatch`.
No public error variant or parallel source taxonomy is added.

### Five-event production trace

D14A defines four closed event names that produce exactly five ordered events
on a successful startup probe:

| Order | Completion claim | Required actual fields |
| --- | --- | --- |
| 1 | TCP stage completed — `classifier` | retained program id; peer and gateway Intercept before/after brackets |
| 2 | TCP stage completed — `original_destination` | the same fields from two fresh validated calls |
| 3 | attachment reopened | the same retained program id; actual D6 revision; `program_count = 1`; scratch TAP ifindex |
| 4 | detached guard completed | the same retained program id; unchanged classifier Intercept bracket; actual D9 packet/byte brackets; `host_datagrams = 0` |
| 5 | cleanup observed | primary/cleanup failure flags; actual fully-observed/empty predicates; every one of the fifteen semantic scratch counts |

The emission fences are exact and source-honest:

- the real scratch adapter retains `GuestTcxLink::program_id` before `PinLink`
  consumes the opaque link;
- a TCP-stage event is emitted only after both peer and gateway outcomes pass
  the production validator, so no scripted boolean can manufacture completion;
- the attachment event follows loader close, fresh pin adoption, and a D6 query
  whose complete program-id vector equals exactly the retained id;
- the detached event follows attachment absence, exact D9 identity, checked
  packet delta one, checked positive byte delta, equality of every classifier
  counter, and zero UDP delivery; and
- the owner emits the cleanup event exactly once after cleanup constructs the
  complete fifteen-family complement and before choosing its existing return
  path.

The trace is also sufficient for a deterministic successful-boot receipt. The
four TCP calls must form one continuous Intercept sequence with delta one per
call. The detached classifier bracket begins at the final TCP value and remains
unchanged. Guard packets advance exactly one, bytes advance positively, and
host datagrams remain zero. The final event requires every family to be
`Observed(0)`, not an unavailable field or inferred absence. The first four
events carry one identical program id; cleanup deliberately carries resource
facts rather than pretending a released program identity is still live.

Failure semantics are honest: a mismatch or lower error emits no completion
for that stage; already completed earlier stages remain true history; attachment
or guard disagreement emits no corresponding event; and cleanup still emits
its actual result. Tracing has no return value, does not influence admission,
and cannot turn emission failure into product failure. Cancellation before a
completion emits no claim; cancellation after a synchronous completed effect
does not invalidate the already-true event.

### Determinism, ownership, and architecture audit

The tracing replacement is feasible with existing production composition. The
integration body installs the repository's existing minimal subscriber before
awaiting ordinary `run_server`. D14A emits every event synchronously inside
that awaited boot path; there is no detached monitor, poll interval, barrier,
callback, sleep, or transient post-return read. Filtering the captured trace by
the four exact names yields a stable five-event sequence after `run_server`
returns.

Cross-process isolation is independently correct. The committed whole-binary
nextest override places every `overdrive-control-plane` integration test in the
one-thread `host-kernel-shared` group. The added `cargo nextest show-config
test-groups` gate uses supported package/target/ignored/group/filter options and
proves the actual resolution; no in-process `serial_test` claim substitutes for
it.

The events extend existing `guest_network.shared_owner_*` operational
telemetry, not product state. They are not persisted, indexed, replayed,
queried by admission or recovery, or exposed through a public enum, row, port,
health repository, callback, or state accessor. The private validator is a
production decision function, not a test hook. D14A adds no component, owner,
dependency edge, system of record, recovery protocol, supervisor behavior, or
product-facing API. Feature-delta exactness is sufficient; brief, C4, and the
accepted ADRs remain unchanged.

### D5 supersession and DISTILL/environment handoff

D14A preserves D14's single normative owner order and makes the transition
requirement explicit: attach/pin → fresh peer/gateway `Classifier` and
`OriginalDestination` validation → close → adopt/query the same id → detach →
distinct D9/TAP guard proof → reverse cleanup/all-fifteen observation. Existing
D5 tables must be rewritten to that one order; the obsolete
close/adopt/query-before-exercise sequence is not retained as an alternative.
Pre-close failure still lazily constructs only the approved handle-free adopted
state inside first unpin, without a new D5 action or success claim.

The malformed projection handoff now names the missing finite classes at the
correct dataplane-private pure boundary: wrong EtherType, IPv4 version/IHL,
non-TCP protocol, and destination-port bytes outside the returned buffer. Each
must produce `None` only for the affected semantic field. The separate
control-plane validator table owns semantic mismatch/source behavior, avoiding
raw/projection and owner-decision collapse.

The environment matrix remains layered and executable:

- source-local projection, validator, and D5 order/cleanup bodies are
  deterministic Rust RED evidence and never kernel proof;
- Lima root supplies Linux BPF/TCX, nft, netlink, persistent TAP, UDP, bpffs,
  the integration feature, tracing-captured ordinary production boot, and the
  verified `host-kernel-shared` assignment; and
- S08/09, S10, D12 inventory, D12A provision/teardown, D13 Sim ordering and
  native-metal evidence remain independent. D14A neither weakens nor replaces
  the inherited pinned-kernel/verifier and native-metal gates.

No monitor, timing poll, public Sim outcome, ignored panic placeholder,
test-only BPF runner, fabricated successful outcome, or direct private-runner
call may satisfy the Lima gate.

### Mechanical checks

| Check | Iteration-9 result |
| --- | --- |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; existing length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no validator errors. |
| JSON/diff hygiene | PASS — `jq empty` and `git diff --check`. |
| Locked dependency graph | PASS — `cargo metadata --format-version 1 --locked --no-deps`; D14A adds no dependency. |
| Exact private validator shape | PASS — private requirement/observation/mismatch/validation values and one exact production-used function; no cross-crate surface. |
| Error/source taxonomy | PASS — semantic mismatches remain source-less owner postconditions; lower typed failure stays in the existing I/O source chain. |
| Five-event trace | PASS — exact names, fields, emission fences, identity continuity, counter/guard conditions, and cleanup complement are pinned. |
| Deterministic Lima feasibility | PASS — synchronous tracing capture replaces transient polling. |
| Cross-process serialization | PASS — existing whole integration-binary override plus mandatory `show-config` proof. |
| D5 order | PASS — old order explicitly superseded; only exercise-before-close remains normative. |
| Malformed/environment handoff | PASS — finite malformed rows and layered source-local/Lima/independent evidence are explicit. |
| Roadmap execution status | Correctly pending until transitioned DISTILL and independent roadmap review. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

### Iteration-9 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-08 | Remain CLOSED. |
| DESIGN-P02-09 — production semantic mismatch decision not drivable | CLOSED by the production-used private validator and exact mismatch table. |
| DESIGN-P02-10 — malformed complete-output rows absent | CLOSED by the exact dataplane projection handoff; executable transition remains DISTILL work. |
| DESIGN-P02-11 — transient/racy Lima observation | CLOSED by synchronous production events and verified existing nextest serialization. |
| D14 public/doc-hidden API, D12/D12A/D13 | Remain APPROVED and unchanged. |

No unresolved DESIGN finding remains in the bounded D14A remediation.

## Iteration 9 final verdict

# APPROVED

D-295-DISTILL-14A is exact, implementable, source-honest, and bounded to the
proven translation gaps. It adds only a production-used private validator and
non-persisted completion telemetry, preserves D14's API and one-owner
architecture, makes the Lima proof deterministic, and closes
DESIGN-P02-09/10/11. DISTILL must now transition the four named S00 bodies and
pass independent review; roadmap `validation.status` correctly remains pending
and DELIVER step `02-01` is not yet executable.

## Iteration 10 — final D14A DESIGN-to-DISTILL translation review

### Scope and accepted translation portions

Iteration 10 reviews only the final four D14A S00 bodies and their matching
DISTILL/roadmap claims against the iteration-9 contract. D14A's architecture is
not reopened.

The translation preserves the approved boundaries:

- all D14A requirement/observation/mismatch/validation types and
  `validate_guest_tcx_tcp_probe` are module-private and exact;
- `GuestNetworkTcpProbeObservation::from_outcome` copies only the six approved
  semantic accessors, while the production validator remains an explicit RED
  panic;
- no public constructor, outcome field, raw packet/SKB/FD/syscall value, trait,
  free runner, error variant, owner, persistence record, callback, or timing
  hook was added;
- the dataplane projection table now includes complete-length wrong EtherType,
  IPv4 version/IHL, non-TCP protocol, and unavailable destination-port rows,
  with MACs retained and original destination absent;
- both the original D5 table and the focused D14A table now require only the
  exercise-before-close order, including pre-close cleanup, lazy adopted-state
  preparation, and all fifteen inventory observations; and
- the Lima body removes the monitor/polls and asserts the exact five-event
  trace: two ordered TCP stages, same-id attachment reopen, detached D9 guard,
  and fifteen-field all-zero cleanup. Counter continuity, guard packet/byte
  deltas, zero delivery, identity continuity, and cleanup predicates are all
  asserted. The existing whole integration-binary `host-kernel-shared`
  assignment remains intact and the roadmap requires `show-config` proof.

One blocking acceptance-table gap remains.

### Finding DESIGN-P02-12 — Blocking: the validator table does not enforce the full closed value partitions or exact first-mismatch order

**Evidence:**

Iteration 9 pins `Accept` as the sole valid verdict, `Intercept` as the sole
valid mark, exact `SocketAddrV4` equality, exact counter identity at each array
index, every wrong counter delta, and the first mismatch in the order verdict →
mark → source MAC → destination MAC → original destination → ordered counters.

The authored body at `guest_network.rs:3737-3873` leaves reachable incorrect
implementations green:

- verdict validation exercises `Drop` but not the other closed invalid value,
  `Unexpected`;
- mark validation exercises `Unexpected` but not `None` or `Accepted`;
- the wrong `OriginalDestination` row changes IP and port together, so a
  validator comparing only one axis passes; no row isolates same-IP/wrong-port
  and wrong-IP/same-port;
- wrong Intercept delta exercises `2` but not `0`, so a validator rejecting
  only oversized deltas passes;
- counter-identity rows replace one identity and therefore create a duplicate/
  missing pair; they do not include an order-preserving-cardinality permutation
  that distinguishes exact index validation from set/uniqueness validation;
  and
- the sole multi-error row proves only that verdict precedes mark/source. It
  does not prove mark-before-source, source-before-destination,
  destination-before-original-destination, semantic fields before counters, or
  lower counter index before higher counter index.

**Reachability and consequence:** The validator is the production decision
boundary added specifically to make every D14 semantic mismatch testable. An
implementation that accepts `GuestTcxProbeVerdict::Unexpected`, accepts
`GuestTcxProbeMark::None`, checks only the destination IP, accepts an unchanged
Intercept counter, treats counter identities as a set, or returns a later
mismatch first can satisfy every current D14A body. The DISTILL documents
therefore overstate “every exact private mismatch,” “every wrong delta,” and
“deterministic first mismatch.”

**Required bounded disposition:** Extend the existing private pure validator
table with the missing closed enum alternatives, isolated destination IP/port
rows, zero Intercept delta, a counter-identity permutation, and multi-error rows
that mechanically establish each adjacent first-mismatch precedence including
counter index order. This is test-only table completion under the already-
approved private validator; it requires no DESIGN, API, production seam,
telemetry, or environment change.

### Evidence and mechanical disposition

| Check | Iteration-10 result |
| --- | --- |
| Exact private D14A shape | PASS — no visibility or signature drift. |
| Raw/API/production-behavior fence | PASS — only exact semantic projection glue is implemented; decision/effect bodies remain RED. |
| Malformed projection | PASS — all iteration-9 complete-output classes plus short boundaries are present. |
| Superseded D5 order | PASS — both order tables use exercise-before-close exclusively. |
| Five-event Lima oracle | PASS — exact event order/fields, identity/counter/guard/cleanup assertions, no monitor or poll. |
| Kernel-test serialization | PASS — committed whole-binary nextest assignment retained; roadmap includes `show-config`. |
| Validator mismatch completeness | FAIL — DESIGN-P02-12. |
| Roadmap schema | PASS — `VALID: 4 phases, 11 steps`; existing length/count warnings remain non-blocking. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no validator errors. |
| JSON/diff hygiene | PASS — `jq empty` and `git diff --check`. |
| Roadmap execution status | Correctly pending; no DELIVER authority. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

The environment split remains honest: source-local tables are not kernel proof;
Lima is still required for the tracing-captured real adapter; S08/09/S10 and
D12/D12A/D13 retain their independent roles. The Lima body is compile/list
verified but remains `PENDING_ENVIRONMENT`, with no claimed real-kernel run.

### Iteration-10 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-11 | Remain CLOSED at DESIGN; D14A architecture remains APPROVED. |
| DESIGN-P02-12 — incomplete validator value/order table | OPEN, blocking in DISTILL translation. |

## Iteration 10 final verdict

# CHANGES_REQUIRED

The final translation is exact at the API, malformed-projection, owner-order,
telemetry, Lima, and serialization boundaries, but its core validator table
does not yet enforce the full closed semantic contract or deterministic
first-mismatch order. Complete the bounded rows above and re-review before
roadmap approval or DELIVER step `02-01`.

## Iteration 11 — final D14A validator-table re-review

### Scope and prior-finding disposition

Iteration 11 reviews only the bounded remediation for DESIGN-P02-12. D14A's
iteration-9 architecture, the previously accepted malformed projection, D5
order, five-event Lima oracle, serialization, and all D12/D12A/D13/D14
decisions remain unchanged.

DESIGN-P02-12 is **CLOSED**. The production-validator body now covers every
missing closed partition and the complete adjacent precedence chain:

- both invalid verdicts, `Drop` and `Unexpected`;
- all three invalid marks, `None`, `Accepted`, and `Unexpected`;
- absent and wrong source/destination MAC values;
- absent original destination, wrong IP with the expected port, expected IP
  with the wrong port, and both axes wrong;
- a full counter-identity permutation that preserves the eight-value
  cardinality, plus wrong identity at every individual index;
- wrong delta at every counter, including both zero and oversized Intercept
  deltas;
- ordinary decrease and max-to-zero wrap at every counter; and
- multi-error rows proving verdict before mark, mark before source, source
  before destination, destination before original destination, original
  destination before counters, lower counter record before the next record,
  counter identity before decrease, and decrease before delta.

The `Classifier` original-destination exemption remains separately asserted,
while `OriginalDestination` still requires exact IP and port. The healthy row
still returns the actual Intercept bracket, and the lower `GuestTcxError::Io`
remains downcastable beneath `std::io::Error::other`. These additions complete
the accepted private decision contract without adding a type, variant,
function, parameter, constructor, hook, or production implementation.

### Regression and evidence audit

- The D14A validator/type shapes remain byte-for-byte aligned with iteration 9
  and module-private.
- `GuestNetworkTcpProbeObservation::from_outcome` remains the sole semantic
  accessor projection; the validator is still an explicit RED scaffold.
- The dataplane malformed table, exercise-before-close D5 tables, lazy cleanup,
  five-event tracing receipt, and monitor-free Lima body are unchanged.
- The existing whole `overdrive-control-plane` integration-binary
  `host-kernel-shared` assignment and roadmap `show-config` command remain
  intact.
- Revised DISTILL and RED-classification prose now describe the actual closed
  partitions and adjacent precedence evidence without claiming a kernel run;
  the Lima body remains honestly `PENDING_ENVIRONMENT`.
- No public/raw ABI, system of record, owner, recovery behavior, supervisor
  behavior, product event consumer, or test coordination seam was introduced.

### Mechanical checks

| Check | Iteration-11 result |
| --- | --- |
| P02-12 invalid-value partitions | PASS — complete verdict, mark, optional value, destination-axis, identity, delta, decrease and wrap rows. |
| P02-12 first-mismatch precedence | PASS — every adjacent semantic/counter boundary and counter index order is exercised. |
| Exact private API/ownership | PASS — no D14A/D14 surface drift. |
| Malformed projection / D5 order / five-event Lima oracle | PASS — unchanged from iteration 10. |
| Roadmap-only DES integrity | PASS — roadmap format OK, no validator errors. |
| JSON/diff hygiene | PASS — `jq empty` and `git diff --check`. |
| Roadmap execution status | Correctly pending until independent roadmap approval; no DELIVER authority. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

### Iteration-11 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-11 | Remain CLOSED. |
| DESIGN-P02-12 — incomplete validator value/order table | CLOSED by the complete closed-partition and adjacent-precedence rows. |
| D14A architecture and prior D12/D12A/D13/D14 approvals | Remain APPROVED and unchanged. |

No unresolved finding remains in the bounded final D14A translation.

## Iteration 11 final verdict

# APPROVED

The final D14A DESIGN-to-DISTILL translation now enforces the complete private
semantic validator contract and deterministic first-mismatch order while
preserving every approved API, owner, evidence boundary, and real-kernel gate.
Independent roadmap approval remains required before DELIVER step `02-01`.

## Iteration 12 — D-295-DISTILL-15 DESIGN and revised-roadmap review

### Scope

This iteration reviews only proposed D-295-DISTILL-15 and its revised
`02-02`/`02-03` roadmap allocation. The previously approved D12/D12A/D13/D14/
D14A decisions and completed findings DESIGN-P02-01 through DESIGN-P02-12 are
not reopened.

The proposed architecture closes the implementation review's D1-D5 shape
problems in a bounded way:

- the cross-crate boundary is exactly one doc-hidden
  `SharedIpInterceptIdentity`, its three semantic constructor/projection
  methods, and two IP-intercept-specific observe/conditional-replace
  functions; `Client`, a public `nft::ip` bundle, family parameters, raw
  builders, observed-handle values, and `AtomicRuleMutation::Append/Replace`
  are all excluded;
- semantic equality contains canonical table/base-chain definitions, exact
  aligned set schemas, ordered normalized rules, exact userdata, and the two
  listener targets, while kernel handles remain private mutation receipts;
- `None -> Some`, `Some -> Some`, and `Some -> None` each use one complete nft
  batch, so rejected first-boot creation cannot leave the partial
  table/chain/set state reproduced by the step reviewer;
- the literal conditional-replacement arguments make valid wrong-read-back
  rollback exact for both `prior = Some(identity)` and `prior = None`, without
  fabricating an absence identity;
- the private guard's conditional Drop is sufficient for unpublished startup,
  and the `02-03` owner alone retains then privately forgets the erased guard
  after listener/task/element drain. No public relinquish method, downcast,
  second guard type, runtime mode, or ownership transfer is introduced; and
- the intended evidence split is honest in principle: deterministic failure
  partitions belong in the source-local stateful algorithm universe, actual
  nft schemas/transactions/read-back/complements belong in Lima, and
  BootClosed/publication/runtime retry/fail-stop/published relinquishment
  belong only to `02-03`. In particular, the private runtime helper receives no
  S-ND295-19 credit.

Two blocking gaps prevent approval.

### Finding DESIGN-P02-13 — Blocking: a failed mandatory replacement read-back has no source-honest transition or return shape

**Evidence:**

- The accepted fresh-process contract says that after a successful replacement
  commit, **any failed or mismatched** full read-back triggers one atomic
  rollback and a second read-back (`feature-delta.md:2641-2648`).
- D15 defines rollback only "after a valid but wrong replacement read-back" as
  `replace_atomically(replacement_observed.as_ref(), prior.as_ref())`
  (`feature-delta.md:2423-2431`). It does not define the expected-current
  argument, retained primary source, or returned disposition when that first
  post-commit `observe()` returns `Err(NetlinkError)`.
- None of the accepted public errors can represent that path honestly.
  `NftSharedReplaceFailed` asserts that the batch was rejected and the prior
  state is intact (`feature-delta.md:3585-3588`);
  `NftSharedRollbackFailed` is explicitly limited to a lower failure from the
  rollback write or rollback read (`feature-delta.md:3599-3603`); and the two
  remaining rollback dispositions are source-less semantic outcomes.
- The production algorithm demonstrates the concrete ambiguity today:
  `replace_observed_shared_program` commits at
  `mtls_intercept_port.rs:336-342`, then maps a failed replacement read-back at
  `:344-352` to `NftSharedRollbackFailed { operation: ReadBackPrior }` and
  returns without executing the rollback at `:357-378`. Thus the operation tag
  claims a rollback read which never occurred, and the accepted rollback-on-
  failed-read guarantee is not implementable from the pinned contract.
- D15 says the stateful I/O can fail at each observe/replace stage
  (`feature-delta.md:2417-2421`), but its exact evidence list covers batch
  rejection, valid post-commit mismatch, rollback write/read failures, and
  semantic rollback mismatch only (`:2452-2456`, `:2470-2475`). No body forces
  this missing transition for either optional-prior branch.

**Consequence:** A crafter must either mislabel the initial post-commit read
failure as a rollback-read failure, skip the mandatory rollback, discard the
real primary source, or invent a new public error/operation shape. Each choice
violates an accepted statement, and the repository's exact-API rule forbids the
crafter from choosing among them.

**Required bounded disposition:** Reopen only this transition in DESIGN and pin
the exact algorithm and accepted error shape for a failed first post-commit
read-back: the conditional rollback expectation for both `prior = Some` and
`prior = None`, retention of the real read-back source, rollback write/read/
semantic outcomes, and the final returned disposition. If that requires a new
or changed public error variant/field, obtain explicit approval rather than
delegating the shape to DELIVER. DISTILL must then add stateful rows for both
optional-prior branches. Do not change the approved doc-hidden netlink API or
move the owner/runtime behavior into `02-02` to close this finding.

### Finding DESIGN-P02-14 — Blocking: the revised roadmap credits bodies that DISTILL has not authored or allocated

**Evidence:**

- D15 names three exact Lima bodies and one exact `02-03` runtime body
  (`feature-delta.md:2470-2484`), and the roadmap credits those names at
  `roadmap.json:244-245` and `:292-300`. None of
  `shared_program_absence_create_readback_idempotence_and_guard_drop`,
  `shared_program_replaces_only_listener_targets_and_preserves_foreign_complement`,
  `shared_program_refuses_ambiguous_owned_state_without_mutation`, or
  `published_wrong_shared_target_is_observe_only_until_bounded_fail_stop`
  exists in the test tree.
- The two credited source-local functions do exist, but their current
  `ScriptedIo` is a pair of queued results plus call kinds
  (`mtls_intercept_port.rs:545-604`), the exact non-stateful evidence D15
  rejects. They are active tests, not authored reasoned-pending RED bodies, and
  their current assertions do not own the declared program/complement state.
- The DISTILL scenario matrix remains on the superseded allocation: it names
  only the old source-local body plus a generic Tier-3 read-back and says all
  S-ND295-14..19 remain in `02-02`
  (`distill/test-scenarios.md:49`, `:82-85`). The revised roadmap instead gives
  `02-02` S-ND295-14..18 as incomplete adapter layers and repeats S-ND295-14..18
  in `02-03`, with S-ND295-19 exclusively activated there
  (`roadmap.json:246-251`, `:269-300`).
- `distill/red-classification.md` contains no D15 body, command, observed RED,
  or environment-pending Lima classification. A DELIVER crafter would have to
  author or materially repair the tests to satisfy the roadmap, contrary to
  the roadmap's own no-re-authoring rule and the repository's acceptance-
  designer ownership rule.
- The verification selectors do not detect the omissions. `test(mtls_intercept_install)`
  runs the existing legacy module even when all three named D15 bodies are
  absent (`roadmap.json:260-262`), and `test(netns_density_shared_owner)` can
  pass after other owner bodies activate without the named S-ND295-19 body
  (`:317-322`). A green command therefore would not prove the roadmap's exact
  paired evidence.

**Consequence:** The proposed paired allocation is conceptually sound but is
not an executable DISTILL handoff. Current source, DISTILL SSOT, roadmap IDs,
and runner filters disagree about which step owns and closes S-ND295-14..19.
Advancing would either lose the Lima/runtime obligations or force a crafter to
invent acceptance evidence during DELIVER.

**Required bounded disposition:** Send D15 through DISTILL before roadmap
approval. The acceptance designer must transition the two source-local bodies
to the specified stateful universe, author the three exact Lima bodies and the
exact published-owner S-ND295-19 body, give every new/transitioned test its
required Contract Shape declaration and reasoned-pending ownership, record
their RED/environment classification, and update the scenario matrix so
S-ND295-14..18 close only after both steps while S-ND295-19 closes only in
`02-03`. Then make the `02-02` and `02-03` verification filters name the exact
credited bodies so their absence yields zero selected evidence rather than a
green legacy-module run. This remediation must preserve D15's source-local /
Lima / owner-layer separation.

### Mechanical and boundary checks

| Check | Iteration-12 result |
| --- | --- |
| Exact doc-hidden netlink API | PASS — necessary, sufficient for the named worker translation, and no generic/raw surface is authorized. |
| Semantic identity versus private handles | PASS — identity equality excludes handles; the adapter retains live handles only for the conditional batch. |
| Exact set ABI and complete-object atomicity | PASS at DESIGN — IPv4 and aligned IPv4/service schemas plus one mixed-object batch are explicit. |
| Optional-prior valid-mismatch rollback | PASS — `replacement_observed -> prior`, including deletion to genuine absence. |
| Failed replacement read-back | **FAIL — DESIGN-P02-13.** |
| Unpublished Drop / published relinquish ownership | PASS — `02-02` conditional cleanup and `02-03` private forget are disjoint; no public guard API is added. |
| Runtime wrong-target allocation | PASS at DESIGN — observe-only conflict/retry/fail-stop remains exclusive to `02-03`; the private helper is uncredited. |
| Stateful source-local / Lima / owner evidence split | PASS conceptually, **not executable — DESIGN-P02-14.** |
| DISTILL scenario/test/RED alignment | **FAIL — DESIGN-P02-14.** |
| Runner selectors | **FAIL — broad legacy-module filters do not require the named D15 bodies.** |
| Roadmap schema / JSON hygiene | PASS — `jq empty`; `validation.status` remains `pending`. |
| Diff hygiene | PASS — `git diff --check`. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

### Iteration-12 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-12 | Remain CLOSED. |
| DESIGN-P02-13 — failed replacement read-back has no honest rollback/error disposition | OPEN, blocking DESIGN/API completion. |
| DESIGN-P02-14 — D15 tests, DISTILL ownership, RED classification, and exact runner selection are absent or stale | OPEN, blocking DISTILL/roadmap execution. |
| D15 doc-hidden adapter shape, handle privacy, atomic valid transitions, and 02-02/02-03 owner split | ACCEPTED portions; do not reopen while resolving P02-13/14. |

## Iteration 12 final verdict

# CHANGES_REQUIRED

D-295-DISTILL-15 correctly removes the rejected public `nft::ip`/
`AtomicRuleMutation` expansion, supplies a minimal implementable semantic
cross-crate boundary, keeps handles private, makes valid optional-prior
rollback atomic, and defers published-owner runtime/relinquish behavior to
`02-03` without weakening `02-02`. It is not yet complete: the accepted
rollback contract has no source-honest outcome for a failed first post-commit
read-back, and the paired evidence exists only in DESIGN/roadmap prose rather
than aligned DISTILL bodies and exact runner selectors. Resolve
DESIGN-P02-13/14 and re-review before approving D15 or resuming step `02-02`.

## Iteration 13 — D-295-DISTILL-15 remediation re-review

### Scope

This iteration re-reviews only DESIGN-P02-13 and DESIGN-P02-14 against the
amended D15 rollback algebra, exact pending-authoring evidence handoff, revised
DISTILL prose, and revised `02-02`/`02-03` roadmap. The iteration-12 accepted
doc-hidden netlink boundary, handle-private semantic identity, mixed-object nft
atomicity, unpublished/published guard ownership split, and runtime deferral are
not reopened.

### DESIGN-P02-13 disposition — CLOSED

The amended state machine now covers the formerly unrepresentable committed-
replacement / failed-desired-read path without overloading an existing
disposition:

- `prior` is captured once; `replace_atomically(prior, Some(requested)) ->
  Ok(())` is the single desired commit boundary. No later branch can return
  `NftSharedReplaceFailed` or submit the desired replacement a second time.
- A successful wrong semantic read records `replacement_observed` and no
  desired-read source, then conditionally rolls back from that exact observed
  identity.
- A failed desired read records the real `replacement_read_source`, records no
  observation, and conditionally rolls back from `requested`. This expectation
  is honest because the desired batch already acknowledged commit; the
  netlink adapter still performs a fresh complete observation and refuses the
  conditional batch without mutation if `requested` is no longer current.
- Both trigger classes perform one rollback attempt. A successful rollback is
  followed by one read-back; a failed rollback write performs no fabricated
  read. `prior = None` remains the literal deletion-to-absence target.
- Exact restoration after a semantic trigger returns the existing source-less
  `NftSharedReplacementMismatchRolledBack`. Exact restoration after a lower
  desired-read failure returns the new
  `NftSharedReplacementReadFailedRolledBack` and retains that real source.
- Rollback write/read failure returns `NftSharedRollbackFailed`; its annotated
  `source` is always the failing rollback operation, while optional
  `replacement_read_source` separately retains an earlier lower trigger.
- A successful rollback read of the wrong identity returns
  `NftSharedRollbackPostconditionMismatch`. It remains source-less for a
  semantic trigger and exposes the earlier desired-read error as its sole
  source only for the lower-error trigger.

The trigger partitions do not overlap. `replacement_read_source = Some(_)`
requires `replacement_observed = None`; with no desired-read source,
`replacement_observed` carries the successful observation and may itself be
`None` to represent genuine observed absence. The optional prior is independent
of that trigger distinction and is preserved in every rollback disposition.
The state table distinguishes rejected desired batch, committed desired state,
failed rollback write, restored-but-unverified rollback read failure, verified
exact restoration, and successful wrong rollback observation without
inventing a fifth mutation.

### Public API and Rust error-source feasibility

The API impact is minimal and exact:

- one public `InterceptError` variant,
  `NftSharedReplacementReadFailedRolledBack`;
- one `replacement_read_source: Option<NetlinkError>` field added to
  `NftSharedRollbackFailed`; and
- the same optional field, annotated `#[source]`, added to
  `NftSharedRollbackPostconditionMismatch`.

No method, trait, parameter, guard operation, netlink type, or ownership API is
added. Reusing `NftSharedReplacementMismatchRolledBack` would conflate a
source-less semantic mismatch with a lower read failure; using
`NftSharedRollbackFailed` after successful restoration would falsely report a
rollback failure. The new variant is therefore the smallest honest successful-
restoration disposition, and the two optional fields are the smallest way to
retain the earlier source when a later rollback outcome also must be reported.

The shape is implementable with the repository's actual Rust types.
`NetlinkError` is move-only, but each terminal branch consumes the saved
desired-read error exactly once, so no `Clone` requirement is introduced.
`thiserror` 2.x supports `#[source] Option<T>` by returning `None` for the empty
case and the contained `T` otherwise. Only one field is the Rust error-chain
source in each variant: the rollback operation for
`NftSharedRollbackFailed`, the optional desired-read error for
`NftSharedRollbackPostconditionMismatch`, and the required desired-read error
for `NftSharedReplacementReadFailedRolledBack`. When two real errors exist,
the later rollback failure remains the chain source and the earlier desired-
read error remains structured evidence; neither is overwritten or falsely
chained as the other operation.

### DESIGN-P02-14 disposition — CLOSED at the DESIGN-to-DISTILL handoff

The amended artifacts now name exactly seven new or transitioned bodies:

1. `shared_program_post_commit_failure_rolls_back_source_honestly_for_every_prior`
2. `shared_program_replace_refusal_idempotence_and_guard_cleanup_preserve_complete_state_delta`
3. `shared_program_prior_snapshot_mismatch_preserves_complete_state_and_complement`
4. `shared_program_absence_create_readback_idempotence_and_guard_drop`
5. `shared_program_replaces_only_listener_targets_and_preserves_foreign_complement`
6. `shared_program_refuses_ambiguous_owned_state_without_mutation`
7. `published_wrong_shared_target_is_observe_only_until_bounded_fail_stop`

Their homes, exact fully-qualified nextest selectors, exact Contract Shape
line, reasoned markers, and present `PENDING_AUTHORING` status agree across
`feature-delta.md`, `distill/test-scenarios.md`,
`distill/red-classification.md`, and `deliver/roadmap.json`. The artifacts no
longer claim absent bodies as RED, executable, or environment-blocked. The
roadmap remains `pending`, explicitly forbids DELIVER from authoring or
repairing the bodies, and requires acceptance authoring plus independent
DISTILL/roadmap review before `02-02` resumes.

The evidence universes are complete and non-overlapping:

- source-local owns the semantic program, exact contents of all three dynamic
  sets, ordered foreign bytes, conditional mutation journal, and remaining
  fault schedule. Its finite table covers both optional priors, both desired-
  read triggers, rollback write/read failures, exact restoration, and semantic
  rollback mismatch, with exact program/complement deltas and both real
  sources where applicable;
- Lima owns actual IP-family table/chains/set schemas/elements/eight rules,
  generation-consistent semantic observation, target-only replacement,
  idempotence/no mutation, malformed/foreign refusal, unrelated-table
  preservation, and successful unpublished guard Drop. It drives public
  `HostMtlsIntercept` and does not manufacture rollback faults; and
- `02-03` owns BootClosed/zero-managed-TAP gating, listener/task publication
  and cleanup, retained published guard, observe-only wrong-target retry,
  bounded fail-stop, and private published relinquishment. The new S-ND295-19
  body is exclusive to `02-03`; the private helper remains uncredited.

S-ND295-14..18 therefore close only after the source-local and Lima layers from
`02-02` and the existing owner-refusal/publication layer from `02-03` pass.
S-ND295-19 closes only through the new published-owner body in `02-03`. This is
paired evidence, not weakened single-layer substitution.

### Environment routing and evidence honesty

D15's real nft/netlink bodies correctly remain on Lima root; they require no
KVM or microVM. Source-local state-machine and `02-03` owner bodies may use
Lima only as the explicit Linux runner and receive no kernel credit from that
fact. Native VMM/microVM bodies elsewhere in the roadmap remain routed through
`cargo xtask metal run --`; compile/clippy commands may still use Lima because
they do not claim KVM execution.

The current workspace has `OVERDRIVE_METAL_TARGET` configured in `.env`, and
`cargo xtask metal --help` succeeds while documenting that the runner loads the
target from either the process environment or workspace `.env`. The user's
confirmation that `cargo xtask metal run --` works is authoritative current
environment evidence. Consequently, an inherited interactive shell lacking an
exported variable is never grounds for classifying metal unavailable.

Two older sentences remain stale but do not affect D15's API/evidence design:
`feature-delta.md:7232-7235` and `distill/red-classification.md:3-10` describe
the earlier run as though `OVERDRIVE_METAL_TARGET` were currently unset. They
must be read as historical execution context or corrected before the next
global DISTILL/roadmap evidence review; they are not a current environment
blocker and do not authorize rerouting a KVM/microVM body to Lima.

### Mechanical checks

| Check | Iteration-13 result |
| --- | --- |
| P02-13 complete transition table | PASS — two triggers × rollback write/read/exact/mismatch outcomes, for both optional priors. |
| Trigger/prior/state/source distinction | PASS — exact non-overlap and no fabricated, substituted, or lost source. |
| Rust feasibility | PASS — move-only `NetlinkError` is consumed once; `thiserror` optional-source derivation is supported. |
| Public API minimality | PASS — one necessary variant plus two necessary optional evidence fields; no port/owner/netlink expansion. |
| Seven-body handoff | PASS — exact names, homes, markers, selectors, universes, and `PENDING_AUTHORING` truthfulness. |
| Paired evidence allocation | PASS — source-local algorithm, Lima real adapter, and `02-03` owner/runtime remain distinct and jointly complete. |
| Runtime/published relinquish deferral | PASS — exclusive to `02-03`; no `02-02` credit or API escape hatch. |
| Metal/Lima routing | PASS — nft-only D15 stays Lima; native VMM/microVM execution stays metal. |
| Metal configuration | AVAILABLE — workspace `.env` is configured; runner help succeeds; user confirms remote runner works. |
| Roadmap JSON and execution status | PASS — valid JSON and correctly `pending` until acceptance authoring/review. |
| Diff hygiene | PASS — `git diff --check`. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

### Iteration-13 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-12 | Remain CLOSED. |
| DESIGN-P02-13 — failed replacement read-back lacked an honest rollback/error disposition | **CLOSED** by the exact trigger/state table and minimal source-retaining error amendment. |
| DESIGN-P02-14 — D15 test ownership/selectors/evidence were absent or stale | **CLOSED at DESIGN handoff** by the exact seven-body `PENDING_AUTHORING` contract and aligned artifacts. Actual body authoring and DISTILL review remain mandatory downstream gates. |
| D15 doc-hidden netlink boundary, handle privacy, atomic valid transitions, guard ownership, and 02-02/02-03 split | Remain accepted and unchanged. |

No unresolved D15 DESIGN finding remains.

## Iteration 13 final verdict

# APPROVED

D-295-DISTILL-15 is now exact, minimal, source-honest, implementable, and
properly layered. It represents the committed-replacement / failed-desired-read
path without corrupting the established rollback variants, retains two real
causes without conflation when rollback also fails, preserves genuine absence,
and adds no ownership or mutation surface beyond the necessary error-algebra
amendment. The seven-body handoff is precise and truthfully pending; D15's Lima
adapter evidence and `02-03` runtime/published-owner evidence remain separate.
Approval of D15 does not approve the roadmap for execution: the acceptance
designer must author/transition and independently review all seven bodies, then
the roadmap must be independently reapproved before DELIVER step `02-02` may
resume.

## Iteration 14 — D15 DESIGN-to-DISTILL translation review

### Scope

This iteration reviews only the authored D15 acceptance translation: the seven
exact body names, test boundaries, declared state universes, error scaffolds,
source-local/Lima pairing, deferred `02-03` runtime evidence, and executable
selectors. D15's iteration-13 architecture is not reopened. No production
implementation, test, DESIGN, DES, or commit change is part of this review.

The translation preserves several load-bearing decisions:

- all seven required bodies exist at the exact approved homes, carry the exact
  `/// CONTRACT_SHAPE: bounded-change.` line and step-specific reasoned marker,
  and exact-select one body each;
- the three source-local bodies use the module-private production algorithm and
  a stateful seam whose declared universe includes owned program, three dynamic
  element inventories, ordered foreign bytes, conditional mutation journal,
  and remaining fault schedule;
- the three Lima bodies enter through public `HostMtlsIntercept`, use real
  nft/netlink under the serialized worker integration binary, and do not inject
  deterministic rollback faults;
- the S-ND295-19 body remains in `02-03`; the private host-adapter helper still
  receives no scenario credit;
- the only production scaffold changes are D15's exact one error variant, two
  optional earlier-source fields, and neutral construction fallout. No method,
  parameter, guard operation, netlink family parameter, raw identity type, or
  ownership API was added; and
- RED classification is honest: two source-local bodies, three Lima bodies,
  and the owner body are RED at production gaps; the stale-prior body is
  recorded GREEN-on-arrival rather than fabricated as RED.

Three blocking translation gaps remain.

### Finding DESIGN-P02-15 — Blocking: the S-ND295-19 body neither supplies a valid wrong target nor reaches bounded fail-stop

**Evidence:**

- The accepted scenario is a published owner whose canonical rule names a
  different but valid listener target, followed by the production 250 ms / 5 s
  recovery budget and one fail-stop request (`feature-delta.md:2498`,
  `deliver/roadmap.json:275-284`, `distill/test-scenarios.md:529-538`).
- The authored body mutates the fixture identity by appending `0xff` to
  `prerouting[0]` (`netns_density_shared_owner.rs:354-361`). That is a malformed
  byte vector, not the same canonical identity carrying a different non-zero
  listener port. An implementation that distinguishes malformed identity from
  a valid wrong target is not exercised.
- The body manually calls `converge_shared_owner()` twenty times in an
  immediate loop (`:378-407`). It advances no injected clock, observes no
  250 ms cadence or five-second deadline, drives no retained control-plane
  supervisor, and asserts no `ServeShutdownRequest`, FailStop transition,
  component/cause, attempts, or elapsed duration.
- Its own comment states that the control-plane supervisor owns the clock and
  fail-stop request (`:378-381`), but that owner is absent from the test. After
  the manual calls, the test invokes ordinary `shutdown_owner()` (`:421-425`).
  Thus the exact body can become green even if persistent target conflict never
  produces fail-stop.

**Consequence:** The body proves only repeated worker-level observe-only
conflict and sealed guard relinquishment. It cannot close the named
`until_bounded_fail_stop` scenario or the roadmap's exclusive S-ND295-19
activation.

**Required bounded disposition:** Keep target replacement forbidden and add no
public hook. Seed an otherwise canonical observed identity whose leg-F or
leg-C target is a different valid non-zero port. Then drive the existing
production owner/supervisor composition that consumes this exact
`MtlsSharedOwnerError::Intercept`, using its accepted clock, and assert the
250 ms / five-second budget, exact attempt count, one typed fail-stop request,
no bind/converge/port substitution, retained published guard during retries,
and private relinquishment during terminal drain. If the accepted worker-file
home cannot reach that existing consumer because of the dependency direction,
reconcile the test-home/pairing in DESIGN/DISTILL; do not add a worker method or
test-only production seam.

### Finding DESIGN-P02-16 — Blocking: the Lima bodies do not force exact target semantics or isolate every ambiguity regression

**Evidence:**

- `assert_only_listener_targets_changed` proves only that prerouting entries 1
  and 3 differ from their old bytes (`mtls_intercept_install.rs:375-391`). It
  never proves that either changed register equals the requested
  `D15_LEG_F_NEW.port()` / `D15_LEG_C_NEW.port()`. The absence/create body also
  checks counts and emptiness but not the exact initially requested targets
  (`:394-436`). A consistently wrong target encoder can therefore satisfy both
  success bodies.
- The `ForeignFamily` row seeds only `table bridge overdrive-mtls` on an absent
  IP-family program (`:515-532`). It does not seed an exact valid IP program
  and then add the same-named foreign-family table. Consequently it cannot
  catch the concrete review defect in which the foreign-family check runs only
  when the IPv4 table is absent.
- The `ConflictingSetSchema` row creates table/chains/sets but no eight-rule
  program (`:533-541`). It therefore contains both a conflicting schema and an
  incomplete program. Because the assertion checks only `is_err()`, an adapter
  that wrongly accepts the set schema but later refuses missing rules remains
  green. This is not an isolated schema partition.
- The DISTILL executable audit nevertheless claims exact target boundaries,
  every malformed partition, and COMPLETE 15/15 coverage. Those claims exceed
  the executable assertions.

**Consequence:** The real-kernel layer proves useful effect reachability but
does not yet pin D15's two target registers as the sole exact delta or reproduce
two of the implementation review's specific ambiguity failures.

**Required bounded disposition:** Within the existing three Lima bodies,
assert that observed/new nft target registers equal both requested listener
ports, not merely that bytes changed, while every non-target normalized byte
remains equal. Add the valid-IP-plus-same-name-foreign-family coexistence row.
Make the conflicting-set-schema row invalid on the schema axis only (or provide
an exact typed semantic oracle that proves schema refusal before any independent
incompleteness can decide the case). Retain public-host-adapter entry, external
fixture-only mutation, real-kernel serialization, and the unrelated foreign-
table complement; add no production inspection API.

### Finding DESIGN-P02-17 — Blocking: source-local tests do not assert the complete declared error/universe contract

**Evidence:**

- The post-commit table destructures and checks both stored `NetlinkError`
  fields, but never calls `std::error::Error::source` on the returned
  `InterceptError` (`mtls_intercept_port.rs:928-1053`). Removing or moving the
  `#[source]` annotations would leave the body green even though D15 explicitly
  requires the rollback-operation error to be the chain source for
  `NftSharedRollbackFailed`, the desired-read error to be the source for
  `NftSharedReplacementReadFailedRolledBack`, and the optional desired-read
  error to be the sole source of the lower-trigger rollback semantic mismatch.
- The rejected desired-batch row asserts only
  `conditional_mutations.len() == 1` (`:1119-1153`), the target-retarget row
  asserts no journal value at all (`:1196-1219`), and exact reapply asserts only
  one cleanup mutation by count (`:1221-1240`). The declared source-local
  universe and DISTILL prose require the exact conditional mutation journal,
  including expected-current, desired target, and committed/rejected
  disposition, on every row.
- No authored body exercises D15's explicit `for_listener_ports` boundary that
  rejects leg-F zero and leg-C zero independently before I/O
  (`feature-delta.md:2395-2398`). Merely asserting successful production-bound
  ports are non-zero does not falsify an implementation that accepts zero.

**Consequence:** Field retention is well covered, but Rust source-chain
behavior, portions of the claimed closed state universe, and the semantic API's
minimum boundary can regress while all three source-local selectors remain
green.

**Required bounded disposition:** Extend the existing source-local bodies—do
not add a new public seam—to assert `Error::source()` for every semantic/lower
trigger disposition (including `None` for source-less semantic outcomes), exact
journal entries for refusal/retarget/reapply/cleanup, and independent zero
leg-F / zero leg-C rejection with no observe or mutation. Keep the seven-name
allocation unless the acceptance designer proves a separate body is required;
DELIVER must not author these missing assertions.

### Selector, boundary, and completeness checks

| Check | Iteration-14 result |
| --- | --- |
| Seven exact names/homes/markers | PASS. |
| Exact nextest selection | PASS — recorded runs discover one ignored body per selector. |
| Real-kernel serialization | PASS — all three Lima names resolve to `host-kernel-shared`. |
| D15 public error scaffolds | PASS — exact variant/fields/annotations only; neutral fallout is bounded. |
| Source-local stateful rollback matrix | PASS for two priors × two triggers × four rollback outcomes; FAIL for source-chain and complete-journal assertions (P02-17). |
| Lima public-adapter boundary | PASS; exact-target and isolated ambiguity oracles FAIL (P02-16). |
| `02-03` runtime deferral | PASS in ownership; executable bounded fail-stop proof FAIL (P02-15). |
| API/ownership drift | PASS — no unauthorized public surface or owner movement. |
| Roadmap JSON / formatting / diff hygiene | PASS — `jq empty`, `cargo fmt --all -- --check`, and `git diff --check`. |
| Mutation testing | NOT RUN — final DELIVER gate only. |

### Canonical D15 completeness audit

| Item | Result |
| --- | --- |
| C1a | PASS — genuine prior absence/create/rollback/Drop. |
| C1b | FAIL — zero-port boundary and exact requested target values are not asserted. |
| C2a | PASS — commit/read/rollback/guard/published states are documented. |
| C2b | FAIL — persistent wrong target never reaches the required terminal fail-stop transition. |
| C3 | PASS — absent/present, exact rule/set counts, zero/one dynamic element, duplicate rule. |
| C4a | PASS — exact reapply/no-write is exercised. |
| C4b | PASS — rollback-to-absence and stale conditional guard cleanup. |
| C5a | PASS — semantic/lower triggers, optional prior, rollback outcomes, and fresh/runtime modes are enumerated. |
| C5b | PASS — complements remain independent of the allowed owned-program delta. |
| C6a | FAIL — foreign-family coexistence and set-schema-only malformed partitions are absent. |
| C6b | FAIL — declared Rust error-chain source behavior is not asserted. |
| C6c | PASS — terminal variants and trigger fields are otherwise closed by exhaustive matching. |
| C7a | PASS — real nft failure plus scripted read/write failures and malformed inventory. |
| C7b | PASS — commit-before-read and restored-but-unverified interruption states. |
| C7c | PASS — stale prior and stale guard conditional no-mutation. |

Mechanical score: **11/15 — ACCEPTABLE_WITH_DOCUMENTED_GAPS** under the
canonical threshold, but the four failures map directly to mandatory D15
acceptance criteria and S-ND295-19 scenario closure. Coverage-completeness and
observable-boundary gaps therefore remain blocking for this handoff.

### Iteration-14 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-14 | Remain CLOSED at their approved scopes. |
| DESIGN-P02-15 — S-ND295-19 lacks valid-target and bounded-fail-stop proof | OPEN, blocking DISTILL translation. |
| DESIGN-P02-16 — Lima exact-target and ambiguity partitions are not falsifying | OPEN, blocking DISTILL translation. |
| DESIGN-P02-17 — source-chain, journal, and zero-port assertions are incomplete | OPEN, blocking DISTILL translation. |
| D15 architecture/API/owner allocation | Remains APPROVED; findings require test remediation only unless P02-15 confirms the approved test home cannot reach the existing production consumer. |

## Iteration 14 final verdict

# CHANGES_REQUIRED

The translation has the correct seven names, exact selectors, layered
source-local/Lima/owner boundaries, and sanctioned error scaffolds, but it does
not yet enforce the complete D15 contract. The runtime body substitutes a
malformed identity and manual calls for the accepted valid wrong-target /
bounded fail-stop path; the Lima layer omits exact requested-port equality and
two isolated ambiguity regressions; and the source-local layer does not assert
Rust source-chain behavior, its complete mutation journal, or zero-port
rejection. Remediate P02-15/16/17 within the approved architecture, rerun exact
RED classification and completeness audit, and re-review before roadmap
approval or DELIVER step `02-02`.

## Iteration 15 — P02-15 layered S-ND295-19 DESIGN review

### Scope

This iteration reviews only P02-15's proposed S-ND295-19 layering:

- `02-02` source-local plus Lima adapter no-rewrite evidence;
- `02-03` published-worker conflict/guard prerequisite;
- `03-03` control-plane-private runtime cadence/deadline/request closure through
  `SharedNetworkSupervisorHandle::run_mtls_owner`.

The previously approved D15 adapter/error algebra and P02-16/P02-17 DISTILL
findings are not reopened. The proposed S19-A and S19-B bodies remain pending
acceptance authoring, so this review validates their exact contract and homes,
not executable RED.

### Accepted architecture and evidence allocation

The relocation of retry/fail-stop ownership from the worker test to the
control-plane supervisor is correct:

- `MtlsInterceptWorker` exposes only the already-approved seven lifecycle
  methods. It has no clock, retry budget, request sender, EXEC write
  capability, runtime mode, or fail-stop method, so it cannot honestly close
  S-ND295-19 by itself.
- `overdrive-control-plane` already depends on `overdrive-worker` and may hold
  `Arc<MtlsInterceptWorker>`; there is no worker-to-control-plane edge. Placing
  the closing body in the control-plane source-local module preserves the
  acyclic dependency graph.
- The exact private production join point is implementable and sufficient:

  ```rust
  async fn run_mtls_owner(
      shared_guest_network: Arc<dyn SharedGuestNetworkOwner>,
      mtls_worker: Arc<MtlsInterceptWorker>,
      exec: Arc<GuestNetworkExecSupervisor>,
      clock: Arc<dyn Clock>,
      request_tx: tokio::sync::mpsc::Sender<ServeShutdownRequest>,
      shutdown: CancellationToken,
  ) -> Result<(), SharedNetworkSupervisorError>;
  ```

  Each argument is owned by the existing composition: shared-network quiesce,
  real worker audit/recovery, paired EXEC mutation, the same injected clock as
  `GuestNetworkExecWiring`, the existing capacity-one request channel, and the
  existing intentional-shutdown token. The function is module-private,
  production-used, and directly reachable only from its child acceptance
  module; no exported constructor, test-only callback, second supervisor, or
  public port is introduced.
- Ordinary `run_server` retains one supervisor task and polls this future
  alongside its already-owned signals. The method is a private future, not a
  detached task or second lifecycle owner.
- S19-B's exact home in
  `crates/overdrive-control-plane/src/lib.rs::shared_network_task_owner_acceptance`
  can drive the same private function as production with a real worker and one
  shared `SimClock`. The harness advances time; it does not implement the retry
  loop.
- The timing/oracle contract is precise: no detection before the one-second
  audit; `Open -> Recovering(IpRules)` at detection; twenty completed failed
  attempts separated by twenty 250 ms sleeps; one
  `SharedGuestNetwork { component: IpRules, cause:
  RecoveryDeadlineExceeded, attempts: 20, elapsed: 5s }` request; EXEC remains
  closed; the supervisor parks until intentional shutdown.
- The prerequisite layers remain independent and necessary. S19-A proves the
  canonical wrong target is observable without adapter mutation; `02-03`
  proves a published worker retains its guard, returns one structured
  observe-only conflict, and relinquishes through the existing terminal worker
  shutdown path. S19-B proves cadence/deadline/request closure and retains the
  guard throughout recovery. S19 closes only after all layers pass.

The roadmap dependency order is sufficient: `02-02 -> 02-03 -> 03-01 ->
03-02 -> 03-03`. Repeated S-ND295-19 IDs are explicitly labeled layer evidence,
and only `03-03` claims scenario closure. The exact future selector and
reasoned marker are pinned; their present zero-selection state is honestly
`PENDING_AUTHORING` rather than environment failure or RED.

Two blocking contract inconsistencies remain.

### Finding DESIGN-P02-18 — Blocking: S19-A asks `HostMtlsIntercept::observe_shared` to return a structured mismatch it cannot produce

**Evidence:**

- D15's accepted five-method port defines `observe_shared()` as a non-mutating
  complete identity observation. A canonical program with a different target
  is therefore `Ok(Some(observed_identity))`; comparison with the published
  expected identity belongs to the worker's `audit_shared_owner` /
  `converge_shared_owner` decision.
- The correction correctly requires the transitioned source-local S19-A body
  to use the public `HostMtlsIntercept::observe_shared` boundary and rejects the
  private helper as evidence. However, the normative D15 table still says the
  two `02-02` Host bodies "return the structured mismatch"
  (`feature-delta.md:2499`), and the `02-02` roadmap repeats that both S19-A
  bodies prove a "structured mismatch" (`roadmap.json:242`).
- No approved adapter method can produce `InterceptError::PostconditionMismatch`
  from observe-only state. `converge_shared` is fresh-process mutation authority
  and is forbidden at runtime; reusing the private helper would recreate the
  uncredited/no-production-caller defect; adding a method would be public API
  drift.

**Consequence:** The acceptance designer cannot implement the exact `02-02`
criterion without either fabricating the mismatch in the test, calling an
uncredited private helper, invoking the forbidden mutation path, or inventing
surface.

**Required bounded disposition:** Narrow S19-A to its actual adapter contract:
public `observe_shared` returns the canonical different non-zero target, with
zero bind/replace/notification and complete complement preservation. Assign
the first structured `PostconditionMismatch` exclusively to the existing
`02-03` published-worker prerequisite, and retain all cadence/deadline/request
behavior exclusively in S19-B/`03-03`. Update the D15 table, roadmap criterion,
and pending-body prose consistently; add no adapter method or runtime mode.

### Finding DESIGN-P02-19 — Blocking: authoritative D15 ownership/universe prose still assigns retry and fail-stop to `02-03`

**Evidence:**

- The corrected traceability table says `02-03` is publication/conflict/guard
  prerequisite only and `03-03` exclusively owns the runtime retry/deadline/
  typed request (`feature-delta.md:2499-2512`, roadmap delivery protocol and
  steps `02-02`/`02-03`/`03-03`).
- The immediately following "exact test homes" paragraph still assigns
  `tests/acceptance/netns_density_shared_owner.rs` responsibility for
  "runtime retry, and fail-stop ownership" (`feature-delta.md:2501-2508`).
- The declared `02-03` owner universe still includes "retry clock/attempts,
  and fail-stop request" (`feature-delta.md:2561-2564`), although the correction
  explicitly says the worker owns none of those and the 03-03 private
  supervisor owns them.
- These are normative statements inside the same D15 section, not historical
  review text. They conflict with the new roadmap and coverage map and could
  direct the `02-03` crafter to reintroduce the very worker-owned retry loop the
  correction rejects.

**Consequence:** Scenario IDs are mechanically aligned, but the SSOT still has
two competing owner assignments. That violates the repository's precise-owner
terminology rule and makes the handoff unsafe despite the sound dependency
choice.

**Required bounded disposition:** Rewrite only those stale D15 paragraphs:
the `02-03` test home/universe ends at published sockets/tasks/guard, one
structured observe-only conflict, and sealed relinquishment; the `03-03`
control-plane universe owns clock, detection, attempts, deadline, EXEC
FailStop, typed request, and terminal orchestration. Preserve the approved
method signatures and roadmap order.

### Terminal relinquishment and public-surface audit

Terminal ownership remains implementable without extending `run_mtls_owner`'s
signature. The private future retains the published guard during detection and
all retries, sends one fail-stop request, and parks on the existing shutdown
token. The existing `ServerHandle` terminal path already calls
`MtlsInterceptWorker::shutdown_owner`, whose `02-03` acceptance prerequisite
proves listener/task/capability drain and sealed constant-program guard
relinquishment. S19-B may join those accepted layers; it must not make
`run_mtls_owner` a second worker-shutdown owner or add a new shutdown error
variant merely to duplicate the existing terminal owner.

No public seam is required or permitted. `SharedNetworkSupervisorHandle`,
`run_mtls_owner`, its error, and its test home all remain control-plane-private;
`ServerHandle::shutdown_requested` is the unchanged public delegation. The
worker port, netlink API, EXEC capabilities, and shared-network owner port are
unchanged.

### Mechanical checks

| Check | Iteration-15 result |
| --- | --- |
| Exact private signature | PASS — necessary arguments only; no `pub`/`pub(crate)` method or test-only parameter. |
| Dependency direction | PASS — control-plane -> worker/core; no reverse edge. |
| Sole supervisor ownership | PASS — one retained task/handle; private future is polled, not spawned as a second owner. |
| One-second / 20 × 250 ms / five-second contract | PASS at DESIGN — exact clock, attempt, component, cause, and request fields are pinned. |
| Published guard prerequisite / terminal relinquish | PASS as layered evidence — worker retains during retries; existing terminal owner relinquishes. |
| S19 test homes and selectors | PASS structurally — S19-A `02-02`, worker prerequisite `02-03`, S19-B `03-03`; pending bodies are honestly unexecuted. |
| S19-A adapter observable | FAIL — P02-18 requires an impossible adapter-authored structured mismatch. |
| Ownership SSOT consistency | FAIL — P02-19 leaves retry/fail-stop in the `02-03` paragraphs. |
| Public API / ownership drift | PASS in the selected mechanism; remediation must not add surface. |
| Roadmap JSON / formatting / diff hygiene | PASS — `jq empty`, `cargo fmt --all -- --check`, and `git diff --check`. |
| Mutation testing | NOT RUN — final DELIVER gate only. |

### Iteration-15 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-01 through DESIGN-P02-14, P02-16 and P02-17 | Unchanged from their prior scopes. |
| DESIGN-P02-15 — worker body could not own bounded fail-stop | **PARTIALLY CLOSED** — correct private control-plane future/home/timing contract selected; P02-18/19 must close before approval. |
| DESIGN-P02-18 — impossible adapter-authored structured mismatch | OPEN, blocking. |
| DESIGN-P02-19 — stale `02-03` retry/fail-stop ownership prose | OPEN, blocking. |

## Iteration 15 final verdict

# CHANGES_REQUIRED

P02-15 selects the correct architecture: S19-A adapter no-rewrite in `02-02`,
published-worker conflict/guard prerequisite in `02-03`, and sole runtime
closure in `03-03` through the production-used private control-plane
`run_mtls_owner` future. Its signature, dependency direction, timing contract,
typed `IpRules` fail-stop, test home, terminal ownership join, and lack of
public surface are sound. The handoff is not yet exact because `02-02` still
demands a structured mismatch from an observe-only adapter, and two normative
D15 paragraphs still assign retry/fail-stop state to the worker step. Resolve
P02-18/19 without adding API or moving behavior, then re-review before S19
acceptance authoring or roadmap approval.

## Iteration 16 — P02-18/P02-19 correction re-review

### Scope

This iteration re-reviews only the P02-18 adapter-observation correction and
the P02-19 `02-03`/`03-03` ownership correction. P02-16/P02-17's acceptance-
body findings remain outside this bounded pass.

### P02-18 disposition — CLOSED

The S19-A contract now matches the accepted five-method adapter exactly:

- public `HostMtlsIntercept::observe_shared` returns
  `Ok(Some(canonical_wrong_target))` for a complete owned identity carrying a
  different valid non-zero target;
- partial, foreign, duplicate, malformed, or lower observation failure maps
  through the existing typed
  `NftRuleInstallFailed { op: "observe-shared", source }` path;
- the adapter never authors `PostconditionMismatch` and never calls the
  fresh-process `converge_shared` path at runtime;
- the stateful S19-A body snapshots the exact owned program, three dynamic-set
  inventories, ordered foreign bytes, mutation journal, and remaining schedule
  around both the canonical-wrong-target success and lower-observe-error rows;
  the sole allowed harness delta is one `Observe` journal entry; and
- the Lima S19-A body retains public-host entry, generation/notification
  observation, exact semantic/foreign complement, and zero nft mutation. It
  proves adapter no-rewrite only and claims no publication, retry, deadline, or
  fail-stop.

The first structured `PostconditionMismatch` is now assigned exclusively to
the `02-03` published worker, which owns the comparison between the observed
identity and its recorded target. This removes the prior impossible test
requirement without reviving the private helper, calling `converge_shared`, or
adding API.

The exact transitioned name
`runtime_present_wrong_target_and_observe_error_are_non_mutating`, Lima name
`shared_program_valid_wrong_target_observation_is_non_mutating`, markers,
fully-qualified selectors, status, and expected result/error semantics agree
across feature delta, DISTILL matrix, RED classification, and roadmap.

### P02-19 disposition — CLOSED

The normative owner/universe statements are now unambiguous:

- `02-03` ends at two published sockets/addresses, two task slots, node guard,
  lifecycle/publication state, capability registry/elements/handles, one
  compare-and-return structured conflict, and sealed worker relinquishment;
- `02-03` explicitly owns no clock, attempt count, deadline, EXEC mutation,
  request sender, or terminal orchestration;
- `03-03` exclusively owns the injected clock, one-second detection, twenty
  completed attempts on the 250 ms cadence, five-second deadline, paired EXEC
  FailStop, typed `IpRules/RecoveryDeadlineExceeded` request, supervisor
  shutdown token, and terminal orchestration; and
- the exact test-home paragraph now names worker acceptance only for
  publication/conflict/relinquishment and the control-plane source-local module
  for clock/attempt/deadline/request behavior.

This matches the roadmap's layered repeated-ID protocol: S19-A is independently
approvable in `02-02`, `02-03` supplies a non-closing published-worker
prerequisite, and only `03-03` closes S-ND295-19 through the production-used
private `SharedNetworkSupervisorHandle::run_mtls_owner` future.

### Boundary and drift audit

The correction adds no method, trait, parameter, error variant, guard
operation, persisted state, reverse dependency, or public/test-only seam.
`run_mtls_owner` remains module-private and production-used inside the sole
retained supervisor task. Control-plane already depends on worker/core; worker
does not depend on control-plane. Terminal guard relinquishment remains with
the existing worker shutdown owner, joined by the existing `ServerHandle`
terminal path; the private runtime future does not become a second shutdown
owner.

### Mechanical checks

| Check | Iteration-16 result |
| --- | --- |
| Canonical wrong-target observation | PASS — `Ok(Some(identity))`, not adapter-authored mismatch. |
| Lower observe error | PASS — existing typed `NftRuleInstallFailed` with exact operation/source. |
| Stateful no-mutation universe | PASS at contract — exact snapshots plus one Observe journal delta only. |
| 02-03 publication prerequisite | PASS — one worker conflict, guard retention, sealed relinquishment; no runtime budget claim. |
| 03-03 runtime closure | PASS — sole owner of detection, cadence, deadline, EXEC FailStop, typed request, and terminal orchestration. |
| Test names/homes/selectors/traceability | PASS — aligned and honestly `PENDING_AUTHORING` where absent. |
| API/dependency/ownership drift | PASS — none. |
| Roadmap JSON / formatting / diff hygiene | PASS — `jq empty`, `cargo fmt --all -- --check`, and `git diff --check`. |

### Iteration-16 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-15 — worker body could not own bounded fail-stop | **CLOSED at DESIGN** by the approved three-layer S19 allocation and private control-plane future. |
| DESIGN-P02-18 — impossible adapter-authored structured mismatch | **CLOSED** by exact observe result/error semantics and worker-owned comparison. |
| DESIGN-P02-19 — stale `02-03` retry/fail-stop ownership prose | **CLOSED** by exact worker/control-plane universe and test-home separation. |
| DESIGN-P02-16/P02-17 | Unchanged; remain acceptance-translation findings outside this correction review. |

No unresolved P02-18/P02-19 DESIGN finding remains.

## Iteration 16 final verdict

# APPROVED

The correction is exact and non-expansive. S19-A now proves only what the
observe-only Host adapter can expose, the published worker is the first owner
of a structured target conflict, and the existing private control-plane
supervisor alone owns one-second detection, twenty 250 ms attempts, the
five-second typed `IpRules` fail-stop, and terminal orchestration. Traceability,
test homes, selectors, dependency direction, and public-surface constraints are
consistent. This approval closes P02-18/P02-19 only; P02-16/P02-17 and pending
S19 acceptance authoring/review continue to block roadmap execution.

## Iteration 17 — final D15/S19 DESIGN-to-DISTILL translation review

### Scope

This iteration reviews the final authored D15/S19 acceptance translation
against approved Iteration 16: exact layer homes and names, public-adapter
no-mutation, published-worker prerequisite, private `run_mtls_owner` closure,
no API/ownership drift, and preservation of P02-16/P02-17 evidence.

### Translation portions that pass

The layer allocation is preserved exactly:

- S19-A source-local
  `runtime_present_wrong_target_and_observe_error_are_non_mutating` lives under
  the worker-private stateful seam but enters through public
  `HostMtlsIntercept::observe_shared`. Canonical wrong target returns
  `Ok(Some(identity))`; partial/foreign/duplicate/malformed/lower observation
  returns the existing typed error. Every row proves one Observe journal entry,
  no conditional mutation, and exact owned/dynamic/foreign complement.
- S19-A Lima
  `shared_program_valid_wrong_target_observation_is_non_mutating` drives public
  `HostMtlsIntercept`, exact canonical old/new identities, stable generation/
  rule snapshots, zero nft notifications, complete target-table equality, and
  unrelated foreign-table preservation. It claims no worker comparison or
  runtime budget.
- The `02-03` worker body now constructs an exact canonical different non-zero
  leg-F target through `SharedIpInterceptIdentity`, proves one
  `PostconditionMismatch` from `audit_shared_owner` and one from
  `converge_shared_owner`, asserts zero bind/fresh convergence/target rewrite,
  retains the node guard, and proves sealed worker relinquishment. Its removed
  manual retry loop receives no closure credit.
- S19-B
  `published_wrong_shared_target_retries_on_production_cadence_and_emits_one_typed_fail_stop`
  lives in the control-plane-private acceptance module, constructs a real
  worker, invokes the exact production-private
  `SharedNetworkSupervisorHandle::run_mtls_owner` future, uses the same
  `SimClock` for EXEC wiring, and scripts only accepted adapter state. No
  fixture retry loop, public constructor, reverse dependency, or test-only
  production method is introduced.
- The private production scaffold has the exact approved signature and remains
  non-public. Control-plane already depends on worker/core; no reverse edge or
  second retained supervisor is added.
- Names, exact selectors, Contract Shape declarations, markers, RED results,
  scenario IDs, and `02-02`/`02-03`/`03-03` traceability agree across feature
  delta, DISTILL, RED classification, and roadmap. All three S19 bodies are
  honestly reasoned-pending/RED at current production scaffolds.

P02-16/P02-17 evidence remains intact. Source-local rollback rows assert actual
Rust error-chain sources, exact conditional journals, and independent zero-port
rejection. Lima success compares against exact requested semantic identities;
the ambiguity table seeds valid IP plus same-name foreign family and rebuilds
all eight rules around a schema-only conflict. No prior falsifier was weakened.

Two blocking S19-B test defects remain.

### Finding DESIGN-P02-20 — Blocking: S19-B makes `run_mtls_owner` a second worker-shutdown owner

**Evidence:**

- Iteration 16 approved terminal ownership explicitly: the private future
  sends the fail-stop request and parks on its shutdown token; the existing
  `ServerHandle` terminal path remains the owner that calls
  `MtlsInterceptWorker::shutdown_owner`, then shuts down/joins the supervisor.
  `run_mtls_owner` must not become a second worker-shutdown owner.
- The production contract still says cancellation makes `run_mtls_owner`
  return normally and does not assign it worker shutdown
  (`feature-delta.md:4056-4068`).
- The authored S19-B body calls only `owner.shutdown().await`, which merely
  cancels the supervisor token and joins its task, then asserts that the worker
  is already `OwnerShutdown`/`NotStarted` and that its node guard was privately
  relinquished (`overdrive-control-plane/src/lib.rs:1733-1742`). It never calls
  the existing worker terminal owner or a `ServerHandle` terminal path.

**Consequence:** For the body to become green, DELIVER must add
`mtls_worker.shutdown_owner()` inside `run_mtls_owner`. That duplicates the
existing `ServerHandle` shutdown ownership, changes approved private behavior,
and can double-drive worker shutdown. Leaving the future faithful to Iteration
16 makes the acceptance body impossible.

**Required bounded disposition:** Preserve the approved future: after the
typed request it parks, and cancellation returns normally. In the body, execute
the existing production terminal order—worker `shutdown_owner()` through the
existing terminal-owner path, followed by private supervisor shutdown/join—
then assert sealed relinquishment and terminal worker state. Do not move worker
shutdown into `run_mtls_owner`, add a shutdown parameter/error, or invent a
second owner.

### Finding DESIGN-P02-21 — Blocking: the S19-B loop does not prove exact 250 ms cadence or fail-stop-not-before-five-seconds

**Evidence:**

- The body correctly proves no detection at 999 ms and recovery begins at one
  second (`lib.rs:1692-1697`).
- For each recovery attempt it then advances 250 ms and waits only until
  `progress.attempts >= expected` (`:1571-1587`, `:1699-1702`). It never asserts
  the previous attempt count remains unchanged before the tick, never checks
  the recovery snapshot's exact elapsed duration after each attempt, and uses
  `>=` rather than equality.
- An implementation that performs the first attempt immediately at detection,
  batches multiple attempts after one wake, or reaches FailStop at 4.75 seconds
  can still satisfy the loop: later ticks occur after the request is already
  queued, the final total can remain twenty observations, and the body does not
  call `shutdown_requested` until all twenty harness ticks have elapsed.
- The final request assertion proves its reported fields are `20`/`5s`; it does
  not prove the production attempt schedule or request emission time generated
  those fields honestly.

**Consequence:** The body can pass a retry loop that violates the exact
twenty-times-250-ms recovery contract while fabricating or delaying observation
of the expected final receipt.

**Required bounded disposition:** Before each tick, assert the recovery state
is still exactly attempt `n-1`, its elapsed duration is the prior 250 ms
multiple, and FailStop/request has not occurred. Advance logical time to each
deadline, then wait for and assert exactly attempt `n` with elapsed
`n * 250 ms`—not `>=`. Before the twentieth deadline, prove no request/FailStop;
at the twentieth, prove the single exact request becomes observable. The
harness may advance `SimClock` and observe production state, but it must not
author retries or requests.

### Boundary and mechanical checks

| Check | Iteration-17 result |
| --- | --- |
| S19-A public observe semantics/no mutation | PASS. |
| Lima canonical identity/generation/foreign complement | PASS. |
| `02-03` worker comparison/guard prerequisite | PASS and non-closing. |
| Private `run_mtls_owner` signature/home/dependency | PASS. |
| IpRules component, typed cause, attempts and elapsed fields | PASS for final receipt. |
| Exact per-attempt cadence / not-before deadline | FAIL — P02-21. |
| Terminal ownership/relinquishment join | FAIL — P02-20. |
| P02-16/P02-17 evidence preservation | PASS. |
| Public API / error / owner drift | PASS in production scaffolds; the S19-B test would force private ownership drift unless remediated. |
| Selectors/markers/traceability | PASS. |
| Roadmap JSON / formatting / diff hygiene | PASS — `jq empty`, `cargo fmt --all -- --check`, and `git diff --check`. |

### Iteration-17 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-15/P02-18/P02-19 | Remain CLOSED at DESIGN. |
| DESIGN-P02-16 — Lima falsifiers | **CLOSED in final translation**; exact target and isolated ambiguity evidence is preserved. |
| DESIGN-P02-17 — source chain/journal/zero-port | **CLOSED in final translation**; all required assertions are preserved. |
| DESIGN-P02-20 — S19-B duplicates worker shutdown ownership | OPEN, blocking DISTILL translation. |
| DESIGN-P02-21 — S19-B does not prove exact cadence/deadline | OPEN, blocking DISTILL translation. |

## Iteration 17 final verdict

# CHANGES_REQUIRED

The final translation preserves the approved adapter, worker, and private
control-plane layers; exact names/homes/selectors; P02-16/P02-17 falsifiers;
and the no-public-surface/no-reverse-dependency contract. S19-B is not yet an
executable specification of Iteration 16: its terminal assertions require the
private future to become a second worker-shutdown owner, and its logical-time
loop permits early or batched attempts while still observing a final 20/5s
receipt. Remediate P02-20/P02-21 inside the existing body without changing
production API or ownership, rerun exact RED classification, and re-review
before roadmap approval.

## Iteration 18 — P02-20/P02-21 correction review

### Scope

This iteration reviews only the proposed P02-20/P02-21 correction: private
future cancellation, sole terminal-owner ordering, exact 249 ms + 1 ms cadence,
4,999/5,000 ms request boundaries, no attempt 21, and closed pre-terminal
effect journals. Previously approved S19 layer homes and APIs are not reopened.

### P02-20 ownership correction — PASS

The terminal contract now preserves Iteration 16 exactly:

- `run_mtls_owner` detects, recovers, enters FailStop once, sends the typed
  request once, and parks on the existing shutdown token;
- cancellation makes the private future return normally;
- `run_mtls_owner` never invokes `MtlsInterceptWorker::shutdown_owner` and does
  not gain a shutdown result, callback, or second ownership field;
- the source-local S19-B body receives the request through a real private-field
  `ServerHandle` fixture and invokes existing `ServerHandle::shutdown`;
- that established terminal owner alone awaits worker drain/sealed guard
  relinquishment, resolver shutdown, then private supervisor cancellation and
  join; and
- guard retention belongs to the pre-terminal supervisor phase, while terminal
  worker state and zero guard Drop belong to the disjoint ServerHandle phase.

This is implementable from existing private fields and methods in the same
control-plane module. It adds no API, reverse dependency, error variant, or
second task/owner. The closed pre-terminal journal is also exact and sufficient:
one detection observe, one designed `quiesce_managed_taps`, twenty attempt
observes, zero bind/fresh convergence/guard Drop, and no unrelated
provision/teardown/probe/sweep/shared-converge/shared-audit/repeat-quiesce/
element effect.

### P02-21 cadence correction — directionally correct, one blocking specification defect remains

The correction properly requires equality rather than `>=`, separates each
deadline into 249 ms + 1 ms, proves no request at 4,999 ms, requires one exact
request at 5,000 ms, and prohibits attempt 21 or a second request. The harness
advances only `SimClock` and observes production snapshots/journals; it does not
author attempts or request values.

However, the exact prose currently requires two impossible or racy
observations.

### Finding DESIGN-P02-22 — Blocking: the boundary oracle conflates elapsed-time projection with stable state and requires an unobservable terminal recovery snapshot

**Evidence:**

- Before each deadline the proposed body records `attempts = n-1` and
  `elapsed = (n-1)*250 ms`, then advances 249 ms and says those values/state
  remain unchanged (`feature-delta.md:4232-4240`;
  `distill/test-scenarios.md:141-149`). `GuestNetworkExecSupervisor::recovery_progress`
  computes `elapsed` from the injected clock on every read. After the tick it
  must therefore be `(n-1)*250 ms + 249 ms`, even though attempts, journal,
  component, and request remain unchanged. Requiring elapsed to remain at the
  prior boundary is incompatible with the approved clock semantics.
- The prose also requires a Recovering snapshot with exactly attempt `n` after
  the final one-millisecond tick for every `n = 1..=20`. On attempt 20 the same
  production future completes the attempt, synchronously calls `fail_stop`, and
  sends the request. There is no required await/yield between
  `complete_attempt(Some(IpRules))` and `fail_stop`; the task awaiting
  `recovery_progress()` is not guaranteed to observe the transient
  Recovering(attempts=20) state before it becomes FailStop.
- The final typed request already is the authoritative attempt-20 receipt:
  `IpRules`, `RecoveryDeadlineExceeded`, `attempts = 20`, `elapsed = 5s`.
  Requiring both that receipt and an observable transient snapshot would force
  a production yield/test seam or make the test scheduler-dependent.

**Consequence:** An acceptance designer cannot implement the stated oracle
against the approved state machine without asserting a false elapsed value or
adding production scheduling behavior solely to expose attempt 20.

**Required bounded disposition:** Specify the boundary oracle as follows:

- before attempt `n`, assert exact attempts `n-1`, component `IpRules`, elapsed
  `(n-1)*250 ms`, empty request, and exact journal prefix;
- after advancing 249 ms, assert attempts/component/journal/request are
  unchanged **and elapsed is exactly the prior boundary plus 249 ms**;
- for attempts 1..19, advance the final 1 ms and assert an observable
  Recovering snapshot with exact attempts `n` and elapsed `n*250 ms`;
- before attempt 20, prove attempts 19 and no request at 4,999 ms; at 5,000 ms,
  await the single typed request and use its `attempts=20, elapsed=5s` fields as
  the terminal receipt rather than requiring an intermediate recovery snapshot;
  and
- advance later time and prove FailStop remains terminal, journals remain
  closed, no attempt 21 occurs, and no second request appears.

Do not add a yield, timer accessor, request probe, public state accessor, or
test-only production hook.

### Mechanical and boundary checks

| Check | Iteration-18 result |
| --- | --- |
| `run_mtls_owner` fail-stop/send/park only | PASS. |
| Cancellation returns normally | PASS. |
| Sole `ServerHandle` terminal worker ownership | PASS. |
| Pre-terminal closed journals | PASS. |
| 249 ms + 1 ms attempt boundaries | PASS in intent; elapsed assertion requires P02-22 correction. |
| 4,999 ms no request / 5,000 ms one request | PASS in intent; terminal receipt must replace transient attempt-20 snapshot. |
| No attempt 21 / no second request | PASS. |
| API/dependency/ownership scope | PASS — no drift. |
| Roadmap JSON / formatting / diff hygiene | PASS — `jq empty`, `cargo fmt --all -- --check`, and `git diff --check`. |

### Iteration-18 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-20 — duplicated worker shutdown ownership | **CLOSED** by real ServerHandle terminal ordering. |
| DESIGN-P02-21 — cadence/deadline oracle | **PARTIALLY CLOSED** — exact boundaries and terminal uniqueness are selected; P02-22 must correct observable snapshots. |
| DESIGN-P02-22 — false 249 ms elapsed / unobservable attempt-20 snapshot | OPEN, blocking. |

## Iteration 18 final verdict

# CHANGES_REQUIRED

The correction restores sole terminal ownership, cancellation semantics, closed
effect journals, exact retry boundaries, request uniqueness, and scope/API
discipline. One narrow oracle defect remains: elapsed time necessarily advances
during each 249 ms pre-boundary interval, and attempt 20 transitions directly
to FailStop without an observable Recovering(20) guarantee. Correct P02-22 in
the test contract without changing production scheduling or surface, then
re-review before acceptance-body remediation or roadmap approval.

## Iteration 19 — P02-22 correction re-review

### Scope

This iteration re-reviews only P02-22's boundary oracle: independent elapsed
advance during 249 ms subintervals, observable Recovering snapshots through
attempt 19, the 4,999/5,000 ms terminal boundary, and no attempt 21 or second
request. P02-20's terminal ownership and all earlier D15 layers remain fixed.

### P02-22 disposition — CLOSED

The corrected oracle now matches `GuestNetworkExecSupervisor` and `SimClock`
semantics exactly:

- detection at one second produces Recovering(`IpRules`) with `attempts = 0`,
  `elapsed = 0`, one detection observe, one designed quiesce, and no request;
- before each attempt `n`, the snapshot is exactly `attempts = n-1`, component
  `IpRules`, and elapsed `(n-1)*250 ms`, with the exact journal prefix and empty
  request receiver;
- advancing 249 ms changes only the projected elapsed value to the prior
  boundary plus 249 ms. Attempt count, component, adapter/shared-owner journals,
  guard ownership, EXEC recovery state, and request receiver remain unchanged;
- for attempts 1 through 19, the final one-millisecond tick yields an
  observable Recovering snapshot with exact attempt equality and elapsed
  `n*250 ms`;
- at 4,999 ms the authoritative state remains Recovering with attempts 19 and
  no request;
- the 5,000 ms boundary is observed solely through one typed
  `IpRules/RecoveryDeadlineExceeded/attempts=20/elapsed=5s` FailStop request.
  No transient Recovering(20) snapshot is required or credited; and
- later logical time leaves FailStop and both closed journals byte-equal,
  produces no attempt 21, and yields no second request.

This removes both defects from iteration 18. Elapsed is no longer falsely
frozen during a clock advance, and the acceptance body no longer depends on a
scheduler-visible gap between synchronous attempt completion and FailStop.
The terminal request is the authoritative twentieth-attempt receipt.

### Ownership, cancellation, and feasibility

The correction preserves P02-20:

- `run_mtls_owner` calls FailStop, sends once, parks, and returns normally only
  after its shutdown token is cancelled;
- the private future never calls worker shutdown;
- the real private-field `ServerHandle` fixture receives the request and drives
  the existing production terminal order: worker drain/sealed relinquishment,
  resolver shutdown, then supervisor cancellation/join; and
- pre-terminal effects remain exactly one detection observe, one quiesce, and
  twenty attempt observes, with zero bind/fresh convergence/guard Drop and no
  unrelated owner operation.

The oracle is implementable without a yield added to production, timer/request
accessor, public state method, test-only callback, alternate clock, or duplicated
retry loop. The test harness controls only the already-injected `SimClock` and
observes existing private production state/channel fields from the source-local
child module.

### Mechanical checks

| Check | Iteration-19 result |
| --- | --- |
| 249 ms elapsed projection | PASS — elapsed advances; attempts/effects/request remain fixed. |
| Attempts 1..19 Recovering equality | PASS — exact equality, never `>=`. |
| 4,999 ms boundary | PASS — attempts 19, persistent Recovering, empty request. |
| 5,000 ms terminal boundary | PASS — one typed 20/5s request; no Recovering(20) dependency. |
| Later time | PASS — no attempt 21, second request, or journal delta. |
| Sole terminal owner / cancellation | PASS — unchanged ServerHandle ordering. |
| API/dependency/ownership scope | PASS — no drift. |
| Roadmap JSON / formatting / diff hygiene | PASS — `jq empty`, `cargo fmt --all -- --check`, and `git diff --check`. |

### Iteration-19 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-20 — duplicated worker shutdown ownership | Remains CLOSED. |
| DESIGN-P02-21 — cadence/deadline oracle | **CLOSED** by exact boundary-specific observation. |
| DESIGN-P02-22 — false elapsed / unobservable attempt-20 snapshot | **CLOSED** by independent elapsed assertions and terminal request receipt. |

No unresolved P02-20/P02-21/P02-22 DESIGN finding remains.

## Iteration 19 final verdict

# APPROVED

The corrected timing contract is exact, deterministic, and faithful to the
approved state machine. It distinguishes elapsed-time projection from attempt
mutation, observes only stable Recovering snapshots, uses the typed 20/5s
FailStop request as the sole terminal receipt, and preserves the singular
ServerHandle shutdown owner and closed effect journals. Approval covers the
P02-20/P02-21/P02-22 correction contract; the authored S19-B body must still be
remediated to that contract, reclassified, and independently reviewed before
roadmap execution.

## Iteration 20 — final D15/S19 translation re-review

### Scope

This iteration reviews the remediated executable D15/S19 translation against
Iteration 19: S19-A adapter behavior, the non-closing worker prerequisite,
S19-B's exact private production future, timing/request oracle, closed journals,
sole terminal ownership, and preservation of every earlier D15 falsifier.

### S19-B executable contract

The authored body now implements the approved oracle rather than merely
describing it:

- it constructs a real `MtlsInterceptWorker` over a test-local implementation
  of the accepted port and uses `SharedIpInterceptIdentity::for_listener_ports`
  for both the healthy identity and a canonical different non-zero leg-F
  target;
- it invokes the exact module-private production
  `SharedNetworkSupervisorHandle::run_mtls_owner` future, not a copied retry
  loop or test-only wrapper;
- it uses one `SimClock` shared with `GuestNetworkExecWiring` and proves no
  detection at 999 ms;
- at one second it observes Recovering(`IpRules`) with attempts 0, elapsed 0,
  one adapter observation, one `TapSetDown`, an empty request receiver, and no
  other effect;
- for attempts 1 through 19 it asserts the exact prior snapshot/journal,
  advances 249 ms and observes only elapsed advance, then advances the final
  one millisecond and waits for equality with the exact new attempt and elapsed
  boundary;
- at 4,999 ms it observes attempts 19 and no request;
- at 5,000 ms it consumes one exact
  `IpRules/RecoveryDeadlineExceeded/attempts=20/elapsed=5s` request, treats that
  receipt as the sole attempt-20 oracle, and confirms Recovering has become
  FailStop;
- after another five seconds it proves no attempt 21, second request, adapter
  delta, shared-owner delta, or supervisor return; and
- it then calls real `ServerHandle::shutdown`, whose existing terminal order
  owns worker drain/sealed relinquishment before supervisor cancellation/join.
  Return is the join oracle; the private runtime future never calls worker
  shutdown.

The pre-terminal journals are mechanically closed. The intercept delta is
exactly zero bind, zero fresh shared convergence, twenty-one observations,
zero allocation installs, and zero guard Drop. The shared-network owner journal
is exactly `[TapSetDown]`. Provision, teardown, startup probe, sweep,
shared-switch converge/audit, repeat quiesce, element install, and
relinquishment are absent until the terminal ServerHandle phase.

### Layer and non-regression audit

All earlier D15 layers remain aligned:

- S19-A source-local uses public `observe_shared`, returns canonical identity
  or the existing typed observation error, asserts one Observe journal entry,
  and preserves the complete state/complement;
- S19-A Lima proves exact canonical wrong-target observation with stable rule/
  generation snapshots, zero notification, and foreign complement;
- the `02-03` worker body proves exact target recording, one structured
  compare-and-return conflict, zero bind/fresh convergence/target rewrite,
  retained guard, and sealed relinquishment only;
- S19-B alone owns clock, detection, attempts, deadline, EXEC FailStop, typed
  request, and terminal orchestration in `03-03`;
- P02-16's exact requested identities, valid-IP-plus-foreign-family row, and
  schema-only conflict remain present; and
- P02-17's actual `Error::source()` assertions, exact conditional journals,
  and independent zero-leg-F/zero-leg-C refusal remain present.

The exact names, homes, Contract Shape lines, markers, selectors, RED
classification, and repeated-ID traceability match across source, feature
delta, DISTILL, and roadmap. S19-B exact-selects one body and fails at the
earlier worker owner-start production scaffold, so its later oracle is compiled
without fabricating an executable consequence.

### API, dependency, and ownership audit

No public or architectural surface was added beyond the already-approved D15
error/netlink scaffolds. `run_mtls_owner` remains private and production-used;
the source-local child module can call it without exporting it. Control-plane
depends on worker/core and no reverse edge exists. One retained supervisor task,
one request receiver, one EXEC supervisor, one worker owner, and one
ServerHandle terminal path remain the complete ownership graph.

Several feature-delta status sentences still say P02-20/P02-21/P02-22
"requires remediation" or is merely "PROPOSED" even though the body and RED
classification now contain the reviewed correction (for example
`feature-delta.md:2530`, `:6805`, `:6841-6863`, and `:7287`). This is
post-review status bookkeeping, not a competing signature, behavior, test
home, or owner contract. It should be updated when this verdict is reconciled,
but it does not invalidate the executable translation.

### Mechanical and completeness checks

| Check | Iteration-20 result |
| --- | --- |
| S19-A source-local/Lima no mutation | PASS. |
| `02-03` published-worker prerequisite | PASS and non-closing. |
| Private S19-B production future | PASS. |
| 999 ms / one-second detection | PASS. |
| Attempts 1..19, 249 ms + 1 ms equality | PASS. |
| 4,999 ms no request / 5,000 ms one 20/5s request | PASS. |
| No Recovering(20), attempt 21, or second request | PASS. |
| Closed pre-terminal effect journals | PASS. |
| Real ServerHandle terminal ownership/join | PASS. |
| P02-16/P02-17 falsifiers | PASS and unchanged. |
| Names/homes/selectors/markers/traceability | PASS. |
| Public API/dependency/owner drift | PASS — none. |
| Canonical completeness | PASS — 15/15 executable checks. |
| Roadmap JSON / formatting / diff hygiene | PASS — `jq empty`, `cargo fmt --all -- --check`, and `git diff --check`. |
| Mutation testing | NOT RUN — final DELIVER gate only. |

### Iteration-20 finding disposition

| Finding | Status |
| --- | --- |
| DESIGN-P02-15 through DESIGN-P02-19 | Remain CLOSED. |
| DESIGN-P02-16/P02-17 translation gaps | Remain CLOSED in the final bodies. |
| DESIGN-P02-20 — duplicated shutdown ownership | CLOSED in the executable S19-B body. |
| DESIGN-P02-21 — aggregate/non-falsifying cadence | CLOSED in the executable S19-B body. |
| DESIGN-P02-22 — false elapsed/transient attempt-20 oracle | CLOSED in the executable S19-B body. |

No unresolved D15/S19 DESIGN-to-DISTILL finding remains.

## Iteration 20 final verdict

# APPROVED

The final D15/S19 translation is exact, executable, layered, and non-expansive.
It proves adapter no-rewrite, worker comparison/guard ownership, private
control-plane cadence/deadline/fail-stop, and the singular ServerHandle terminal
path at their correct boundaries. The 249 ms/1 ms schedule, 4,999/5,000 ms
transition, terminal request, no-attempt-21 complement, and closed effect
journals are mechanically falsifying. All earlier D15 acceptance evidence and
API/ownership constraints remain intact. Independent roadmap validation remains
the final pre-DELIVER approval gate.
