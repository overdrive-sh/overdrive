# DISTILL upstream issues — netns-density-295 (vsock attachment replacement)

Back-propagation per `nw-distill` § "Document Update (Back-Propagation)". Each
item names the prior-wave artifact, the gap, the scenarios it blocks, and the
**decision needed**. DISTILL did not work around any of them: no production
surface the feature-delta does not pin was added, and no test was written over
an invented seam. The blocked scenarios keep their GIVEN/WHEN/THEN specs in the
piece docs; the never-compiled drafts that called invented surface were removed
from the tree (their text is in commit `0d3b9242f` for reference).

Severity: **BLOCKER** = blocks authoring of compiling acceptance tests or
conformance drivers; **HIGH** = blocks one scenario family; **LOW** = a
choice the scaffold had to make that DESIGN should confirm.

Status as of 2026-10-09 (this DISTILL pass).

**DESIGN response, 2026-10-09 — every item below is closed.** The
architect closed the DESIGN gaps under the user's delegation (decisions
DG-1 to DG-16, feature-delta § *DISTILL design-gap closure* and
§ *DISTILL design-gap decisions*). Each item carries a **Resolution** line.

---

## UI-C1 — Single-cut signature replacements (G-C1(a)) — BLOCKER

**Artifact:** `feature-delta.md` § "[REF] Interface contracts" and QPROJ-2..6.
**Gap.** The pinned signatures below *replace* live production signatures that
the old TAP/bridge model's callers and GREEN tests still use. A Mandate 7
scaffold of any of them is the single-cut migration itself (it breaks the live
callers), so DISTILL cannot add it as an additive RED scaffold.

| Pinned (feature-delta line) | Live today |
|---|---|
| `GuestAddressPool::new(node_prefix, gateway, Arc<dyn GuestCidClaim>, Arc<dyn Entropy>)` (17857) | `new(node_prefix, bridge, gateway, dns)` — `overdrive-control-plane/src/guest_network.rs:581` |
| `GuestAddressPool::assign(alloc, &[ListenPort]) -> Result<GuestNetworkPlan>` (17870) | `assign(alloc)` — `:597` |
| `GuestNetworkPlan` without `bridge()`, `assignment() -> &GuestTransportAssignment`, `+ listen_ports()` (19250) | `:75`, `assignment() -> &GuestNetworkAssignment` |
| `AllocationSpec.transport: Option<GuestTransportAssignment>` (17634) | `AllocationSpec.network` — `overdrive-core/src/traits/driver.rs:329` |
| `Vmm::create(&config, ClaimedGuestCid)` (17647) | `create(&config)` — `overdrive-core/src/traits/vmm.rs:148` and ~20 `Vmm` impls |
| `GuestNetworkProvisioner::activate -> Result<ForwardingActivation>` (19114) | `-> Result<TapActivation>` — `guest_network.rs:157` |
| `SharedGuestNetworkOwner::{quiesce_forwarding, restore_forwarding}` (19197, 19208) | `quiesce_managed_taps`, `restore_quiesced_taps` — `:210`, `:212` |
| `HostSharedGuestNetworkOwner` constructor taking `MtlsResolve`, the mTLS worker, the VIP ranges and the wildcard port set (19147-19150, 19269) | `pub(super) fn new()` — `:2522` |
| `SharedGuestNetworkComponent` variant set, derives `Ord` (20005) | `Bridge`, `TcxLink`, … — `overdrive-core/src/guest_network.rs:66` |
| `GuestNetworkOperation` variant set (19986); `GuestNetworkError::Tcx` → `Flow` (19941) | `guest_network.rs:241`, `:500` |
| `GuestNetworkExecSupervisor::recovery_progress() -> Vec<SharedGuestNetworkRecovery>` (U-QP-3) | `-> Option<SharedGuestNetworkRecovery>` — `overdrive-core/src/guest_network.rs:301` |
| `BeaconMessage::Ready { pid, port, launch: LaunchToken }` (18088) | `Ready { pid, port }` — `overdrive-core/src/vm/beacon.rs:177` (+ green `vm_beacon_roundtrip`) |
| `ServiceBackendsResolve::new(.., vip_ranges)` (19579) | `new(store, frontend)` — `mtls_resolve_adapter.rs:650` |

**Blocks:** P1-S2..S9, P1-S13 (pool + `cid_lease` driver); P2-S3..S13, P2-S15
(owner flows + `owner_flows` drivers); P2-S14 (beacon READY); P3 host side and
P3-S8 host driver; P4-S6..S15, P4-S17, P4-S18; P5-S2, P5-S5..S7, P5-S9; P6-S1..S6,
P6-S8, P6-S10, P6-S12..S15, P6-C1, P6-C2.
**Decision needed:** either (a) authorize DISTILL to land these replacements now
as scaffolds — which is the DELIVER roadmap reconciliation's single cut and
deletes the old-model callers and tests with it — or (b) sequence the
remaining DISTILL work as one increment after each DELIVER step that lands an
owner's surface (pool + claim: new-step 15; owner flows: 7; intake + steering
wiring: 9/13; VMM: 3; beacon: 6; supervisor: 09-01 rework), each increment
authoring that owner's acceptance bodies and its in-crate conformance driver
before the step that makes them green. **Recommendation: (b)**, which is the
user's G-C1 sequencing option and keeps each single cut in DELIVER.

**Resolution — DG-2.** The replacement table names every replaced signature
and its production callers; no shims. Every row but 14 lands in one DELIVER
step, the **attachment cut** (no smaller cut leaves the tree compiling, green
and free of dead mechanism). Additive surface (core codecs, constants, the
three seams and their sims, `GuestFlowOwnerPorts` / `with_flow_io`, the
observation accessors, the `overdrive-init` setup and flow-owner modules) is
scaffolded by DISTILL now with compiled `#[ignore = "pending DELIVER step N"]`
bodies — including the `overdrive-init` guest-side conformance drivers.
Bodies that name a replaced signature are authored by a DISTILL increment at
the start of the attachment cut, in its working tree, and committed compiled
with the cut. No unregistered file, no always-false `cfg`, no rule
exception.

