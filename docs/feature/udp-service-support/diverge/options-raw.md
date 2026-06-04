# Options (raw, unfiltered, NO evaluation) — udp-service-support DIVERGE

> **Separation principle.** Generation and evaluation are separate phases.
> This file contains ZERO evaluative language — no "best", "smallest",
> "preferred", "wins", "too big". Blast-radius and reconciliation facts
> are recorded as *neutral descriptions of mechanism*, not judgments.
> Scoring happens only in `taste-evaluation.md`.

## HMW framing

The raw decision is solution-shaped ("how to thread proto through
`update_service`"). Reframed to open the option space:

> **How might we let the production dataplane install REVERSE_NAT entries
> that match every service's declared L4 protocol, so the simulated and
> real adapters provably install the identical `(ip, port, proto) → vip`
> set?**

This HMW does not embed "aggregate" or "positional arg" — the answer
could be a new type, a new positional parameter, a re-keyed map, or a
per-listener fan-out.

## SCAMPER lenses

**S — Substitute the carrier of proto.** Replace the *implicit* proto
(today hardcoded `Tcp` in Ebpf Step 4b, `[Tcp,Udp]` in Sim) with an
*explicit* positional `proto: Proto` argument on `update_service`. The
mechanism that changes: the trait signature gains one scalar argument;
Step 4b reads that argument instead of a literal. → **Option 1.**

**C — Combine proto with vip+port+backends into one value.** Merge the
four pieces a service-update needs into a single typed descriptor passed
as one argument. The mechanism: a new struct (`ServiceDescriptor`)
becomes the sole parameter; both adapters destructure it. → **Option 2.**

**A — Adapt the Kubernetes `Vec<ServicePort>` shape.** Borrow the
per-listener model: `update_service` (or the descriptor) carries a
*per-listener* shape so one VIP with TCP 8080 + UDP 8081 is expressed as
two listener entries. The mechanism: the unit of update is the listener,
not the service. → **Option 3.**

**M — Magnify the existing `BackendKey` symmetry.** The reverse side
already keys by `BackendKey { ip, port, proto }`. Amplify that: make the
forward-path update carry the *same* `(port, proto)` shape so the
forward descriptor and the reverse key are structural twins, and the
lockstep is a pure set-equality over one shared key type. The mechanism:
the descriptor's `(port, proto)` field IS the projection that derives the
`BackendKey`. → folds into **Option 2** (the aggregate is where this twin
lives); recorded as a property, not a separate option.

**P — Put the proto carrier to other use (the Action envelope).** The
`Action::DataplaneUpdateService` envelope already carries `service_id`
beside `vip`/`backends` (`validate.rs:288`). Put that envelope to further
use: carry `proto` (or a `listener`/`service_id→listener` reference) on
the *Action*, and have Step 4b look up the proto from the action's
`service_id` rather than from a new trait argument. The mechanism: the
trait signature is unchanged; the proto reaches Step 4b through the
action-shim/observation layer keyed by `service_id`. → **Option 4.**

**E — Eliminate the signature change entirely.** Key the REVERSE_NAT map
derivation by `(vip, proto)` where proto is recovered without touching
the `Dataplane` trait surface at all — e.g. the hydrator pre-expands one
`update_service`-equivalent per proto, or the proto is read from an
observation row at install time. The mechanism: `update_service(vip,
backends)` stays byte-identical; proto enters through a *different* call
or a *different* store read. → folds into **Option 4** (both are "no
trait signature change; proto carried elsewhere") — recorded as the
elimination variant of option 4.

**R — Reverse the granularity: descriptor carries ALL listeners.** Invert
"one update per listener" (option 3): a single descriptor carries the
whole `Vec<Listener>` for the VIP, and the *adapter* fans out internally
to one `(ip, port, proto)` set per listener. The mechanism: the trait
takes one rich aggregate per service; the per-listener fan-out lives
inside the adapter, not the caller. → **Option 5.**

## Crazy 8s supplements

**Supplement A — Newtype-only thread (no aggregate, no new positional
scalar).** Keep `update_service(vip, backends)` arity but change `vip`'s
TYPE to a `ServiceFrontend` newtype that wraps `(Ipv4Addr, u16 port,
Proto)` — proto rides inside the *existing* first argument's type rather
than as a new argument or a new whole-call aggregate. The mechanism: the
first parameter's newtype is the proto carrier; `backends` stays a
separate argument. → **Option 6.**

**Supplement B — Two methods (split by proto).** Add a sibling
`update_udp_service(vip, backends)` beside the existing
`update_service(vip, backends)` (now implicitly TCP). The mechanism: proto
is encoded in the *method name*; each adapter implements two methods.
→ recorded but folds toward elimination in curation (see below).

## All generated options (pre-curation)

1. Positional `proto: Proto` argument (SCAMPER-S)
2. Typed `ServiceDescriptor` aggregate (SCAMPER-C, +M twin property)
3. Per-listener descriptor — listener is the unit of update (SCAMPER-A)
4. No signature change; proto carried on the Action envelope / observation
   keyed by `service_id` (SCAMPER-P, +E elimination variant)
5. Service aggregate carries `Vec<Listener>`; adapter fans out (SCAMPER-R)
6. `ServiceFrontend` newtype on the existing first arg (Crazy-8s A)
7. Two methods split by proto name (Crazy-8s B)

## Curation to the evaluated set

**Merges / removals (exact-or-variation only):**

- **Option 7 (two methods) → merged into the eliminated set.** It is a
  variation of "proto encoded structurally" but its mechanism (method-name
  encoding) does not scale to SCTP/QUIC and is a strict variation of
  option 6's "proto rides structurally" with worse extension shape; it
  shares option 6's assumption (proto is known at the trait boundary) and
  cost profile is dominated by option 6. Removed as a variation, not
  carried as a distinct option.
- **SCAMPER-M (BackendKey twin)** is a *property* of option 2, not a
  separate mechanism — recorded inside option 2.
- **SCAMPER-E (eliminate signature change)** shares option 4's mechanism
  and assumption ("no trait change; proto carried elsewhere") — merged
  into option 4 as its elimination variant.

**Curated evaluated set (6 → 5 after merges; all structurally distinct):**

The diversity test is applied below. Five options survive curation as
structurally distinct; option 7 is the only exact-variation removal.

---

### Option 1: Minimal positional proto

**Core idea:** `update_service(vip, proto, backends)` — one new scalar
argument; Step 4b reads `proto` instead of a hardcoded literal.
**Key mechanism:** a single `Proto` value threaded positionally; the
adapter derives `BackendKey { ip, port, proto }` from the new argument.
**Key assumption:** a service has exactly ONE protocol per
`update_service` call (multi-listener is handled by the hydrator emitting
one call per listener upstream).
**SCAMPER origin:** Substitute.
**Closest competitor:** loxilb / GLB thin per-rule surface (proto rides
the rule, no aggregate object).
**Mechanism facts (neutral):**
- Call sites that change: the trait (`dataplane.rs:101`), `EbpfDataplane`
  impl, `SimDataplane` impl, the action-shim dispatch that calls
  `update_service`, and the `ReverseNatLockstep` invariant's two
  `update_service` call sites.
- `service_id` / `ServiceVip` reconciliation: NOT re-absorbed — they stay
  on the `Action` envelope where they already live (`validate.rs:288`);
  the trait gains `proto` only, `vip` stays raw `Ipv4Addr`.
- The Sim's `reverse_nat_keys_for` `[Tcp, Udp]` hardcode is narrowed to
  the single passed `proto`.

### Option 2: Typed `ServiceDescriptor` aggregate

**Core idea:** `update_service(descriptor)` where `descriptor` carries
`(vip, port, proto, backends)` as one typed value; both adapters
destructure it.
**Key mechanism:** a new struct is the sole parameter; the descriptor's
`(port, proto)` field is the projection that derives the forward set and
is the structural twin of the existing `BackendKey { ip, port, proto }`
(SCAMPER-M property).
**Key assumption:** a service-update is naturally described as one
aggregate value; the descriptor is the single source of `(vip,port,proto)`
(C2).
**SCAMPER origin:** Combine (+ Magnify twin property).
**Closest competitor:** Katran `VipKey { address, port, proto }` /
Cilium `L4Addr`.
**Mechanism facts (neutral):**
- Call sites that change: the trait, both adapters, the action-shim
  dispatch (now constructs a `ServiceDescriptor` from the Action's
  fields), and the lockstep invariant (now builds a descriptor).
- `service_id` / `ServiceVip` reconciliation: a DESIGN-level choice the
  descriptor forces — it may (a) re-absorb `service_id` + `ServiceVip`
  into the descriptor (re-converging toward locked-A's typed surface),
  (b) carry `ServiceVip` but leave `service_id` on the Action, or (c)
  carry raw `Ipv4Addr` + `port` + `proto` and leave `service_id` on the
  Action. The descriptor MUST resolve this explicitly (review B2).
- This is `feature-delta.md` D1 / Q-Sig option B — the user's standing
  preference.

### Option 3: Per-listener descriptor (listener is the unit of update)

**Core idea:** `update_service` takes a per-listener shape, so a VIP with
TCP 8080 + UDP 8081 is two listener-update calls/entries — each carrying
its own `(port, proto, backends)`.
**Key mechanism:** the unit of update is the `(VIP, port, proto)`
listener tuple (which is already the SERVICE_MAP outer key per phase-2
architecture.md §5 Drift-3), not the whole service.
**Key assumption:** multi-listener is a first-class shape; the dataplane
is told about listeners, and the hydrator/caller does not have to
collapse them.
**SCAMPER origin:** Adapt (Kubernetes `Vec<ServicePort>`).
**Closest competitor:** kube-proxy `ServicePort` per-port entries.
**Mechanism facts (neutral):**
- Call sites that change: the trait, both adapters, the hydrator (US-05
  emission shape), the action-shim, the lockstep invariant.
- `service_id` / `ServiceVip` reconciliation: the listener tuple is
  `(VIP, port, proto)`; `service_id` may key the listener group or stay
  on the Action. The SERVICE_MAP outer key `(VIP, port)` aligns with this
  granularity.
- This is the granularity US-05 (multi-listener) needs; options 1 and 2
  handle US-05 by upstream hydrator fan-out instead.

### Option 4: No signature change; proto carried on Action / observation by `service_id`

**Core idea:** `update_service(vip, backends)` is byte-identical; Step 4b
recovers the proto from the `Action`'s `service_id` (via a
listener/observation lookup) rather than a new trait argument. The
elimination variant: the hydrator pre-expands the call per-proto so the
adapter never needs proto in its signature.
**Key mechanism:** proto reaches Step 4b through the existing
`service_id`-keyed action/observation layer, OR through caller-side
pre-expansion — the `Dataplane` trait surface is untouched.
**Key assumption:** the proto is recoverable at install time from a
`service_id`-keyed lookup the adapter already has access to, OR the caller
can fan out per-proto without the adapter knowing proto at all.
**SCAMPER origin:** Put-to-other-use (+ Eliminate variant).
**Closest competitor:** GLB split-tier (proto classified outside the
update surface).
**Mechanism facts (neutral):**
- Call sites that change: the trait is UNCHANGED; the change lands in
  Step 4b's proto-recovery logic (a `service_id`→listener lookup) and/or
  the hydrator's emission. The lockstep invariant changes to assert over
  whatever the recovery path produces.
- `service_id` / `ServiceVip` reconciliation: `service_id` becomes
  LOAD-BEARING for proto recovery (it is the lookup key); `ServiceVip`
  stays as-is.
- Constraint interaction: C5 ("production not shaped by simulation") and
  C2 ("single source of `(vip,port,proto)`, no scattered reconstruction")
  bear directly on whether the recovery-by-lookup shape reconstructs the
  triple from scattered state.

### Option 5: Service aggregate carries `Vec<Listener>`; adapter fans out internally

**Core idea:** `update_service(descriptor)` where the descriptor carries
the whole `Vec<Listener>` for the VIP; the *adapter* iterates listeners
and installs one `(ip, port, proto)` set per listener.
**Key mechanism:** one rich aggregate per service; the per-listener
fan-out lives inside each adapter (Sim and Ebpf both iterate the Vec).
**Key assumption:** the dataplane should own the per-listener fan-out, so
the hydrator emits one call per service (not per listener).
**SCAMPER origin:** Reverse (granularity inversion of option 3).
**Closest competitor:** Cilium `lb4_service` rich aggregate carrying
backend/port structure.
**Mechanism facts (neutral):**
- Call sites that change: the trait, both adapters (each grows an internal
  listener loop), the hydrator (emits one call per service), the
  action-shim, the lockstep invariant.
- `service_id` / `ServiceVip` reconciliation: the aggregate is the
  natural home for `service_id` + `ServiceVip` + `Vec<Listener>` — it can
  re-absorb both newtypes (closest to locked-A's typed intent, extended
  to multi-listener).
- The per-listener loop inside the adapter is shared logic that both Sim
  and Ebpf must implement identically (C5 interaction: the loop is
  production logic, mirrored by the sim, not a sim-only arm).

## Diversity test (3-point: mechanism / assumption / cost)

| Option | Different mechanism? | Different assumption? | Different cost profile? |
|---|---|---|---|
| 1 Positional proto | Scalar arg threaded positionally | One proto per call; multi-listener handled upstream | Smallest set of changed call sites; no new type |
| 2 Typed aggregate | New struct as sole param | Update is one aggregate value; descriptor is SSOT | Every call site constructs the struct; one new type |
| 3 Per-listener | Listener tuple is the unit | Multi-listener is first-class at the trait | Trait + hydrator granularity change; emission reshape |
| 4 No-sig / Action-carried | Proto recovered via `service_id` lookup | Proto recoverable at install from existing state | Trait untouched; logic moves into Step 4b recovery + lockstep retarget |
| 5 Vec<Listener> aggregate | Adapter-internal fan-out loop | Dataplane owns per-listener fan-out | Richest aggregate; new shared loop in both adapters |

All five answer YES on all three axes — no two share mechanism +
assumption + cost. **Diversity test: PASS for 5 options.**

## Eliminated (exact variation)

- **Option 7 (two methods split by proto name):** strict variation of
  option 6's "proto rides structurally" with a worse extension shape
  (method-per-proto does not scale to SCTP/QUIC); merged out as a
  variation. *(Option 6 — `ServiceFrontend` newtype on the first arg — is
  RETAINED and carried into evaluation as a sixth option below, because
  its mechanism (proto inside the existing argument's type, NO new
  argument and NO whole-call aggregate) is structurally distinct from both
  1 and 2.)*

### Option 6: `ServiceFrontend` newtype on the existing first argument

**Core idea:** keep `update_service(frontend, backends)` arity; change the
first argument's TYPE from raw `Ipv4Addr` to a `ServiceFrontend` newtype
wrapping `(Ipv4Addr, u16 port, Proto)`. Proto rides inside the existing
first argument's type — no new positional argument, no whole-call
aggregate, `backends` stays a separate argument.
**Key mechanism:** the first parameter's newtype is the proto carrier; it
is the forward-path twin of `BackendKey { ip, port, proto }` minus the
`backends` field.
**Key assumption:** the frontend identity `(vip, port, proto)` is one
typed value, but the backend set is conceptually separate from the
frontend (matching the existing two-argument split).
**SCAMPER origin:** Crazy-8s A.
**Closest competitor:** Katran `VipKey` (which is exactly
`(address, port, proto)` with backends held separately).
**Mechanism facts (neutral):**
- Call sites that change: the trait, both adapters, the action-shim
  (constructs `ServiceFrontend`), the lockstep invariant. `backends`
  argument is unchanged at every site.
- `service_id` / `ServiceVip` reconciliation: `ServiceFrontend` can BE the
  `ServiceVip`-carrying type (re-absorbing `ServiceVip`) while
  `service_id` stays on the Action; or it can wrap raw `Ipv4Addr`. It is a
  narrower re-absorption than option 2's full descriptor.

**Diversity re-check for option 6:** mechanism (newtype on existing arg,
backends separate) differs from option 1 (new scalar arg) and option 2
(whole-call aggregate); assumption (frontend identity typed, backends
separate) differs from both; cost (one new newtype, `backends` site
unchanged) differs. **PASS — 6 structurally distinct options total.**

## Final evaluated set: Options 1, 2, 3, 4, 5, 6 (six, diversity-PASS)
