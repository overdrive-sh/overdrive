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