## UI-C2 — Owner effect seams are crafter-shaped (G-C1(b)) — BLOCKER

**Artifact:** `feature-delta.md` § "Driven port — host forwarder",
"Simulation seam" (18900-18905); guest-prefix steering "Simulation seam"
(19085-19091); QPROJ-1 "Linearization and stepping" (20187-20194); QPROJ-7
(20594-20675).
**Gap.** The design pins the *read surfaces* of the seam sims
(`SimOwnerSocketObservation`, `SimSteeringObservation`, `SimSteeringEffect`,
`SimBeaconSocketObservation`, `SimGuestSocketObservation`) but leaves the seam
traits themselves ("one private effect seam the crafter shapes") to DELIVER.
Every owner acceptance test (seeded-sim lane) and every in-crate conformance
driver steps the owner through that seam (hold/release each effect, deliver
connections with buffered records, fail a steer). No seam sim can be written
before the seam trait exists, and writing the trait in DISTILL would invent
the crafter's internal structure.
**Blocks:** the same owner scenarios as UI-C1; P2-S14 (VM-driver beacon seam);
all ten in-crate drivers (`cid_lease` excepted, which needs only UI-C1).
**Decision needed:** either (a) DESIGN pins each seam's driving shape (the
effect set and the hold/release stepping the drivers use), or (b) accept the
seams as the crafter's and author the seam sims and drivers in the DISTILL
increment that follows the DELIVER step landing each seam (UI-C1 (b)).

**Resolution — DG-3 (host owner: `GuestFlowIo`, `HostGuestFlowIo`,
`SimGuestFlowIo`, typed move-only socket handles, `GuestFlowOwnerPorts`,
`HostSharedGuestNetworkOwner::{new, with_flow_io}`) and DG-4 (VM driver: the
core port `BeaconIo` with `BeaconSocket` / `BeaconConnKey`, `VsockBeaconIo`
in `overdrive-host`, `SimBeaconIo` in `overdrive-sim`, `BeaconIntake::bind(io)`
passed to `VmDriver::new`, `ServerConfig.vm_override: VmOverride { vmm,
beacon }`).** Option
(a): the seam traits, their production and sim adapters, and the owner
constructors are pinned with behavioural contracts; the sims' stepping and
scripting methods stay DISTILL's.

## UI-C3 — `overdrive-init` guest flow owner is unpinned (G-C2) — BLOCKER

**Artifact:** QPROJ-5 (20468-20542); guest adaptation contract (19633-19729).
**Gap.** QPROJ-5 pins `guest_observation()` and its value types, but not the
guest flow owner type, its constructor, its driving ports, the guest-owner
seam trait, or `SocketCookie` / `CellGeneration` (named crate-private). The
transport-token grammar is pinned (19650) but its parser and the binary's
error rendering are not. `overdrive-init` is binary-only, so tests must be
source-local; the previous drafts imported invented modules
(`overdrive_init::slots`, `overdrive_init::transport`) and an invented
`OVERDRIVE_INIT_CMDLINE` hook, and running today's binary as root on the test
host reaches `reboot(RB_POWER_OFF)`.
**Blocks:** P3-S2, P3-S4..S6, P3-S8 (guest), P3-S9 (guest half), P3-S10;
P6-S11; `owner_flows` guest side, `flows_half_guest`, `udp_slots` guest driver.
P3-S3 (reserved-port bind refusal) needs a real guest kernel and moves into
the P6 walking skeleton. The pure reserved-range facts are authored in
`overdrive-core` (`vsock_guest_transport_vocabulary.rs`).
**Decision needed:** pin the guest flow owner's type, constructor and driving
ports (or accept them as crafter-shaped and schedule a DISTILL increment after
DELIVER new-step 5).

**Resolution — DG-5.** `GuestFlowOwner::{new, dispatch_next, run,
guest_observation}`, the synchronous seam `GuestOwnerIo` (production
`KernelGuestOwnerIo`, sim `SimGuestOwnerIo`), the time port `GuestClock`
(`SimGuestClock`), `SocketCookie`, `CellGeneration`, `GuestOwnerFatal`
(event wait and setup-bound listeners only; control-session loss recovers
at the backoff); setup: `parse_transport_token`, `GuestTransportToken`,
`GuestTransportOps` (incl. `install_udp_slots`), `configure_guest_transport`,
the console line and every `Display` text. Tests are source-local, pass the
command line as a string, and never reach `main`.

## UI-HOST-1 — `VhostVsockCidClaim` has no pinned constructor — HIGH

**Artifact:** core vocabulary (17571-17573) names the host adapter
`overdrive-host::VhostVsockCidClaim` but no way to construct it.
**Blocks:** P1-S12 (four Tier-3 cells: exclusive claim, release at last close,
probe under the `serve` identity, held device refuses reclaim until dropped),
and therefore the Tier-3 pairing that QPROJ-7 requires before any conformance
projection may read `SimGuestCidClaim::held_cids`.
**Decision needed:** pin the constructor (e.g. `pub fn new() -> Self`, opening
`/dev/vhost-vsock` per claim, or a fallible open).

**Resolution — DG-6.** `#[must_use] pub fn new() -> Self` (infallible, opens
nothing); each `claim` opens a new instance of `VHOST_VSOCK_DEVICE`
(`/dev/vhost-vsock`, `O_RDWR | O_CLOEXEC`); an open failure is
`DeviceUnavailable`. No `Default`, no path override.

## UI-HOST-2 — VMM probe stage test seam — withdrawn

