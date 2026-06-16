# Evolution — transparent-mtls-host-socket (GH #26 · J-SEC-003 · ADR-0069 / ADR-0070)

**Finalized:** 2026-06-16 · **Wave arc:** DISCUSS → DESIGN → DELIVER (no
DIVERGE; mechanism settled empirically by 6 committed Tier-3 spikes) ·
**Branch:** `transparent-mtls-host-socket` · **Architect:** Apex
(nw-platform-architect)

---

## Feature summary

Kernel-mediated **transparent mTLS** for host-socket mesh workloads — the
on-the-wire ENFORCEMENT peer that finally **encrypts the wire** with the
workload's own SVID. It completes the mint (#28) → hold/read (#35, `IdentityRead`)
→ **enforce (#26)** identity chain. The workload holds **nothing** (no cert, no
key, identity-unaware); a node-agent-owned **agent-light L4 proxy** (ADR-0069)
terminates/originates TLS 1.3 on its own peer-facing leg and hands steady state
to the kernel.

Mechanism, both directions:

- **Outbound:** `cgroup_connect4` BPF rewrite → agent leg-F → rustls TLS 1.3
  **client** handshake presenting the held SVID → kTLS arm on leg B → forward is
  an agent-light `read → write_all` **copy** into kTLS-TX (a `splice` *into*
  kTLS-TX loses records — the D-MTLS-13 `MSG_DONTWAIT` loss class), return is an
  agent-light **zero-copy `splice`** out of a plain kTLS-RX leg.
- **Inbound:** nft-TPROXY + `IP_TRANSPARENT` listener → `getsockname` orig-dst →
  server-mTLS (`WebPkiClientVerifier` REQUIRE+VERIFY) → kTLS-RX arm → zero-copy
  `splice` deliver of byte-exact plaintext to the identity-unaware server.

Composition: the `run_server` root constructs **and `probe()`s** `MtlsDataplane`
+ `HostMtlsEnforcement` **after** `IdentityMgr` (fail-closed — a failed probe
emits `health.startup.refused` and the node refuses to boot rather than degrade
to cleartext). A worker-owned **`MtlsInterceptWorker`** lifecycle component
(mechanism (B), mirroring `ProbeRunner`; `ExecDriver::new` unchanged) does
per-alloc attach/enforce/detach. Supervision is **per-connection self-teardown**
(kernel `TCP_USER_TIMEOUT`/keepalive (C) + the SD-2 port-owned pump
self-tearing-down on EOF/error (B)) — **no central supervisor** (`MtlsSupervisor`
deleted, ADR-0070 / D-MTLS-16).

**Pinned contract (never widened):** the `MtlsEnforcement` port stayed exactly
4 methods — `probe / enforce / liveness / teardown`. No `intercept()` method, no
`mtls/intercept.rs` adapter file, no new `AllocationSpec` field. `enforce`
dispatches on `Direction`; intercept-install + leg-acquire is the **worker
composition-root role**, not adapter API.

## Business context

Vision principles **2** ("mTLS is in-kernel and undisableable") and **3**
("every packet carries cryptographic workload identity") were *aspirational
until an enforcer shipped* — identity was mintable and held, but a packet
capture would still have shown cleartext. This feature makes them
**operationally true on the wire**, provable by `tcpdump`. It is a foundation
security primitive (D1) — no operator CLI verb (encryption is automatic;
`overdrive deploy <SPEC>` is the only workload verb). The honest observables are
TEST-tier: wire capture, `ss -tie` ULP state, fail-closed negatives, agent-light
`strace`.

