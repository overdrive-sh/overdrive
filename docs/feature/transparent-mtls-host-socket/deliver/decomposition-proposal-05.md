# Decomposition proposal — replace the single `05-01` with the production mTLS dataplane-integration layer

**Agent**: Morgan (nw-solution-architect) · **Date**: 2026-06-14 · **Status**:
PROPOSAL (planning doc; the orchestrator materializes `roadmap.json` after user
approval — this doc does NOT edit `roadmap.json` / `.develop-progress.json` /
`execution-log.json`).

Pins the production design in **D-MTLS-17** (`design/wave-decisions.md`). This
doc proposes the concrete step breakdown that replaces the single, scope-hiding
`05-01`. Each step is independently landable with its own **production-observable
Tier-3 boundary** (`bpftool`/`tcpdump`/`ss` against the real production loader,
NOT a test-harness-only assertion).

---

## Why the single `05-01` was dishonest (the layer it concealed)

`05-01` was named "wire `HostMtlsEnforcement` into the production boot path …
productionise the intercept install + leg-acquire." Its ACs (criteria 3–6) and
`implementation_scope` named four production concerns — but every one of them
sat on top of an **entirely unbuilt production dataplane layer** that no AC
named:

1. The production loader **never loads/attaches `cgroup_connect4_mtls`** — only
   the three `*_service` programs (`lib.rs:691–765`). `05-01` AC3 ("the boot
   path … installs the outbound `cgroup_connect4` leg-F redirect") assumed a
   production attach that does not exist.
2. There is **no production `MTLS_REDIRECT_DEST` programming surface** — the
   only writer is the test harness (`mtls_roles.rs::program_redirect_dest`); a
   Grep of `src/` for the map name returns empty. `05-01`'s "the worker programs
   `MTLS_REDIRECT_DEST[peer]`" (D-MTLS-15) assumed a surface that does not exist.
3. `HostMtlsEnforcement` is **never constructed in production**, and cannot be
   built where D-MTLS-16 assumed (the only `IdentityRead`, `IdentityMgr`, is
   built at `lib.rs:1673`, AFTER the `compose_production_driver` point at 1147).

A single step that "wires it in" silently bundled (a) a new BPF loader
integration, (b) a new typed map-programming surface, (c) a composition
resequencing, and (d) the worker per-alloc lifecycle + e2e gate — four
independently-landable layers, each with its own Tier-3 boundary, collapsed into
one "activation" AC. The decomposition below makes each honest.

### Which "done" claims were adapter-mechanism-proven-in-test only

These prior steps are **DONE for the adapter scope** and do NOT re-open — but
their "done" was proven in **Tier-3 adapter/harness tests**, with the
intercept-install + map-programming + program-attach done by **test-harness
glue**, never the production loader:

| Step | What is genuinely done | What was test-harness-only (now D-MTLS-17 production scope) |
|---|---|---|
| `01-01` | Lossless pre-arm capture; `cgroup_connect4_mtls` kernel program + `MTLS_REDIRECT_DEST` map EXIST; inbound `establish`; `dial_leg_s` F5 bypass; 0.5-RTT drain | The program LOAD/ATTACH + map PROGRAMMING were done by `mtls_roles.rs` (test). The composed walking skeleton proved the *mechanism*, not the *production loader*. |
| `02-02` | Agent client+server mutual-TLS handshake presenting the held SVID | Driven by a test `HeldIdentities` `IdentityRead`, not the production `IdentityMgr` wired through the boot path. |
| `02-03` | OUTBOUND enforce: kTLS-TX/RX arm, forward `write_all`, return splice, TLS 1.3 on wire | The `cgroup_connect4_mtls` intercept feeding it was test-harness-attached; no production loader path. |
| `03-01` | INBOUND enforce: orig-dst → server-mTLS → kTLS-RX → splice-to-server | TPROXY install + orig-dst recovery were test-harness (`install_tproxy`); the worker free fns (D-MTLS-14) are un-productionised. |
| `04-01` | Fail-closed cause-distinct, F4/F7 limits, F6 (now (B)) supervision, F5 negatives, authn boundary | Proven against the adapter + DST equivalence harness, not the production boot path. |

**None of 01-01…04-01 needs re-opening.** They are done for the adapter scope;
the new steps ADD the production layer beneath/around them. (`02-01` was already
folded per D-MTLS-14.)

---

## Proposed steps (phase 06 — production dataplane integration)

Numbering continues the existing `NN-MM` scheme; new steps are phase `06`
(phase `05` had the single folded step). Each step's AC carries a real
production-observable Tier-3 boundary.

### `06-01` — Production OUTBOUND BPF integration: load + per-alloc attach `cgroup_connect4_mtls`, typed `MTLS_REDIRECT_DEST` programming

- **scope**: Add the `MtlsDataplane` surface on `overdrive-dataplane`
  (`mtls/dataplane.rs`, D-MTLS-17 item 1): load the shared `overdrive_bpf.o`
  once, recover the `cgroup_connect4_mtls` program handle + the
  `MTLS_REDIRECT_DEST` typed `aya::maps::HashMap` handle, run the program's
  verifier load once, and expose `attach_alloc(alloc_scope)` (per-alloc cgroup
  attach, F5-exempt subtree) + `program_redirect`/`unprogram_redirect` +
  `MtlsCgroupLink` RAII. Productionise the test-only `MtlsDestKey`/`MtlsAddrPort`
  PODs + `program_redirect_dest` glue into `src`.
- **dependencies**: `01-01` (the kernel program + map exist), `02-03` (the
  outbound enforce the attach feeds).
- **acceptance criteria** (production-observable Tier-3):
  - After `MtlsDataplane::load(...)` + `attach_alloc(scope)`, `bpftool cgroup
    show <alloc-scope>` lists a `cgroup_inet4_connect` program named
    `cgroup_connect4_mtls` attached to THAT alloc's `.scope` — and `bpftool
    cgroup show <workloads.slice>` does NOT (proving the F5-exempt per-alloc
    scope, distinct from the service program's global-ancestor attach).
  - After `program_redirect(real_peer, leg_f)`, `bpftool map dump name
    MTLS_REDIRECT_DEST` shows the `(real_peer)→(leg_f)` host-order entry; after
    `unprogram_redirect(real_peer)`, the entry is gone.
  - A cgroup-isolated workload `connect(real_peer)` under the attached scope
    lands on the agent's leg-F listener (the rewrite fires); a `connect` to an
    un-programmed dest passes through unchanged (map MISS → pass).
  - Dropping the `MtlsCgroupLink` detaches the program (`bpftool cgroup show
    <scope>` no longer lists it).
  - Gate: `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane
    --features integration-tests` green for the new test, ACTUALLY EXECUTING.
- **test_file**: `crates/overdrive-dataplane/tests/integration/mtls_dataplane_outbound_install.rs`
- **scenario_name**: `mtls_dataplane_load_attach_per_alloc_program_redirect`
- **files_to_modify**:
  - `crates/overdrive-dataplane/src/mtls/dataplane.rs` (NEW — `MtlsDataplane`, `MtlsCgroupLink`, `MtlsDataplaneError`, the `MtlsDestKey`/`MtlsAddrPort` PODs)
  - `crates/overdrive-dataplane/src/mtls/mod.rs` (`pub mod dataplane;` + re-export)
  - `crates/overdrive-dataplane/tests/integration/mtls_dataplane_outbound_install.rs` (NEW)
- **prior `05-01` AC mapped**: AC3 (outbound `cgroup_connect4` leg-F redirect install).
- **notes**: Tier-3 only (no `BPF_PROG_TEST_RUN` for `cgroup_sock_addr`). Load is
  fold-into-the-existing-single-ELF (NOT a second `EbpfLoader`) — recover from
  the same `aya::Ebpf` shape as `EbpfDataplane`'s `BACKEND_MAP` recovery. The map
  is a plain `BPF_MAP_TYPE_HASH` (native aya, no HoM `pinning = ByName` dance).
  IMPLEMENT TO D-MTLS-17 item 1 verbatim — do NOT add `Dataplane` trait methods.

### `06-02` — Worker per-alloc intercept-install + leg-acquire (productionise the D-MTLS-14 free functions) + inbound TPROXY

- **scope**: Land the D-MTLS-14 composition-root free functions in
  `overdrive-worker/src/mtls_intercept.rs` (`make_transparent_listener`,
  `install_inbound_tproxy` + `TproxyInterceptGuard`, `accept_outbound_leg`,
  `accept_inbound_leg`), productionising the proven `01-01` harness primitives
  (`roles.rs::{make_transparent_listener, getsockname_orig, accept_*}` +
  `mtls_netns_topology.rs::install_tproxy` production half). These produce the
  `InterceptedConnection` for `enforce`; the inbound path rides nft-TPROXY +
  `IP_TRANSPARENT` (no BPF loader change — D-MTLS-17 item 2).
- **dependencies**: `06-01` (the outbound `MtlsDataplane` the worker drives),
  `03-01` (inbound enforce the leg-acquire feeds).
- **acceptance criteria** (production-observable Tier-3):
  - `make_transparent_listener(127.0.0.1:0)` returns a listener whose socket has
    `IP_TRANSPARENT` set (`getsockopt(SOL_IP, IP_TRANSPARENT) == 1`) — proven by
    a real bind on the production code path.
  - `install_inbound_tproxy(virt, agent_port)` installs the nft-TPROXY rule:
    `nft list table ip overdrive-mtls` (or the production table name) shows the
    `tproxy to :<agent_port>` rule + the `ip rule fwmark`/`ip route local …
    table` companions; dropping `TproxyInterceptGuard` removes them (`nft list`
    empty, `ip rule` companion gone).
  - `accept_inbound_leg` on a TPROXY-redirected connection recovers the orig-dst
    via `getsockname` (NOT `SO_ORIGINAL_DST`) and builds an
    `InterceptedConnection { routed: Inbound { orig_dst }, .. }` whose `orig_dst`
    equals the client's intended `virt`.
  - `accept_outbound_leg` on a leg-F-redirected connection builds
    `InterceptedConnection { routed: Outbound { peer }, .. }` with the
    pre-programmed `peer`; the owned leg is handed by value.
  - Gate: `cargo xtask lima run -- cargo nextest run -p overdrive-worker
    --features integration-tests` green, ACTUALLY EXECUTING.
- **test_file**: `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs`
- **scenario_name**: `worker_intercept_install_leg_acquire_outbound_and_inbound`
- **files_to_modify**:
  - `crates/overdrive-worker/src/mtls_intercept.rs` (NEW — the D-MTLS-14 free fns + `TproxyInterceptGuard` + `InterceptError`)
  - `crates/overdrive-worker/src/lib.rs` (`pub mod mtls_intercept;`)
  - `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs` (NEW)
- **prior `05-01` AC mapped**: AC2 (IP_TRANSPARENT listener + getsockname
  orig-dst), AC4 (CAP_NET_ADMIN intercept/listener setup) — the worker-role half
  of `05-01`'s `implementation_scope`.
- **notes**: The harness GAP-3 netns DNAT/masquerade is test-only and does NOT
  productionise (`install_inbound_tproxy` productionises only the
  TPROXY-prerouting + `ip rule`/`ip route` half; the adapter dials orig-dst
  verbatim, #178). IMPLEMENT the D-MTLS-14 signatures verbatim — NOT a `mtls/`
  adapter file, NOT a `MtlsEnforcement` method.

### `06-03` — Composition-root activation: construct + probe `HostMtlsEnforcement` + `MtlsDataplane` AFTER `IdentityMgr`; wire the per-alloc lifecycle; (C)+(B) supervision; delete `MtlsSupervisor`; the end-to-end deploy gate

- **scope**: The external-validity gate. In `run_server`, AFTER `IdentityMgr::new`
  (`lib.rs:1673`, D-MTLS-17 item 3 / decision (3a)): construct `MtlsDataplane::load`
  + `HostMtlsEnforcement::new(identity_as_IdentityRead, MtlsLimits::default())`,
  run `probe()` fail-closed (`health.startup.refused`, mirroring the `:1467`
  `ebpf_dataplane.probe()` precedent), and inject both ports into the worker
  mTLS-intercept component (mechanism α/β per the BLOCKER — orchestrator pins in
  dispatch). Wire the per-alloc lifecycle on `on_alloc_running`/`on_alloc_terminal`
  (D-MTLS-17 item 4): install → enforce → (C) `TCP_USER_TIMEOUT`/keepalive + (B)
  per-connection self-teardown. **Delete the central `MtlsSupervisor` + its tests**
  (ADR-0070/D-MTLS-16 — delete, not refactor). End-to-end gate: a normal exec
  workload deployed via `overdrive deploy <SPEC>` dialing a DECLARED mesh peer
  produces TLS 1.3 (`0x17`) on its peer-facing leg via the real production boot
  path.
- **dependencies**: `06-01`, `06-02`, `04-01` (guardrails the boot path relies on).
- **acceptance criteria** (production-observable Tier-3, through the REAL boot path):
  - `run_server` constructs `HostMtlsEnforcement` + `MtlsDataplane` after
    `IdentityMgr` and runs `probe()` BEFORE serving allocation traffic
    (wire→probe→use); a failed `probe()` emits `health.startup.refused` and the
    node refuses to boot (does NOT degrade to cleartext) — asserted by a fault-
    injected probe failure (mirroring the `ebpf_dataplane` `set_probe_fault` seam).
  - **END-TO-END (the load-bearing gate)**: an exec workload deployed via
    `overdrive deploy <SPEC>` that dials a DECLARED mesh peer produces TLS 1.3
    records (`tcpdump 0x17`) on its peer-facing leg with zero payload cleartext,
    and the peer reads byte-exact plaintext — proven by a driving-port Tier-3
    test that ACTUALLY RUNS under Lima (`cargo xtask lima run -- …`), NOT
    `--no-run`. (AC5 is the ratified declared-mesh-peer OUTBOUND gate, D-MTLS-15;
    arbitrary-peer auto-intercept is #178/#61-deferred — the deploy fixture names
    the destination.)
  - `bpftool cgroup show` on the deployed alloc's `.scope` shows
    `cgroup_connect4_mtls` attached via the PRODUCTION boot path (not a test
    harness); on `on_alloc_terminal` the link is detached and the
    `MTLS_REDIRECT_DEST` entry + the nft-TPROXY rule are removed.
  - F5 runtime self-exempt negative: a workload that sets the bypass on its OWN
    socket is STILL intercepted (the exemption is agent-private cgroup-subtree
    scoping; unreachable from the workload) — proven against the production
    intercept-install this step adds (per `05-01`'s original notes, this negative
    requires the production attach, so it lands HERE).
  - (C)+(B) supervision: a connection whose peer vanishes is reaped by the kernel
    `TCP_USER_TIMEOUT`/keepalive → the per-connection task self-tears-down →
    `liveness` reports `Gone`, no fd/kTLS leak; NO central `supervise_tick` runs
    (`grep` proves `MtlsSupervisor` is deleted).
  - Gate: `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane
    --features integration-tests` green for the e2e test, ACTUALLY EXECUTING.
- **test_file**: `crates/overdrive-control-plane/tests/integration/mtls_production_activation.rs`
- **scenario_name**: `deployed_exec_workload_declared_peer_leg_carries_tls13_via_production_boot_path`
- **files_to_modify**:
  - `crates/overdrive-control-plane/src/lib.rs` (`run_server`: post-`IdentityMgr` construct + probe `HostMtlsEnforcement` + `MtlsDataplane`, fail-closed; resequence per (3a); inject into the worker component)
  - `crates/overdrive-worker/src/` (the `MtlsInterceptWorker` lifecycle component (β) OR the `ExecDriver::new` mTLS param (α) — orchestrator pins; the `on_alloc_running`/`on_alloc_terminal` per-alloc install→enforce→teardown + per-alloc bookkeeping)
  - `crates/overdrive-worker/src/mtls_supervisor.rs` (DELETE — ADR-0070/D-MTLS-16)
  - `crates/overdrive-worker/src/lib.rs` (remove `pub mod mtls_supervisor;`; add the intercept-worker module if (β))
  - `crates/overdrive-worker/tests/acceptance/mtls_supervisor_teardown_on_stall.rs` (DELETE — with the production code, same commit)
  - `crates/overdrive-control-plane/tests/integration/mtls_production_activation.rs` (NEW — the e2e deploy gate)
- **prior `05-01` AC mapped**: AC1 (construct + probe at boot), AC2-residual
  (probe fail-closed `health.startup.refused`), AC5 (the e2e deploy gate — the
  load-bearing one), AC6 (workload holds nothing; `overdrive deploy` verb). The
  D-MTLS-16 `MtlsSupervisor` deletion + per-alloc bookkeeping also land here.
- **notes**: Tier-3 end-to-end through the REAL boot path. A `--no-run` gate is
  green even when every fixture refuses at boot (the built-in-ca 02-02 cold-boot
  regression class) — this MUST actually run under Lima. IMPLEMENT TO THE
  ACCEPTED CONTRACT: `probe`/`enforce`/`liveness`/`teardown` only; do NOT add a
  `mtls/intercept.rs` adapter file or an `intercept()` trait method. The
  worker-injection mechanism (α vs β) is the pinned BLOCKER — the orchestrator
  MUST resolve it in the dispatch (recommend β); the crafter must NOT improvise a
  builder/`Option` to dodge the mandatory-port-param rule.

---

## Dependency graph (proposed)

```
01-01 ──┬─> 02-02 ──┬─> 02-03 ──┐
        │           │           ├─> 04-01 ──┐
        ├───────────┴─> 03-01 ──┘           │
        │                                   │
        └─> 06-01 (needs 02-03) ─> 06-02 ─> 06-03 (needs 04-01, 06-01, 06-02)
                  (needs 03-01)
```

`06-01` depends on `02-03` (the outbound enforce its attach feeds) and `01-01`.
`06-02` depends on `06-01` + `03-01`. `06-03` (the e2e gate) depends on `06-01`,
`06-02`, `04-01`. The happy-path ORDER (intercept → handshake → enforce →
guardrails → activation) is preserved; the production layer is appended as
phase 06.

---

## Step table (summary)

| id | name | deps | production-observable AC boundary | files |
|---|---|---|---|---|
| `06-01` | Production OUTBOUND BPF integration (`MtlsDataplane` load + per-alloc attach + typed `MTLS_REDIRECT_DEST`) | 01-01, 02-03 | `bpftool cgroup show <alloc.scope>` lists `cgroup_connect4_mtls` (and `<workloads.slice>` does NOT — F5 per-alloc scope); `bpftool map dump name MTLS_REDIRECT_DEST` shows/removes the entry; rewrite fires on hit, passes on miss | `overdrive-dataplane/src/mtls/dataplane.rs` (NEW), `mtls/mod.rs`, test |
| `06-02` | Worker intercept-install + leg-acquire (D-MTLS-14 free fns) + inbound TPROXY | 06-01, 03-01 | `getsockopt(IP_TRANSPARENT)==1` on leg-C; `nft list` shows/removes the tproxy rule + `ip rule`/`ip route` companions; `accept_*` builds `InterceptedConnection` with correct `Routed`/orig-dst via `getsockname` | `overdrive-worker/src/mtls_intercept.rs` (NEW), `lib.rs`, test |
| `06-03` | Composition-root activation: construct+probe post-`IdentityMgr`; per-alloc lifecycle; (C)+(B); delete `MtlsSupervisor`; e2e deploy gate | 06-01, 06-02, 04-01 | `overdrive deploy <SPEC>` (declared peer) → `tcpdump 0x17` on peer leg via the REAL boot path, ACTUALLY RUNNING under Lima; `probe()` fail → `health.startup.refused`, no cleartext; production `bpftool` attach; `MtlsSupervisor` deleted | `overdrive-control-plane/src/lib.rs`, `overdrive-worker/src/` (+ `mtls_supervisor.rs` DELETE), e2e test |

---

## Deferrals / blockers surfaced (for orchestrator/user — NO issues created here)

1. **BLOCKER (must pin before `06-03` dispatch): worker-injection mechanism (α
   vs β).** D-MTLS-16's "`Arc<dyn MtlsEnforcement>` is a required
   `compose_production_driver` param" is not literally satisfiable —
   `IdentityMgr` is built after `compose_production_driver`. (α) resequence
   `compose_production_driver` after `IdentityMgr` + add the mTLS port as a
   required `ExecDriver::new` param; (β) a separate `MtlsInterceptWorker`
   lifecycle component constructed post-`:1673`, leaving `ExecDriver` untouched.
   **Recommendation: (β).** The crafter must NOT improvise a builder/`Option` to
   dodge the mandatory-port-param rule (the "invent API surface" failure mode).
2. **Already-tracked deferrals (UNCHANGED, no new issues):** arbitrary-peer
   auto-intercept = #178/#61 (the e2e gate uses a DECLARED peer); operator-tunable
   `MtlsLimits` = #230; kernel-invisible progress-stall watchdog = #232;
   Phase-5 policy force-close = #37/#82 (forward rationale, not v1 work).
3. **No new GitHub issues created** by this proposal. If the user wants the
   worker-injection BLOCKER tracked as an issue, that is a user-approved
   `gh issue create` — not done here.