The probe stage this item asked to drive is withdrawn by the user's mandate:
Overdrive runs only with the vendored Cloud Hypervisor fork, so no probe
identifies the installed build and P6-S7 is withdrawn.

**Resolution — DG-1.** No subject; nothing to approve.

## UI-BEACON-1 — READY `launch=` error shape — HIGH

**Artifact:** Wire contracts, "Beacon READY" (18088-18093).
**Gap.** A token that fails `LaunchToken::from_str` is to be "a
`BeaconParseError` (the existing field-count and field variants)". The only
existing field variant, `InvalidInt`, carries a `ParseIntError` and cannot
represent a `LaunchTokenError`.
**Blocks:** the READY half of P2-S1 / P2-S14 (golden READY line with `launch=`;
malformed token refused). The old-model `vm_beacon_roundtrip` stays green
until the UI-C1 single cut replaces `BeaconMessage::Ready`.
**Decision needed:** pin the `BeaconParseError` variant for an invalid launch
token.

**Resolution — DG-7.** `BeaconParseError::InvalidLaunchToken { kind, raw,
source: LaunchTokenError }`; READY's arity is 3 (`FieldCount`), a third
field not shaped `launch=` is `MissingKey`.

## UI-FRAME-1 — No Rust datagram-frame codec is pinned — HIGH

**Artifact:** Wire contracts, "Datagram frame" (18208-18220) and the golden-bytes
requirement (18200-18206: "an empty frame and a maximum frame across the Rust
codec, the host programs and the guest programs").
**Gap.** Only the constants are pinned (scaffolded in
`overdrive_core::guest_transport::wire`). The previous draft
(`vsock_datagram_frame.rs`) tested an invented `DatagramFrame` /
`DatagramFrameError` API; it was removed.
**Blocks:** P3-S1 (Rust codec property and golden frames). The frame's kernel
behaviour is covered by the forwarder Tier-3 tests (`vsock_egress_unframe.rs`).
**Decision needed:** pin the Rust frame codec (type, `encode` / `decode`,
error type), or state that the golden frame set is BPF-only.

**Resolution — DG-8.** `DatagramFrame<'a>::{new, payload, encode, decode}`
and `DatagramFrameError::{Truncated, BadMagic, Oversize, LengthMismatch}` in
`overdrive_core::guest_transport::wire`; canonical decode only.

## UI-MARK-1 — `OWNER_EGRESS_MARK` value and the TPROXY mark are unpinned — HIGH

**Artifact:** D26-PINS F-6 (19434-19438).
**Gap.** The design pins "one dedicated bit … beside `MTLS_LEG_S_DIAL_MARK`"
and a unit test of pairwise-disjoint marks, but not the bit; the TPROXY mark is
the worker's private `TPROXY_FWMARK`.
**Blocks:** P5-S8 (Tier-3 host-internal output rule on marked sockets) and the
mark-disjointness unit test.
**Decision needed:** pin the bit value and how the disjointness test reaches
the TPROXY mark (export it, or place the test in `overdrive-worker`).

**Resolution — DG-9.** `OWNER_EGRESS_MARK = 0x4`; the TPROXY mark becomes the
core SSOT `MTLS_TPROXY_MARK = 0x1` (the worker's and netlink's private copies
are deleted); the disjointness test lives in `overdrive-core`.

## UI-WORKER-1..4 — forwarded-outbound details (from authoring P5-S10 cells) — LOW

1. `MtlsInterceptInstallError::stage()` label for `NotInterceptLive` /
   `DuplicateForwardedRegistration` is not pinned; the scaffold arm panics
   (`overdrive-worker/src/mtls_intercept_worker.rs:804`).
2. Dropping a *claimed* `ForwardedOutbound` "closes leg-F's side" (19542): FIN
   or reset is not pinned; the test asserts only "closed, no bytes".
3. "Cleared before a clean close" (19557): the test reads a clean close as a FIN
   in both directions while the registration is held.
4. Registration while the shared owner is not published: `NotInterceptLive` or
   an owner error is not pinned; the test covers only an allocation with no
   active capability on a live owner.
**Decision needed:** confirm or correct each reading.

**Resolution — DG-10.** (1) `"not_intercept_live"`,
`"duplicate_forwarded_registration"`. (2) Corrected: a claimed flow's leg-F
sockets close clean (FIN) only if the loopback leg ended cleanly in both
directions, else with reset; dropping an unclaimed one removes the
registration. (3) Confirmed. (4) Corrected: an unpublished shared owner is
`SharedOwner { source: NotStarted }` (as `start_alloc`), then
`NotInterceptLive`, then `DuplicateForwardedRegistration`; worker shutdown
is `OwnerShutdown` first.

## UI-NET-1 — `ListenPort` / `InterfaceIndex` constructor and parse error — LOW

**Artifact:** core vocabulary, "Newtypes (M-5)" (17680-17691).
**Gap.** "A validating constructor returning `Result`" is required but not
named, and the `FromStr` error type is not given. The scaffold adds no
constructor and uses the crate's existing newtype error, `IdParseError`; tests
construct both newtypes through `FromStr` only.
**Decision needed:** confirm the error type and name the constructor.

**Resolution — DG-11.** `ListenPort::new(u16) -> Result<_, ListenPortError>`,
`InterfaceIndex::new(u32) -> Result<_, InterfaceIndexError>` (`Zero`,
`Unparseable`), `get()`; `FromStr` uses the same errors, not `IdParseError`
(the scaffold is corrected at step 2).

## UI-WIRE-1 — A `ListenState` port inside the guest reservation — LOW

**Artifact:** control message table (18123): the port is "never a port in
`guest_reserved_ports()`".
**Gap.** Whether `ControlMessage::decode` refuses such a port, or only no valid
sender produces it, is not stated. The codec property generates ports outside
the reservation only.
**Decision needed:** state whether decode refuses it (and with which
`WireField`).

**Resolution — DG-12.** Decode refuses it: `InvalidField { field:
WireField::ListenPort }`.

## UI-CORE-1 — Location of `IntakePortBlockConflict` — LOW

**Artifact:** 17791: "added to overdrive_core (beside GuestNetworkError)";
`GuestNetworkError` lives in `overdrive-control-plane`.
**Decision needed:** name the crate. (Not scaffolded: no compiling test uses it
yet; P4-S15 is blocked by UI-C1.)

**Resolution — DG-13.** `overdrive-control-plane::guest_network`, beside
`GuestNetworkError`.

## UI-COV-1 — Steering and unframe failure classes reachable only by a seam — LOW

The adapter Tier-3 tests cannot induce: a verifier rejection at `load`, a
failing `BPF_LINK_UPDATE` (`Update`), a failing first-attach pin (`Pin`),
`steer` → `KernelMemory`, unframe attach `ENOMEM` (`KernelMemory`) and
`ERANGE`. The design assigns their coverage to the owner-seam sim (QPROJ-7,
R5-24, R5-34, R5-37) — blocked by UI-C1/UI-C2. Recorded so the owner increment
covers them; no decision needed beyond UI-C2.

**Resolution — DG-3.** `SimGuestFlowIo` scripts each class: `Load`, `Update`,
`Attach`, `Pin`, `steer` → `KernelMemory` / `MapUpdate` /
`IntakeAlreadySteered`, and `load_forwarder` → a `failed` unframe entry of
`KernelMemory` or `Attach` (`ERANGE`, `EEXIST`, `ENODEV`).

## UI-PL-1..7 — `prefix_landing` driver open items (carried from the second pass)

G-PL-1 the steering map's key layout is private but QPROJ-8 reads `entries`
from a map dump (the reader assumes `[IPv4 BE][port BE]`); G-PL-2 the route
protocol tag value is unpinned; G-PL-3 `quint` is provisioned in Lima but not
on the metal host, and this driver needs both a real guest and `quint`; G-PL-4
quint-connect has no definition-evaluation surface, so the oracle runs `quint
repl`; G-PL-5 the shared table's name is not restated in the vsock section (the
reader uses `ip overdrive-mtls`); G-PL-6 `run-native-metal.sh` selects every
ignored `native_metal_kvm_*` test, so it will run this one RED until its
DELIVER steps land; G-PL-7 `NODE_GUEST_PREFIX`, `GUEST_PREFIX_STEERING_PIN_DIR`
and `GUEST_INTAKE_HOST_PORT_BASE` are spelled in the harness until production
exports them (`GUEST_INTAKE_HOST_PORT_BASE` and
`GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX` now exist as scaffolded constants in
`overdrive_core::guest_transport`; `GUEST_PREFIX_STEERING_PIN_DIR` in
`overdrive_dataplane::guest_steering`; the harness may switch to them).
Additionally QPROJ-8 lists `marked` among kernel-queried inputs while the
driver takes it from the caller's configuration (QPROJ-1 source (d)).
**Decision needed:** G-PL-1, G-PL-2, G-PL-3, G-PL-5 and the `marked` source.