**Scope discipline:** v1 is **process/exec only**, **bidirectional**, **single
node**, **chain-to-bundle authn only**. Out of scope, issue-tracked: guest-stack
intercept adapter (#222), intended-peer SPIFFE pinning (#178) + VIP path (#61),
in-place rekey (#229), cert-rotation workflow (#40), revocation (Phase 5),
operator-tunable limits (#230), restart-survival / 1-socket density (#231,
the accepted proxy trade), kernel-invisible progress-stall watchdog (#232),
inbound-TPROXY shared-routing Bar-2 reconciler (#234).

## Key decisions

| # | Decision | Rationale |
|---|---|---|
| ADR-0069 | ONE universal agent-light L4 proxy for all workload kinds, bidirectional | Collapsed the prior "one identity, two mechanisms" framing; folded #222 from a separate mechanism into a staged adapter of the one proxy. Settled empirically (verdict: proxy, lossless, no kernel patch, no race window) on the pinned 6.18 kernel by 6 committed Tier-3 spikes. |
| D-MTLS-13 | Forward = `read → write_all` COPY into kTLS-TX; the sockmap egress-redirect is RETIRED | A `splice` into kTLS-TX (and the sockmap redirect) loses records under `MSG_DONTWAIT`. Eliminating the BPF forward program also removed the Tier-4 verifier-budget baseline the original step needed. |
| D-MTLS-14 / SD-1(a) | Intercept-install + leg-acquire is the WORKER's role (`mtls_intercept.rs` free functions), NOT adapter API | The 4-method `MtlsEnforcement` contract has no home for an `intercept()` method. Crafter correctly STOPPED-and-surfaced rather than invent surface (twice — 02-01, 05-01). Resolved by re-homing to the worker; the former 02-01 step was folded. |
| D-MTLS-15 | Intercept INPUT provenance pinned | needs-intercept = `DriverType::Exec` (derived, no new spec field); legs = ephemeral `127.0.0.1:0`; outbound peer = `MTLS_REDIRECT_DEST[real_peer]`; inbound orig-dst→real-listener DEFERRED to #178 (AC5 is the declared-mesh-peer OUTBOUND gate). **Post-finalize (D-MTLS-19, commit `ce2671b5`): the inbound nft-TPROXY *rule install* is also #178-deferred in production — see the post-finalize correction below.** |
| D-MTLS-17 | Phase 05 superseded → decomposed into Phase 06 (3 steps) | 05-01 silently assumed an unbuilt production mTLS dataplane-integration layer (the loader/attach surface existed only as test glue). Decomposed: 06-01 (MtlsDataplane outbound BPF), 06-02 (worker intercept free fns + inbound TPROXY), 06-03 (composition-root activation + e2e). |
| ADR-0070 / D-MTLS-16 | Per-connection self-teardown ((C)+(B)); DELETE the central `MtlsSupervisor` | A central liveness loop cannot see kernel-invisible stalls and duplicates what `TCP_USER_TIMEOUT` + a self-tearing pump already provide. Deleted production + tests in the same commit (deletion discipline — no gate/salvage/stub); `derive_liveness` + `PumpLiveness` retained as the (B) verdict + #232 hook. |
| MTLS_LEG_S_DIAL_MARK hoist | const hoisted to `overdrive-core` | `overdrive-worker` had no dep edge to `overdrive-dataplane` (adding it drags the aya/BPF chain). Hoisting the shared const to core lets both adapters read it without a circular/heavy edge. |

## Steps completed (9 of 9, all COMMIT/PASS)

DES integrity: `des-verify-integrity … exits 0` — all 9 steps have complete traces.

| Step | What landed |
|---|---|
| 01-01 | Composed bidirectional proxy walking skeleton over real netns/veth, NO RST (BLOCKING integration gate). Shipped the adapter + kTLS/splice/handshake primitives + both BPF programs. |
| 02-02 | Agent mutual-TLS handshake presenting the held SVID (client leg B + server leg C, REQUIRE+VERIFY); identity read-port a REQUIRED constructor param. |
| 02-03 | OUTBOUND enforce: kTLS-TX arm, forward `write_all` copy + return zero-copy splice; TLS 1.3 (0x17) on the wire, `ss -tie` ULP. |
| 03-01 | INBOUND enforce: orig-dst → server-mTLS → kTLS-RX → agent-light splice-to-server, byte-exact plaintext; client leg ciphertext-only. |
| 04-01 | Guardrails: fail-closed cause-distinct (both directions), F4/F7 concrete-value limits, F6 stall→teardown, F5 negatives, honest F1 authn boundary. |
| 06-01 | Production OUTBOUND BPF integration: `MtlsDataplane` (second `EbpfLoader`, justified by aya 0.13.x single-owner model), per-alloc cgroup attach, typed `MTLS_REDIRECT_DEST` programming, `MtlsCgroupLink` RAII. |
| 06-02 | Worker per-alloc intercept-install + leg-acquire (D-MTLS-14 free fns) + inbound nft-TPROXY + `TproxyInterceptGuard`. |
| 06-03 | Composition-root activation (construct+probe after `IdentityMgr`, fail-closed); `MtlsInterceptWorker` per-alloc lifecycle; (C)+(B) supervision; `MtlsSupervisor` deleted; end-to-end declared-peer deploy gate. |

> **Note on the stale 06-03 checkpoint:** the execution-log's last entry records
> 06-03 GREEN as `SKIPPED / CHECKPOINT_PENDING` claiming criteria[4]
> (peer-vanish self-teardown) was "NOT deterministically observable at Tier-3."
> That note is **superseded and FALSE as of finalize.** Commit `fa7aa635`
> ("reap idle outbound connections on peer transport death") landed afterward and
> fixed the real root cause — the (B) self-teardown trigger was installed on the
> *primary forward pump only*, not the aux return pump. The peer-vanish Tier-3
> test `outbound_idle_workload_peer_transport_death_self_tears_down_to_gone`
> passes **9/9 deterministically** under Lima. **criteria[4] is DONE; the step is
> complete.**

## Quality gates (all PASS)

**Tier-3 ground truth (under Lima, the canonical real-kernel path):**

- dataplane mtls suite **29/29**; worker mtls/intercept **13/13**; control-plane
  e2e **28/28**; peer-vanish **9/9**.
- The 4 e2e criteria proven with real evidence: AF_PACKET wire oracle shows
  6×`0x17` TLS 1.3 records/direction + **0** plaintext hits (confidentiality);
  fail-closed boot refusal on injected probe fault; real `bpftool cgroup show`
  attach→detach; F5 self-exempt-negative (SO_MARK on the workload's own socket
  still intercepted — the exemption is agent-private cgroup-subtree scoping).

**Phase 3 Refactoring (L1–L6):** no changes — the mTLS userspace surface was
already clean (freshly TDD-landed); the conservative pass found no real smells.

**Phase 4 Adversarial review:** APPROVED. Zero testing theater — assertions are
on observable kernel side-effects (wire bytes, `bpftool`, `ss`), not internal
reachability; `--no-capture` confirmed the e2e tests execute their assertions
(no false-green skips).

**Phase 5 Mutation (per-feature ≥80%):** unit-reachable surface PASS —
dataplane `limits`+`supervision` predicates 21/21=**100%**; core
`EnforcedConnectionId` newtype + `mtls_mark` 2/2=**100%**; worker
`mtls_intercept` parsers **91.4%** (hardened from 87.9% by extracting the
`ip_rule` fwmark predicate into a pure unit-tested fn, `b269c9e6`);
control-plane mtls boot 2/2=**100%**. The real-kernel I/O glue
(kTLS/splice/connect/probe/nft) is out of mutation scope per
`.claude/rules/testing.md` ("not for real-kernel integration") and is
Tier-3-covered.

## Lessons learned

1. **Feature tests validated only in filtered subsets hid a full-suite
   concurrency race.** Three real-kernel nextest test-groups (`mtls-shared`,
   `bpf-artifact-shared`, `cgroup-workload-shared`) shared `overdrive_bpf.o`, the
   `/sys/fs/bpf/overdrive/` pin namespace, and the cgroup hierarchy but ran
   concurrently — a cross-group race that surfaced ONLY when the full dataplane
   suite ran concurrently (passed serial / mtls-only-filtered). Fixed by
   unifying them into one `host-kernel-shared` serial domain (`fd9eddf7`), which
   also fixed CI's `ci` profile and unblocked the mutation baseline. *Validate
   features against the full concurrent suite, not just a filtered subset.*

2. **STOP-and-surface beats inventing surface — twice.** At 02-01 and again at
   05-01 the crafter hit a design-signature gap (an adapter file / production
   routing API with no home in the frozen contract) and correctly returned a
   blocker rather than improvise public API. Both resolved into the right design
   (worker-role re-home D-MTLS-14; Phase-06 decomposition D-MTLS-17). The cost of
   surfacing a gap is one message; inventing past it is a wrong contract.

3. **A stale checkpoint is not an incomplete step.** The 06-03 criteria[4]
   CHECKPOINT_PENDING was a real Tier-3 observability concern at the time, fixed
   by a later root-cause commit (`fa7aa635`). Finalize verified the corrected
   ground truth (9/9) rather than trusting the trailing log entry.

4. **Decompose when a step silently assumes an unbuilt layer.** 05-01 assumed a
   production loader/attach surface that existed only as test glue; the honest
   move was to supersede it and decompose into three buildable steps (06-01/02/03)
   rather than force a half-built e2e gate.

## Issues encountered (resolved)

- **`MtlsDataplane` dual-load** (a second `EbpfLoader`) initially read as
  contradicting the design's BPF_OBJ_GET reuse contract; resolved as justified by
  aya 0.13.x's single-owner model (the shared SERVICE_MAP HoM is still reused
  by-name via `pinning = ByName`; only the cgroup_connect4_mtls program + the
  plain-HASH `MTLS_REDIRECT_DEST` are owned by the second object).
- **`MTLS_LEG_S_DIAL_MARK` import source** (06-02 compile gap) — resolved by the
  core hoist above.
- **Lima guest `CARGO_TARGET_DIR` empty under sudo** (01-01 early commit block) —
  pre-existing VM env issue, resolved per the workspace Lima target-dir note.

## Links to permanent artifacts

- **ADRs (architect-managed, permanent):**
  `docs/product/architecture/adr-0069-transparent-mtls-universal-agent-light-l4-proxy.md`,
  `docs/product/architecture/adr-0070-mtls-connection-liveness-kernel-timeout-plus-per-connection-self-supervision.md`
- **Architecture / C4:** `docs/architecture/transparent-mtls-host-socket/c4-diagrams.md`
- **UX journey:** `docs/ux/transparent-mtls-host-socket/journey-enforce-transparent-mtls.yaml`
- **Research (already permanent):**
  `docs/research/dataplane/bpf-verifier-complexity-and-perf-optimization-research.md`,
  `docs/research/dataplane/multi-workload-tproxy-interception-resource-model-research.md`
- **eBPF discipline rule:** `.claude/rules/bpf.md`
- **SSOT update:** Component Inventory appended to
  `docs/product/architecture/brief.md` (FINALIZE 2026-06-16).
- **Feature workspace (preserved):** `docs/feature/transparent-mtls-host-socket/`

## Post-finalize corrections

Two security-disposition fixes landed AFTER this doc was finalized (2026-06-16).
Both are recorded here so "what ships" stays honest — the same established pattern
this feature already used for the stale 06-03 checkpoint note above. The
authoritative records live in `design/wave-decisions.md` (D-MTLS-18, D-MTLS-19); the
summary below is the lasting "what does it actually do, and how do we know" pointer.

### Inbound transparent-mTLS data path is NOT live in v1 — inbound TPROXY rule install is #178-DEFERRED (D-MTLS-19, commit `ce2671b5`)

The **Inbound** mechanism described in the Feature summary above
(nft-TPROXY + `IP_TRANSPARENT` listener → `getsockname` orig-dst → server-mTLS) is
**partially deferred in production.** Commit `ce2671b5`
(`fix(mtls): defer inbound TPROXY install to #178`) changed
`MtlsInterceptWorker::start_alloc` to install **no inbound nft-TPROXY rule** in
production (it records `tproxy_guard = None`):

- **What stays production:** the agent's leg-C `IP_TRANSPARENT` transparent listener
  and the inbound accept loop stand up per alloc, exactly as the summary describes.
- **What is #178-deferred:** the nft-TPROXY *rule* that would aim real client traffic
  at that listener. Its match key (`virt` — the server workload's logical loopback
  addr clients dial) has **no v1 production source** (`AllocationSpec` carries no
  listen-addr; the workload binds its own socket) — the SAME east-west
  service-resolution gap ([#178](https://github.com/overdrive-sh/overdrive/issues/178))
  that already defers the outbound peer set (D-MTLS-15). The prior code synthesised
  `virt` from the agent's own ephemeral leg-C port, producing a self-referential rule
  that intercepted no real traffic — inert in production while reading as "installed."
- **The honest v1 state:** inbound transparent mTLS is **not live end-to-end in
  production**; the leg-C scaffold + the `install_inbound_tproxy` free fn exist, and
  the production rule install awaits #178. `install_inbound_tproxy` stays the named
  #178 production-install site, exercised today only by the worker integration tests
  (real, distinct virt) — symmetric with the outbound `program_declared_peer_redirect`
  seam. The OUTBOUND path is proven live e2e (the AC5 declared-peer deploy gate, 0x17
  on the peer wire); the inbound production data path is the tracked v1 gap.

Full RCA: `docs/analysis/root-cause-analysis-inbound-tproxy-virt-intercepts-no-traffic.md`.

### Per-alloc intercept-install is fail-closed (D-MTLS-18, commit `5d7fbae0`)

A separate post-finalize fix (`5d7fbae0`, `fix(mtls): fail closed when
transparent-mTLS intercept install fails`) closed a fail-OPEN gap: an alloc whose
per-alloc intercept could not install was left `Running` with cleartext.
`MtlsInterceptWorker::start_alloc` now returns `Result<(), MtlsInterceptInstallError>`
and the action-shim drives the alloc to terminal `Failed` (typed
`TransitionReason::MtlsInterceptInstallFailed`) on any install-step failure — the
per-alloc layer's counterpart to the boot-path "refuse to start, no cleartext
fallback." After the `ce2671b5` defer (D-MTLS-19) the production install path has
**three** fail-closed sites (outbound cgroup attach, leg-F bind, leg-C transparent
listener); the former fourth site (inbound TPROXY rule install) is no longer a
production install step. See D-MTLS-18 in `design/wave-decisions.md` for the full
disposition.
