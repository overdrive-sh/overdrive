# Evolution — transparent-mtls-enrollment (GH #236 · ADR-0071, amends ADR-0069)

**Finalized:** 2026-06-22 · **Wave arc:** DISCUSS → DESIGN → SPIKE → DELIVER ·
**Branch:** `marcus-sa/transparent-mtls-enrollment` · **Architect:** Morgan
(nw-solution-architect) · **Supersedes the OUTBOUND framing of:**
`transparent-mtls-host-socket` (2026-06-16)

---

## Feature summary

The **ENFORCE / interception layer** for east-west transparent mTLS under the
**enrollment / capture-and-resolve** model (GH #236), ratified in **ADR-0071**
(which amends ADR-0069). It replaces the predecessor's outbound
`cgroup/connect4`-rewrite + cross-socket `SO_ORIGINAL_DST` recovery — proven
unworkable on the appliance kernel by the spike — with **one mechanism serving
both directions**: nft-TPROXY + `IP_TRANSPARENT` + `getsockname`.

Mechanism:

- Each exec workload is born into a **per-allocation Linux netns + veth pair**.
  Its outbound `connect()` leaves the netns via the veth and *ingresses* the
  host-side veth, where **nft-TPROXY** at PREROUTING diverts it to the agent's
  **leg-F `IP_TRANSPARENT`** listener; **`getsockname`** then recovers the
  original destination — the active-side mirror of the already-proven inbound
  leg-C half.
- The agent classifies each captured connection per-connection through a new
  **`MtlsResolve`** driven port: `Mesh` → enforce, `NonMesh` → cleartext
  pass-through, `MeshUnreachable` → fail-closed. This retires the per-destination
  `MTLS_REDIRECT_DEST` map and `program_declared_peer_redirect`.

The agent-light kTLS **enforcement substrate** (ADR-0069/0070, the 4-method
`MtlsEnforcement` port `probe`/`enforce`/`liveness`/`teardown`) is reused
**UNCHANGED**. Path A changes only how the worker *obtains*
`Routed::Outbound { peer }` (now `getsockname`, not a declared-peer slot), never
the port contract.

**Pinned scope:** v1 is **process/exec only**, **both directions**, **single
node**, **authn-only** (the workload presents its SVID; intended-peer SVID
*pinning* is #178-deferred). DNS name-layer *integration* (resolv.conf injection)
is in scope; the DNS responder *daemon* is #61.

## Business context

mTLS in Overdrive is **kernel-mediated**: workloads are identity-unaware and
**hold NO SVID material** — they open ordinary sockets; the platform terminates/
originates TLS transparently. This feature is the L4 **agent-light proxy** that
intercepts those ordinary sockets, recovers the real destination, resolves it to
a mesh backend (and, eventually, an expected identity), and runs the kTLS
enforcement. Because Overdrive ships its own appliance OS (ADR-0068), it can give
every workload a per-netns `resolv.conf` and a per-workload routing point with no
in-pod agent — sidecarless, Fly.io-style. It completes the on-the-wire half of
vision principles 2 ("mTLS is in-kernel and undisableable") and 3 ("every packet
carries cryptographic workload identity") for the OUTBOUND path, provable by
`tcpdump` (TLS 1.3 `0x17` records, zero plaintext).

## Key decisions (the D-TME-* log)

| # | Decision | Rationale |
|---|---|---|
| D-TME-1 | Interception = nft-TPROXY + `IP_TRANSPARENT` + `getsockname`, **both directions** (Path A) | Spike-settled; unifies on the proven inbound mechanism. |
| D-TME-2 | v1 moves OFF host-netns ONTO **per-workload netns+veth** (extend `veth_provisioner`) | TPROXY+getsockname needs an agent-controlled per-workload routing point (Cilium topology). |
| D-TME-3 | `cgroup/connect4`-rewrite + `MTLS_REDIRECT_DEST` map + `program_declared_peer_redirect` **RETIRED** | Probe A DOESN'T-WORK on the appliance kernel. |
| D-TME-4 | `accept_outbound_leg` recovers `peer` via `getsockname`; `real_peer` slot deleted | Symmetric with inbound; follows D-TME-1. |
| D-TME-5 | 4-method `MtlsEnforcement` port **UNCHANGED**; `Routed::Outbound { peer }` still the input | ADR-0069/0070 frozen core. |
| D-TME-6 | New **`MtlsResolve`** driven port = the **#178** anti-corruption boundary; v1 `service_backends`-reading host adapter, **fail-closed (not silent)** | Enrollment needs a per-connection resolve consumer; entangling it into the frozen `MtlsEnforcement` is forbidden. |
| D-TME-7 | Egress nft-TPROXY (unvalidated, no Tier-2 backstop) **validated via a thin Tier-3 spike NOW** (`increment-b/`) before DELIVER | Cheapest place to find an ip-rule/route/F5 collision. |
| D-TME-8 | v1 = both directions, **authn-only**; intended-peer pinning (`expected_peer`/`PeerIdentityMismatch`) deferred to **#178** | Resolve port carries `expected_svid` so the pin wires when #178 supplies the join. |
| D-TME-9 | **Name-layer integration (Q5a)**: node-local DNS responder injected into the per-workload netns `resolv.conf` (Fly `fdaa::3` model); responder addr = the **per-netns gateway** (`plan.host_addr`); responder daemon is **#61** | The per-workload netns IS the DNS injection point; gateway addr is collision-free by construction, zero new converge step. |
| D-TME-10 | DNS-return shape = **HEADLESS for v1**: responder returns a `running` `service_backends` addr — that address IS the `orig_dst` `MtlsResolve` recognizes (one source, two readers); **no #167 VIP dependency** | Keeps v1 `MtlsResolve` thin (identity-only, no LB); VIP was REJECTED (adds #167 + ordering hazard). |
| D-TME-11 | Resolve READ MECHANISM = `ServiceBackendsResolve` over an in-RAM, address-keyed, **ownership-aware** reverse index (`addr → {service → Backend}`), built **List-then-Watch + relist-on-`Lagged`** | Cilium `ipcache` precedent; the model was pinned but the read mechanism wasn't. Closes #237 cold-start; the lossy `subscribe_all` was deleted single-cut for `subscribe_all_events()`. |
| D-TME-12 | **Per-allocation `NetSlot(u16)` (`0..=4095`)** model: netns name, both veth names, AND the /30 subnet all derive from one bounded slot — collision-free **by construction**, **NOT a hash of `AllocationId`**; allocator = `NetSlotAllocator` (smallest-free / release) | Pigeonhole: a 253-char id cannot map collision-free into 15-char IFNAMSIZ / 255-char NAME_MAX. Carries the C3-wiring, ExecDriver→netns JOIN, and adopt-on-restart amendments. |
| D-TME-13 | `MtlsInterceptWorker::leg_c_addr(&AllocationId) -> Option<SocketAddrV4>` diagnostic accessor (inbound twin of the observable leg-F port) | Lets a Tier-3 test drive the spawned production inbound `accept_loop` (the inbound nft rule is #178-deferred). Exposes a socket addr only, no identity material. |

**Scope / sequencing decisions** (design-log, not D-TME-numbered):

- **02-05 SUPERSEDED / removed as a step** — its C3-wiring + ExecDriver-join work
  folded into the merged 04-01 (it could not commit GREEN over a red workspace
  caused by the not-yet-deleted cgroup path).
- **04-01 + 04-03 MERGED** into one atomic step, resolving a verified
  `04-01 ⇄ 04-03` dependency cycle (04-01 needed 04-03's `host_veth`; 04-03 needed
  04-01's deletions). The merged step: cgroup→nft-TPROXY swap + C3 netns wiring +
  single-cut deletion of the cgroup/`MtlsDataplane` surface + the
  `mtls_production_activation` e2e, in one commit. 04-03 is retained as a tombstone
  (the detailed C3 reference; no DES entries).
- **02-06 → renumbered 04-04** (adopt-on-restart; depends on the C3 wiring being
  live).

## Spike outcomes

The mechanism was settled empirically before DELIVER (`.claude/rules/spike.md` was
*extracted from* this feature's arc). Probe code lived in gitignored
`spike-scratch/{increment-*}/`, aya-rs Rust (never C), run for real under Lima.

| Probe | What it tested | Verdict | Gate |
|---|---|---|---|
| **Probe A** (`increment-a`, aya-rs) | Recover orig-dst after a `cgroup/connect4` redirect via kernel stash + `cgroup/getsockopt(SO_ORIGINAL_DST)` | **DOESN'T-WORK** | **PIVOT → Path A** |
| **Probe B** (`increment-b`, Rust + `ip`/`nft`) | Path A **egress** nft-TPROXY on a per-workload veth + `getsockname` | **WORKS** | **PROMOTE** |
| **`increment-c`** (Rust + libc/nix) | adopt-on-restart runtime assumptions (SPIKE-A/B/C) | **WORKS** (3/3) | **PROCEED** |
| **`increment-d`** (nft/ip CLI) | nft-rule survival + by-handle sweep across a restart (SPIKE-D) | **WORKS** | §5 required |

- **Probe A** (kernel 7.0.0-22): `getsockopt(SOL_IP, SO_ORIGINAL_DST)` returned
  **`ENOENT`**. Three independent fatal walls: (1) `connect4` fires before the
  ephemeral source port binds (no 4-tuple key); (2) a `connect4` sockaddr rewrite
  is **not a netfilter DNAT** → conntrack has no original tuple; (3)
  `cgroup/getsockopt` fires only for tasks *inside* the cgroup, but the agent runs
  outside it. **Cross-checked against Cilium** (`main @ dac977e678`): zero
  `cgroup/getsockopt` / `SO_ORIGINAL_DST` in tree — its mediating-proxy path is
  **TPROXY + `getsockname`**, independently confirming the pivot.
- **Probe B**: `getsockname` recovered the dialed peer; the **F5 leg-dial `SO_MARK`
  exemption is load-bearing** (without it the agent's own dial loops back into
  leg-F). Surfaced four converge-on-boot prereqs: per-workload netns+veth,
  `ip_forward=1`, `rp_filter` relaxation, leg-dial `SO_MARK`.
- **`increment-c`**: SPIKE-B (code investigation) found `WorkloadLifecycle::reconcile`
  does **NOT** re-drive Running survivors → a dedicated boot adopt pass is
  **mandatory** (drove 04-04's design). SPIKE-A proved a `setsid` + cgroup-scope
  workload survives a SIGKILL of its serve parent, still in its netns.

## Steps completed (16 executed, all COMMIT/PASS · 2 superseded)

| Step | What landed |
|---|---|
| 01-01 | `MtlsResolve` driven port: 3-variant `MtlsResolution::{Mesh(ResolvedBackend),NonMesh,MeshUnreachable}` + 2-field `ResolvedBackend { addr, expected_svid }`. |
| 01-02 | `SimMtlsResolve` scriptable sim adapter (overdrive-sim) + DST controllability. |
| 01-03 | `ServiceBackendsResolve` host adapter — List-then-Watch, ownership-aware `addr → {service → Backend}` index; List-at-probe (closes #237, `25e7acf3`); ownership-aware index (`bf927306`); lossy `subscribe_all` deleted single-cut for `subscribe_all_events()` (`36a79762`). |
| 02-01 | Per-alloc netns+veth derivation + pure default-lane `converge_steps`; the **NetSlot** model (D-TME-12) makes IFNAMSIZ/NAME_MAX overflow unrepresentable + a compile-time IFNAMSIZ guard. |
| 02-02 | Real netns+veth provision execution (Tier-3 Lima) — idempotent `ip netns`/`ip link`/sysctl converge; per-veth `rp_filter` is the load-bearing guard (`3cba64e0`). |
| 02-03 | `resolv.conf` injection into the per-workload netns (D-TME-9); host-side `/etc/netns/<ns>/` reaped only by explicit teardown (`e004e9e7`). |
| 02-04 | Per-host `NetSlotAllocator` (smallest-free / release) + C3 lifecycle wiring. |
| ~~02-05~~ | **SUPERSEDED** — C3-wiring + ExecDriver-join folded into the merged 04-01. |
| 03-01 | `install_outbound_tproxy` — egress nft-TPROXY on the host-side veth, sibling of inbound. |
| 03-02 | `accept_outbound_leg` recovers orig-dst via `getsockname` (symmetric with inbound). |
| 03-03 | Tier-3 egress capture walking proof: workload connect → egress nft-TPROXY → leg-F → getsockname == dialed-dst. |
| 04-01 | **MERGED (absorbs 04-03):** `start_alloc` installs outbound nft-TPROXY + C3 netns wiring + ExecDriver join; **single-cut DELETE** of `cgroup_connect4_mtls`, `MTLS_REDIRECT_DEST`, the `MtlsDataplane` surface + every test defending them. `fail_closed_on_netns_provision` writes a `WorkloadNetnsProvisionFailed` Failed row (`bf60c0d8`). |
| 04-02 | Per-connection resolve consumer in the outbound accept loop (`decide_outbound` 3-arm) — DELETE the declared-peer path; store-`Err` treated fail-closed (APPROVED, zero defects). |
| ~~04-03~~ | **TOMBSTONE** — merged into 04-01; retained as the detailed C3 reference. |
| 04-04 | Adopt-on-restart: `NetSlotAllocator::adopt` + `run_server` boot recovery pass + §5 by-handle nft sweep; `ChainAbsent` typed split (fail-closed, `f021b701`); destructive-GC `NotFound`-vs-genuine-error split (`f57623e9`). |
| 05-01 | Composed bidirectional walking skeleton (Tier-3) — drives production `start_alloc`/`accept_loop` for both halves (replica deleted, `70266e11`); D-TME-13 `leg_c_addr` drives the inbound leg. |
| 05-02 | name→resolve→enforce consistency (Tier-3, DNS stubbed until #61): the **genuine getsockname-recovered** addr fed into `resolve` is the single-source invariant (Oracle 3 reframed honestly, `1b10f115`). |
| 05-03 | Outbound enforce-substrate per-direction asymmetry (Tier-3) — forward `write_all` copy / return `splice`, re-established on Path-A egress; race-free clone-tree TID partition + panic-safe teardown (`7de8e0ac`). |

## Quality gates

- **Tier-3 ground truth (under Lima, kernel 7.0.0-22):** the composed walking
  skeleton (05-01) drives the production composition root both directions; the
  worker integration suite ran **167–169 passed / 0 failed** across the phase-05
  steps. Per-step mutation runs where unit-reachable production logic was touched
  (05-01 11 mutants, 100% kill). Test-only steps (05-02/05-03) carry no per-step
  mutation run per `.claude/rules/testing.md`.
- **Ground-truthing over log-trust.** Because the predecessor's phase-06 (01/02)
  shipped false `COMMIT-EXECUTED-PASS` and vacuous/root-gated-skip assertions,
  **every DELIVER review re-ran the gates rather than trusting the execution log** —
  the discipline that caught most of the issues below (and several reviewer false
  negatives).

## Lessons learned

1. **Spike isolation + Rust-eBPF discipline.** Probe code lives in gitignored
   `spike-scratch/{increment-*}/`, self-contained, **never** in `crates/`; eBPF is
   **aya-rs Rust, never C**. The probes that pivoted (A) and promoted (B) the whole
   feature were exactly this shape. `.claude/rules/spike.md` was extracted from this
   arc.
2. **No Tier-2 backstop for `cgroup_sock_addr`/`cgroup_sockopt` forces Tier-3.**
   `BPF_PROG_TEST_RUN` returns `ENOTSUPP`, so a kernel-interception decision (Probe A)
   can only be settled by a real `connect()` under Lima as root — never a `--no-run`
   / compile gate.
3. **`--no-run` / seam-only gates miss boot-path regressions.** The leg-F plain-bind
   bug shipped green because every test built a *correct transparent* leg-F in-test;
   a step touching the composition root (`start_alloc`/`accept_loop`/`run_server`)
   must actually *run* the production path under Lima, not a hand-rolled replica.
4. **Verify "unproven"/"proven" claims against the actual evidence.** Reviewers
   produced false negatives (01-03) and tests produced over-attributed claims (03-03
   F5, 05-02 Oracle 3, 05-03 forward oracle). Ground-truth the claim — including a
   reviewer's own — before acting on it.
5. **Single-source invariants in Tier-3 tests — feed the genuine recovered value,
   not the test constant.** 05-02's load-bearing oracle worked because it fed the
   *real getsockname output* into `resolve`; the companion that set `recovered = b`
   was tautological.
6. **Size a derived id to its destination grammar's ceiling — a hash is the
   forbidden hand-wave.** The IFNAMSIZ/NAME_MAX overflow arc (02-01 B1/B3) was closed
   only by a bounded `NetSlot` making collisions structurally impossible.
7. **Never absorb a fallible boundary read into a default — especially before a
   destructive action.** The 04-04 `list_chain` swallow (fail-open) and the
   destructive orphan-GC swallow (would `ip netns del` a *live* workload) were the
   same rule-class; the fix is `NotFound`-vs-genuine-error splits that fail closed.
8. **Surface gaps, don't invent API.** The `host_veth` channel (JOIN-6) and the
   `leg_c_addr` accessor (D-TME-13) were both STOPPED-and-surfaced to the architect
   rather than improvised — the correct response to a design that pinned a value
   source but not a channel/signature.

## Issues encountered (resolved)

- **leg-F plain-bind production gap (`3c085e5d`).** Production `start_alloc` bound
  leg-F with a plain `TcpListener::bind("127.0.0.1:0")`; under the non-rewriting
  egress `tproxy to 127.0.0.1:<legF>` divert the kernel delivers orig-dst-addressed
  packets a plain socket can't receive → `ConnectionRefused`. Masked because every
  test built a correct transparent leg-F. Fixed → `make_transparent_listener` + a
  real-traffic Tier-3 guard that REDs on revert.
- **01-03 F2 concurrent-subscription TOCTOU + F-A addr-collision cleartext.** The
  "single held subscription" claim was false under concurrency (row-loss → stale
  index → cleartext); a flat `addr → Backend` index wrongly evicted a shared addr on
  another service's shrink. Resolved by the single-owner drain (`25e7acf3`) +
  ownership-aware index (`bf927306`).
- **02-01 IFNAMSIZ/NAME_MAX overflow arc (3 passes).** veth names overflow
  IFNAMSIZ=15 / netns name overflows NAME_MAX=255 for long alloc ids — the same
  pigeonhole class, with an arithmetically false "≤255" claim. Closed by the NetSlot
  model + a `const _: () = assert!(...)` guard.
- **02-02 vacuous Tier-3 sysctl assertions.** A malformed `/`-separator `rp_filter`
  key always read `None` (always passed); three global asserts were pre-satisfied by
  Lima defaults. Fixed to assert the exact value production writes (`3cba64e0`); the
  reviewer's prescribed `Some(0)` global fix was itself a verified error and refused.
- **04-01 AC14 + AC2 blockers.** The merged C3 half had no acceptance test and its
  "provision-failure → Failed row" sub-claim was unimplemented (bare `?` → Pending
  retry loop); AC2 was a vacuous `bpftool prog show` assertion. Resolved
  (`bf60c0d8`): real `WorkloadNetnsProvisionFailed` Failed-row + `alloc_netns_lifecycle.rs`
  Tier-3 AT; AC2 replaced with a `readelf -S` ELF-section litmus.
- **04-04 two blockers (same rule-class).** §5 `sweep_per_workload_tproxy_rules`
  swallowed every `list_chain` failure into `Ok(0)` (fail-open on a survivor-recovery
  boot); the sibling netns-observe path swallowed fallible reads → could run a
  destructive `ip netns del` on a live workload. Both fixed with `NotFound`-vs-genuine
  splits (`f021b701`); cross-test false-orphan-GC race serialized into the
  `host-kernel-shared` nextest group (`64c05b32`).
- **05-01 unreachable-dataplane-harness blocker + C1 composition substitution.** The
  first pass logged a genuine `BLOCKED_BY_DEPENDENCY` (harness unreachable + crate-home
  decision) rather than a false PASS; the walking skeleton substituted a hand-rolled
  replica for the production composition root (the exact substitution that masked the
  leg-F bug). Resolved by driving production `start_alloc`/`accept_loop` for both
  halves and deleting the replica (`6683708f`/`70266e11`).
- **05-02 Oracle 3 overclaim.** Two independent ("different-fox") passes agreed Oracle
  3 `std::fs::write`s the resolv.conf itself then asserts the round-trip while claiming
  to prove the injection mechanism — reframed honestly (`1b10f115`); Oracles 1+2 (the
  genuine getsockname-recovered addr into `resolve`) were the load-bearing deliverable.
- **05-03 forward-copy attribution race (flaky, 2 passes).** The forward-copy oracle
  was confounded by the netns workload's own plaintext writes; the live-`/proc` TID
  sampler fix then proved flaky (~29% under Lima — a sub-15ms pump races the 15ms
  sampler). Resolved with a race-free clone-tree TID partition + panic-safe teardown
  (`7de8e0ac`).

## Deferrals / residual scope (issue-tracked)

- **DNS responder daemon — #61.** This feature wires only the `resolv.conf`
  injection + the return-shape contract (D-TME-9/10); the daemon that answers
  `<job>.svc.overdrive.local` by reading `service_backends` is a separate build that
  must answer on each per-workload host-veth **gateway** address.
- **Intended-peer / expected-SVID join, and the inbound production `virt` source —
  #178.** v1 is authn-only (`expected_svid: None`). `MtlsResolve` is the #178
  anti-corruption boundary. The INBOUND production nft-TPROXY rule's `virt` match-key
  has no v1 source and stays test-callers-only until #178 (the leg-C listener +
  inbound accept loop ARE production). `leg_c_addr` (D-TME-13) is the read-point #178
  is *expected* (not certain) to consume.
- **Fail-toward-handshake — #236.** The irreducible convergence window where a
  resolve miss could classify a should-be-mesh peer as `NonMesh` (cleartext) is the
  **(a) fail-toward-handshake** v1 SECURITY invariant, whose code lands under #236
  (this feature's own issue).
- **VIP allocator — #167.** Explicitly NOT a v1 dependency (the D-TME-10 headless
  choice avoids it); enters with the multi-node VIP evolution.
- **#237 (resolve-index cold-start) — CLOSED** by D-TME-11's List-at-probe + relist
  (`25e7acf3`).
- **Bar-2 reconciler promotions (pre-existing, tracked):** veth → first-class network
  reconciler #197; cgroup hierarchy #198; XDP attachment lifecycle #199;
  inbound-TPROXY shared routing infra #234. The per-workload netns is the per-alloc
  analogue of #197's observed-state hydration.
- **#26-coupled (not assumed):** whether the workload's mTLS kernel material (kTLS /
  bpffs-pinned) survives a CP restart is a Tier-3-spike question out of scope; the
  04-04 adopt-on-restart recovers only the network slot/netns binding, not the crypto.

## Links to permanent artifacts

- **ADRs (architect-managed, permanent):**
  `docs/product/architecture/adr-0071-transparent-mtls-enrollment-path-a-per-workload-netns-nft-tproxy-both-directions.md`
  (amends `adr-0069-transparent-mtls-universal-agent-light-l4-proxy.md`; liveness
  `adr-0070-mtls-connection-liveness-kernel-timeout-plus-per-connection-self-supervision.md`).
- **Architecture / design SSOT:** `docs/architecture/transparent-mtls-enrollment/feature-delta.md`
  (migrated at finalize).
- **Spike findings (migrated at finalize):**
  `docs/architecture/transparent-mtls-enrollment/spike/` — `findings.md` (Probe A
  PIVOT), `findings-egress-tproxy.md` (Probe B PROMOTE), `findings-adopt-restart.md`
  (adopt-on-restart).
- **Research (already permanent):**
  `docs/research/networking/transparent-mtls-resolve-index-coherence-research.md`,
  `docs/research/networking/stable-service-naming-mtls-research.md`.
- **Spike / discipline rule extracted from this feature:** `.claude/rules/spike.md`.
- **Feature workspace (preserved):** `docs/feature/transparent-mtls-enrollment/`
  (the full DELIVER history — execution log, 15 review files, wave-decisions).
- **Predecessor evolution doc:** `docs/evolution/2026-06-16-transparent-mtls-host-socket.md`.