**Resolution — DG-14 (and DG-9 for G-PL-5).** G-PL-1: the key layout is a
contract (8-byte key: IPv4 BE, port BE, two zero bytes; 8-byte value = socket
cookie), encoder `guest_steering_key`. G-PL-2: `GUEST_PREFIX_ROUTE_PROTOCOL =
247`. G-PL-3: `quint` is provisioned on metal from the same version pin as
Lima, checked by the native preflight (DELIVER provisioning step). G-PL-4:
`quint repl` over the spec in place, confirmed. G-PL-5: `MTLS_SHARED_NFT_TABLE`.
G-PL-6: not DESIGN — the runner excludes tests whose ignore reason starts with
`pending DELIVER`. G-PL-7: `GUEST_NODE_PREFIX` / `GUEST_NODE_GATEWAY` in core.
`marked`: the caller (source (d)) — `serve` booted and not killed or stopped.

## UI-SCN-1 — Walking skeleton driven "through `overdrive deploy`" — LOW (doc)

P6-S13 / P6-S14 say "driven through `overdrive deploy`", which conflicts with
`tests/conformance/CLAUDE.md` and `.claude/rules/testing.md` ("Tests never spawn
the `overdrive` binary"). The in-process equivalent is `run_server` + the public
HTTPS API, as `prefix_landing` does. The piece doc is corrected; no design
decision needed.

**Resolution — DG-16.** Not a DESIGN gap; the correction stands.

## UI-P5-1 — Who owns the non-mesh egress cell (P5-S9)

The owner's `NonMesh` path lives in `overdrive-control-plane` (blocked by
UI-C1/UI-C2); the P5 doc's G-2 note assigns the end-to-end cell to the P6
walking skeleton. **Decision needed:** confirm P6-S13 owns it.

**Resolution — DG-16.** Confirmed, split: the owner-level `NonMesh` decision
is a seeded-sim scenario over `SimGuestFlowIo`, authored at the attachment
cut (DG-2); the end-to-end cell is P6-S13's.

---

## Round 2 — 2026-10-10 (this increment)

Raised while authoring the second DISTILL increment over DG-1..DG-16. Every
authored test and driver compiles against the pinned contract and is RED for
the right reason today (`red-classification.md`), but two items bear on
authored bodies: UI-R2-5 blocks the three guest drivers as settled evidence
(they are ignored with that note; a correct implementation may fail them
until the model decision), and UI-R2-1 leaves checklist item 6 failing for
every driver. UI-R2-16 and UI-R2-2 must be decided before the attachment cut.
Each item names the artifact, the gap, what it affects and the decision
needed. Severity as in round 1, plus **MEDIUM** (affects the fidelity of a
whole lane or a composition step, without blocking authoring) and **note**
(recorded for the record; no decision needed).

### UI-R2-1 — The guest-flow-owner specs declare no Quint `run` — HIGH (spec)

- **Artifact:** `specs/quint/guest-flow-owner/*.qnt`, `checks.toml`;
  `.claude/rules/testing.md` § "Quint conformance".
- **Gap:** the rule requires `#[quint_test]` pins for the named runs that
  encode the design's critical scenarios. The specs declare no `run`; the
  `checks.toml` witnesses are invariants (non-vacuity checks), not runs, so
  there is nothing for `#[quint_test]` to pin.
- **Affects:** every conformance driver (the four authored now and the seven
  at the attachment cut) runs `#[quint_run]` randomized traces only.
  Per-driver checklist item 6 requires both the `#[quint_test]` pins and the
  `#[quint_run]` traces, so it fails outright for every driver; having the
  `#[quint_run]` half does not satisfy it. Randomized traces may also never
  reach the critical scenarios: the non-vacuity witnesses are 12-14 steps
  deep, and nothing pins a trace that reaches them.
- **Decision needed:** the quint-modeler adds named runs to the specs (for
  example one per non-vacuity witness), which the drivers then pin with
  `#[quint_test]`. The alternative — DESIGN states that the witnesses suffice
  and `#[quint_run]` alone discharges the rule — contradicts
  `.claude/rules/testing.md` § "Quint conformance" ("Both trace modes") and
  would need a user change to that rule.

**Resolution — closed (feature-delta Round-2 decisions, approved by the user
2026-10-10).** The specs declare 109 named `run`s across eight modules: 103,
plus six `prefix_landing` runs on the `landing_machine` instance
`landing_ok`. Each is mapped to its scenario and decision id in
`specs/quint/guest-flow-owner/README.md` § "Scenario to named-run mapping"
and listed in `checks.toml`; the drivers pin them with `#[quint_test]`.

### UI-R2-2 — `VsockBeaconIo` has no pinned constructor — MEDIUM

- **Artifact:** feature-delta DG-4 (`BeaconIo` port, `VsockBeaconIo`
  production adapter, `BeaconIntake::bind(Arc<dyn BeaconIo>)`).
- **Gap:** composition must construct a `VsockBeaconIo` to pass to
  `BeaconIntake::bind`, but DG-4 pins no constructor (arguments, error type).
- **Affects:** the attachment-cut composition (DG-2 rows 15, 18:
  `VmDriver::new(.., beacon)`, `vm_override`) and the P6-S13 walking skeleton.
- **Decision needed:** pin `VsockBeaconIo`'s constructor signature and its
  failure type, before the cut. Suggested shape: `#[must_use] pub fn new() ->
  Self`, infallible, mirroring DG-6's `VhostVsockCidClaim::new` (the bind
  failure is `BeaconIntake::bind`'s, already typed).

**Resolution — closed by DG-4-CTOR (approved by the user 2026-10-10).**
`#[must_use] pub const fn new() -> Self`, infallible, opens nothing; no
`Default`, no port or address override; the bind failure stays
`BeaconIntake::bind`'s `BeaconIntakeError::Bind` (feature-delta DG-4).

### UI-R2-3 — `KernelGuestTransportOps` has no pinned constructor — LOW — CLOSED (internal structure)

- **Artifact:** feature-delta DG-5 (`overdrive-init` `transport` module).
- **Gap:** `KernelGuestTransportOps` must exist before
  `configure_guest_transport` runs and later feed `KernelGuestOwnerIo::new`;
  DG-5 pins no constructor for it.
- **Affects:** the `overdrive-init` `main` wiring cell (P6-S11, DG-2 row 16)
  and P6-S13 / P3-S3.
- **Decision needed:** pin the constructor (and whether it is fallible).

**Resolution — closed as internal structure.** `KernelGuestTransportOps` is
`pub(crate)` in the `overdrive-init` binary. The design pins only
`KernelGuestOwnerIo::new(ops: KernelGuestTransportOps)` (feature-delta
L21919); the constructor of the ops type is crafter structure (CLAUDE.md
§ "Implement to the design", scope: private and module-private types), not
an interface contract.

### UI-R2-4 — The golden maximum frame's payload bytes are unpinned — LOW — RECLASSIFIED (DISTILL-owned fixture data)

- **Artifact:** feature-delta DG-8 (`DatagramFrame`).
- **Gap:** DG-8 pins the header of the golden maximum frame but not its
  payload bytes. The Rust test
  (`vsock_datagram_frame::golden_maximum_frame_carries_the_inclusive_bound`)
  uses byte i = i mod 251 (header `OVD1 00 00 E6 78`).
- **Affects:** P3-S1; the host and guest BPF program goldens must use the same
  payload or the three goldens diverge.
- **Decision needed:** pin the golden payload (confirm i mod 251 or replace
  it).

**Resolution — reclassified as DISTILL-owned fixture data.** The golden
payload is test data, not an interface contract: DISTILL owns it. The payload
byte i = i mod 251 is the golden. The host and guest BPF-side goldens adopt
the same payload when they are authored. No DESIGN decision is needed.

### UI-R2-5 — Guest-side drivers cannot replay some reachable model situations faithfully — HIGH (model / design fidelity)

- **Artifact:** `specs/quint/guest-flow-owner/{owner_flows,udp_slots}.qnt`;
  the guest-side action maps (feature-delta § *Conformance observation surface
  and action map*, DG-15); the drivers
  `crates/overdrive-init/src/guest_flow/quint_{owner_flows,flows_half,udp_slots}.rs`.
- **Gap:**
  - (a) One guest clock (`GuestClock` / `SimGuestClock`) drives every timer,
    so advancing it for one modelled deadline (`GuestPairingTimeout`,
    `GuestAudit`, `IdleRelease`, `HcFlowError`, the reconnect backoff) fires
    every other armed timer; the audit tick period is unpinned.
  - (b) `udp_slots`' `MAXFID` has no production counterpart; the driver
    returns `DriverError::ModelBound` when a trace reaches it.
  - (c) `HcCleanClose` is not a separate production step: production clears
    linger and closes in the dispatch that completes the second half-close,
    which the acceptance test
    `guest_flow::tests::half_close::a_clean_close_clears_linger_before_closing`
    asserts. The model has a separate `HcCleanClose` step; it should merge it
    into the step that completes the second half-close.
  - (d) How the guest **and** host owners learn that a half-closed direction
    has drained is unpinned. `GuestIoEvent` has no drain-complete event, and
    § *Flow end* (feature-delta L19342-19346) names no re-check trigger for
    `drained(key, dir)`; the host side has the same gap. This is a
    contract-shaping DESIGN decision for the user: a kernel drain-complete
    event on both seams, or a bounded re-check (period and trigger). It is
    needed before new-step 5 (the guest owner) and the attachment cut (the
    host owner).
  - (e) An error or end on an installed but unrouted V_g is not observable to
    the owner until routing (`HcFlowError` before pairing;
    `GuestSeesHostClose` while `gf` is `acc`).
  - (f) Some `flows_ok` orders are not producible: a guest reconnect before
    `GuestLossAbort` of an old-session flow; `GuestLossAbort` with `gf.c`
    stale while a session is live.
- **Affects:** P2-S15 guest side, `flows_half_guest`, P3-S8 guest side. These
  may make a correct implementation fail the drivers. **Blocks the three
  guest drivers** (`quint_owner_flows`, `quint_flows_half`, `quint_udp_slots`)
  as settled evidence: they are ignored `pending DELIVER vsock new-step 5
  (guest adaptation in overdrive-init); model fidelity open, upstream
  UI-R2-5` and stay that way until the model decision.
- **Decision needed:** the architect / quint-modeler either refine the model
  (per-timer time actions with a guard-preserving abstraction, a pinned audit
  tick, merged clean close, an explicit drain-complete action) or state, per
  item, the production behaviour the driver must map the action to.

**Resolution — closed (feature-delta Round-2 decisions, approved by the user
2026-10-10).** The model is refined; the specs follow in the quint-modeler
round.

- (a) DG-5-A: each deadline is its arm instant plus its pinned constant;
  the audit tick is periodic at `GUEST_OWNER_AUDIT_PERIOD` = 1 s, fixed
  rate, and one tick runs the release audit, idle release, quarantine retry
  and `listener_map` retry; one input per `dispatch_next`, earliest deadline
  first, an event before a due deadline. The driver advances the clock to
  exactly the modelled timer's deadline.
- (b) DG-5-B: `MAXFID` is a model-checking bound with no production
  counterpart; it only disables minting, and the conformance instance sizes
  it above any trace's reach.
- (c) DG-5-C: the clean close happens in the dispatch that completes the
  second direction; the model merges `HcCleanClose` into that step.
- (d) DG-5-D: no owner copies payload bytes (the forwarder moves them;
  `drained` reads its exact K2 counters, ADR-0151 / ADR-0160), and the user
  excludes a kernel drain-complete event. The wait for a direction starts at
  its `EPOLLRDHUP` (or at `Paired` for a recorded early outer EOF), arms
  start + `FLOW_DRAIN_DEADLINE`, checks `drained` once, and re-checks every
  `FLOW_DRAIN_RECHECK_PERIOD` (10 ms, `overdrive_core::guest_transport::wire`)
  strictly before the deadline; the deadline aborts (`Abort`,
  `DrainTimeout`), a `drained` error aborts (`Abort`, `OwnerAbort`). Guest:
  owner deadlines in `next_deadline()`; host: `Clock::sleep` on
  `ports.clock`. No seam method is added (feature-delta § *Flow end*,
  "Drain wait").
- (e) DG-5-E: every installed, unrouted V_g is watched for terminal
  readiness; an error or reset aborts with `Abort`, `ReadHangup` ends the
  flow with no message.
- (f) DG-5-F: session loss aborts every flow and association of the lost
  session in the dispatch that observes it, before any reconnect; the model
  replaces per-flow `GuestLossAbort(h)` with `GuestSessionLossObserved(c)`.

The three guest drivers stay ignored until the specs carry these decisions
and their activating DELIVER step lands.

### UI-R2-6 — The guest owner's counters have no observation accessor — LOW

- **Artifact:** feature-delta QPROJ-5 (`guest_observation()`).
- **Gap:** `control_discarded`, `guest_park_exhausted` and
  `guest_udp_slot_quarantined` have no accessor, so "counted" outcomes cannot
  be asserted.
- **Affects:** the scenario outcomes that say a discard, exhaustion or
  quarantine is counted: P2-S10 and P3-S9 (`control_discarded`), P2-S11
  (`guest_park_exhausted`), P3-S5 (`guest_udp_slot_quarantined`), P3-S10
  (`guest_udp_slot_exhausted`, feature-delta L19658). The
  authored `guest_flow::tests` bodies assert the behaviour, not the count.
- **Decision needed:** add the counters to the guest observation, or drop
  "counted" from those scenario outcomes.

### UI-R2-7 — `QuiescenceHolder::as_str` and `FlowCloseCause::as_str` strings are unpinned — LOW

- **Artifact:** feature-delta § *Typed error taxonomy*, QPROJ-3.
- **Gap:** the label strings are not pinned; both methods are scaffolded RED.
  `FlowCloseCause::as_str` is operator-visible: it is the `cause` field of the
  `guest_flow.closed` event (feature-delta L19381), so its strings are part
  of the observable contract, not internal labels.
- **Affects:** the attachment-cut quiescence and owner-flow bodies that assert
  labels.
- **Decision needed:** pin the strings.

### UI-R2-8 — D8-PREFIX-CFG: covering subnet or contained address — LOW — CLOSED

- **Artifact:** feature-delta D8-PREFIX-CFG.
- **Gap:** does an interface address whose subnet covers the guest prefix (for
  example `100.94.0.1/15`) refuse, or only an address inside the prefix? The
  authored tests assume the predicate "some address lies inside the prefix"
  and their generator excludes the covering case.
- **Affects:** P4-S14 (`prefix_boot_acceptance::{a_node_address_inside_the_prefix_refuses_before_any_boot_effect, probe_startup_refuses_a_node_address_iff_one_lies_inside_the_guest_prefix}`).
- **Decision needed:** state which case refuses.

**Resolution — closed by the existing text.** The `probe_startup` contract
(feature-delta L19136-19138) refuses "an address configured on any
root-namespace interface inside the guest prefix". A covering `/15` address
outside the prefix does not refuse, so the authored predicate ("some address
lies inside the prefix") matches. Residual, not discussed by DESIGN: a
covering `/15`'s broadcast route lands inside the prefix.

### UI-R2-9 — Unpinned values used by the scaffolds — LOW

- **Artifact:** feature-delta DG-3, § *Typed error taxonomy*.
- **Gap:** `stale_objects_present`'s error operation (the sim uses
  `CleanupComplement`); the fd-budget `reserve` term; the exact form of the
  owner-cgroup attach-point path.
- **Affects:** `prefix_boot_acceptance::{an_rlimit_nofile_below_the_owner_budget_refuses_before_any_boot_effect, an_owner_outside_the_connect4_attach_path_refuses_before_any_boot_effect}`
  and `SimGuestFlowIo`'s stale-object path.
- **Decision needed:** pin the three values.

### UI-R2-10 — DISTILL-defined crate-private parse errors in `overdrive-init` — LOW (note)

- **Artifact:** feature-delta DG-5 (`SocketCookie`, `CellGeneration`:
  "validating `new` returning `Result`").
- **Gap:** DG-5 names no error type. DISTILL defined crate-private
  `SocketCookieError { Zero, Unparseable }` and
  `CellGenerationError { Unparseable }`. This is binary-internal structure (in
  the crafter's scope per CLAUDE.md), recorded for visibility.
- **Affects:** `guest_flow::tests::newtypes`.
- **Decision needed:** confirm or replace.

### UI-R2-11 — `GuestNetworkOperation::LocalRouteConverge` added before the cut — LOW — CLOSED

- **Artifact:** feature-delta DG-3 (`converge_prefix_route` contract), DG-2
  row 11.
- **Gap:** the variant was added now because DG-3's `converge_prefix_route`
  contract names it, although row 11 replaces the operation set at the cut.
- **Affects:** P4-S12 `a_failed_route_write_refuses_startup_with_the_route_as_it_was`.
- **Decision needed:** confirm `LocalRouteConverge` belongs in the new
  operation set.

**Resolution — closed.** `LocalRouteConverge` is in the taxonomy's operation
list (feature-delta L19999-20004) and in DG-3's `converge_prefix_route`
contract.

### UI-R2-12 — The guest sends `Abort` after `DrainTimeout` — LOW — CLOSED

- **Artifact:** feature-delta § *Flow end*; `owner_flows.qnt` abort step.
- **Gap:** `guest_flow::tests::half_close::a_drain_that_misses_the_deadline_aborts_the_flow`
  assumes the guest sends `Abort` after `DrainTimeout`. This is inferred from
  the `owner_flows` model's abort step; the Flow-end text does not state it.
- **Affects:** P2-S12 guest half.
- **Decision needed:** confirm.

**Resolution — closed.** `owner_flows.qnt:842-843` (`HcFlowError`: "the drain
deadline at s: abort, Abort sent"), the QPROJ action map (feature-delta
L20826-20828) and DG-15 (L22158) give drain-deadline → `Abort` sent. Note for
the architect: the Flow-end prose is editorially inconsistent — L19344 says
only "abort, cause `DrainTimeout`", while L19359-19360 says an abort sends
`Abort`.

### UI-R2-13 — quint 0.32 trace labelling — note (test infrastructure)

`quint run --n-traces` labels the initial state of some traces `step` instead
of `init`, and names variables `<instance>::<module>::<var>`. The drivers
handle both: each maps a `step` action to the initial state (with a comment)
and matches variables by their own name. No decision needed.

### UI-R2-14 — P5-S8 cells that need the prefix `local` route move to P6-S13 — note

The `gateway:53` accept and guest-prefix broadcast reject cells of P5-S8 need
the prefix `local` route that only `converge_shared` writes. A worker-crate
test would have to hand-install it (the production-wiring tell in CLAUDE.md
§ "Build vertical slices"), so they move to the P6-S13 walking skeleton
(attachment cut). No decision needed.

### UI-R2-15 — `SimBeaconIo`'s token projection reads the row-13 READY format — note

`SimBeaconIo::beacon_socket_observation`'s `Option<LaunchToken>` projection
reads the buffered READY's `launch=` field, i.e. the DG-2 row-13 READY format.
A line without a valid token projects `None`, like an empty buffer
(`beacon_io::a_queued_connection_with_nothing_buffered_or_no_valid_token_projects_none`).
No decision needed.

### UI-R2-16 — DG-2 row 14 has no authoring time — HIGH (design text)

- **Artifact:** feature-delta DG-2 (authoring rule), row 14
  (`ServiceBackendsResolve::new(store, frontend, vip_ranges)`).
- **Gap:** row 14 lands in its own new-step 11, before the attachment cut.
  DG-2's authoring rule has only two times: "now" (additive surface) and "at
  the cut" (a replaced signature). A row-14 body names a replaced signature
  but its step lands before the cut, so neither time applies.
- **Affects:** P5-S7 (the VIP branch of the resolve adapter): its
  resolve-adapter bodies have no authoring time.
- **Decision needed:** DESIGN adds "at the start of step 11 for row 14" to
  DG-2's authoring rule. DISTILL then plans P5-S7's resolve-adapter bodies for
  step 11, not for the cut.

**Resolution — closed by DG-2-R2 (user ruling, 2026-10-10).** DG-2 has one
authoring time: every replaced signature (rows 1–18, row 14 included) is
scaffolded in DISTILL in place, with every body and driver over it authored
now, compiled and `#[ignore = "pending DELIVER <step>"]` where its owner is a
RED scaffold. The last column names only the DELIVER step that implements
the row: the attachment-cut step for rows 1–13 and 15–18, step 11 for row
14. P5-S7's resolve-adapter bodies are authored now and ignored pending
step 11.

### UI-R2-17 — `Display` texts of the probe-startup `GuestNetworkError` variants are unpinned — LOW

- **Artifact:** feature-delta § *Typed error taxonomy*.
- **Gap:** the taxonomy gives each variant's meaning, not its text. DISTILL
  wrote the `Display` texts of `GuestPrefixOverlapsNodeAddress`,
  `GuestPrefixOverlapsVipRange`, `FdBudget`, `OwnerCgroupPlacement`,
  `UnframeAttach`, `GuestPrefixSteeringConverge`, `IntakePortBlockConflict`
  and `StackedInterface`.
- **Affects:** any test or operator surface that renders these errors.
- **Decision needed:** DISTILL's texts stand until DESIGN pins them.

### UI-R2-18 — `const fn` where DG-4 / DG-6 write `fn` — LOW

- **Artifact:** feature-delta DG-4 (`BeaconSocket::{from_raw, raw}`), DG-6
  (`VhostVsockCidClaim::new`).
- **Gap:** the scaffolds are `pub const fn` (clippy nursery
  `missing_const_for_fn` under the workspace `-D warnings`); DG-4 and DG-6
  write `pub fn`.
- **Decision needed:** DESIGN records the `const`-ness, or accepts it.

**Resolution — closed by DG-CONST (approved by the user 2026-10-10).**
`VhostVsockCidClaim::new`, `VsockBeaconIo::new` and
`BeaconSocket::{from_raw, raw}` are `pub const fn` (feature-delta DG-4,
DG-6).

### UI-R2-19 — DG-1 withdraws P6-S7 by name but not P6-S8 — LOW (design text)

- **Artifact:** feature-delta DG-1 decision record.
- **Gap:** DG-1 withdraws P6-S7 by name. DISTILL also withdrew P6-S8 with the
  VMM backend probe; its `fd=` obligations moved to the P1-S12 VMM cell and to
  P6-S13. DG-1 does not say so.
- **Decision needed:** the architect adds the P6-S8 line to DG-1.

### UI-R2-20 — CI does not select the source-local quint-connect drivers — MEDIUM (test infrastructure; user decision)

- **Artifact:** `.github/workflows/ci.yml` (the `integration` job's nextest
  selector, near L461).
- **Gap:** the selector does not select the source-local quint-connect
  drivers (the `overdrive-control-plane` lib module
  `guest_network::prefix_boot_conformance`, the `overdrive-init` binary's
  `guest_flow::quint_*`). Once activated they run only in pre-push. Widening
  the CI selector is a user decision; DISTILL has not wired it. Separately, the
  drivers' `#[quint_run]` cost must be measured and bounded (`max_samples` /
  `max_steps`) before activation, because the mutation lane reruns them per
  mutant.
- **Affects:** the four authored drivers and the seven at the cut.
- **Decision needed:** the user decides whether CI selects the drivers; the
  activating DELIVER step measures and bounds their cost.

### UI-R2-21 — `quint` on the metal host — note

`quint` on the native metal host is DG-14 G-PL-3, owned by DELIVER new-step 1
(provisioning). `run-native-metal.sh` should fail closed when `quint` is
missing, so P4-S19 never reports a result without its oracle. No DESIGN
decision is needed.

### UI-R2-22 — The guest drivers' quiescence signal unwinds through the owner — MEDIUM (test support; DESIGN choice)

- **Artifact:** DG-5 `SimGuestOwnerIo` contract ("a call with no event
  released and no deadline passed is a driver defect (it panics rather than
  block a single-threaded driver)"); `crates/overdrive-init/src/guest_flow/test_support.rs`.
- **Gap:** only `next_event` learns the owner's earliest armed deadline, so the
  only exact "the owner has nothing to do" signal a single-threaded driver can
  get is the pinned sim panic. `settle()` catches exactly that payload
  (`SimWouldBlock`) and re-raises every other panic. The owner frame it unwinds
  is at its blocking wait (the QPROJ-5 consistency point); an owner that held a
  side effect only in a local before its wait would lose it, which the sim's
  effect log and socket table then contradict (a visible failure, never a
  silent one). A prediction from the last waited deadline was tried and
  rejected: it misses a deadline armed while handling the last input, and it
  would have to inject `Err` into `next_event`, which the contract makes fatal
  (`GuestOwnerFatal::EventWait`).
- **Affects:** the three guest drivers and every guest-owner acceptance test.
- **Decision needed:** either accept the pinned-panic signal, or DESIGN pins a
  non-unwinding one (an owner read of its next armed deadline, or a blocking
  sim with the owner on its own thread and an observation hand-off).

**Resolution — closed by DG-5-Q (approved by the user 2026-10-10).**
`GuestFlowOwner::next_deadline(&self) -> Option<std::time::Instant>`
(`pub(crate)`, compiled in every build; the value `dispatch_next` passes to
`next_event`, drain re-check deadlines included) plus the sim's read of a
released, undelivered event. The driver calls `dispatch_next` only while
one of them is ready; otherwise the owner is quiescent. No panic is raised
or caught; the sim's `next_event` with nothing released and no deadline
passed is `unreachable!` (a driver defect), and `settle()`'s catch of
`SimWouldBlock` is removed.
