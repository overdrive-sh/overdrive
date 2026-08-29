# Expectations — master status table

Surfaces: **O** operator CLI · **R** reconciler/convergence · **D** dataplane/kernel · **E** end-to-end · **X** build/supply-chain.
Status: `pending | satisfied | partial | broken | unanchored-claim | out-of-scope` (see `../README.md`).

| ID | Surface | Expectation | KPI | Anchors | Status |
|---|---|---|---|---|---|
| [O01](O01-kind-rejection-guidance/) | O | Job/Schedule + probe rejected with actionable guidance | K5 | S-SHCP-PARSE-05/06, CLI-12..14 | `pending` |
| [O02](O02-alloc-status-probes-section/) | O | `workload describe` renders a Probes section for a Service | K4 | S-SHCP-CLI-01..06 | `pending` |
| [E01](E01-coinflip-service-honest-early-exit/) | E | coinflip-as-Service honest EarlyExit, never `(took live)` | K1 | S-SHCP-RECON-04, INT-CLI-01, CLI-07..11 | `pending` |
| [O03](O03-deploy-udp-service-accepted-udp-intent/) | O | `overdrive deploy <udp-spec>` accepted; intent carries `Proto::Udp` | K1 | S-04-A, roadmap 01-05, ADR-0060, ADR-0061, US-04 | `satisfied` |
| [E02](E02-udp-service-reverse-path-vip-sourced/) | E | deployed UDP service's reply sourced from VIP, not backend IP | K1 | S-04-A, K1, roadmap 01-03, ADR-0060, ADR-0061, US-04 | `pending` (remote-path) |
| [E03](E03-ca-full-chain-verifies/) | E | full Root → Intermediate → SVID chain verifies under `openssl verify` | K1 | S-04-07, ADR-0063 D1, built-in-ca K1 | `pending` |
| [E04](E04-workload-reachable-at-canonical-address-mtls/) | E | a mesh workload is reachable at its canonical `workload_addr:service_port` over mTLS, end to end | K1 | S-WS, roadmap 03-02, GH #241, canonical-address design + ADR | `pending` |
| [E05](E05-dial-by-name-ping-pong-mtls/) | E | two services dial each other by name; counters advance on a ~10s cadence; each hop is mTLS'd | K-DBN-3 | S-DBN-PINGPONG, roadmap 03-02, ADR-0072 REV-2, GH #243, slice-02 | `pending` |
| [O04](O04-ca-refuse-to-start-actionable-error/) | O | control plane refuses to start on root-key decrypt failure with an actionable, cause-distinct error (no silent re-mint) | K3 | S-02-06/07, ADR-0063 D3/Earned-Trust, journey error_paths step 1 | `pending` |
| [O05](O05-ca-issued-certificates-audit-row/) | O | every issuance observable as an `issued_certificates` audit row via `workload describe`; no silent issuance | K1 | S-05-03/04, ADR-0063 D6, journey step 4 | `pending` |
| [D01](D01-ca-root-key-never-plaintext-at-rest/) | D | root CA private key never plaintext at rest (byte-scan IntentStore) | K3 | S-02-02, ADR-0063 D2/D4, built-in-ca K3 | `pending` |
| [O07](O07-liveness-probe-drives-restart/) | O | a declared liveness probe reaches the reconciler's restart decision | K1 | ADR-0080 D1/D2 + "A third instance", ADR-0055, ADR-0057 §132-134 | `pending` (captured; sub-claim 4 refuted) |
| [E06](E06-vm-job-deploy-reaches-running/) | E | a `[job]` + `[vm]` deploy reaches Running through the production `VmDriver` path | K4 | S-VM-39, roadmap 03-04, K4, DWD-24, ADR-0083, ADR-0082 | `satisfied` |
| [E07](E07-guest-first-mesh-dial-born-captured/) | E | a VM guest's first mesh dial is born captured, exactly rule-accounted, and mTLS-protected | Q9/D7 | S-GTI-01/02, DESIGN Q9/D7, ADR-0088, ADR-0089 | `pending` |
| [E08](E08-vm-guest-boot-failure-truthful-and-clean/) | E | pre-READY guest-network failure is truthful and clean; post-READY exit 78 remains ordinary | Q7 | S-GTI-05/08a/08b, DESIGN Q7, ADR-0088, ADR-0089 | `pending` |
| [E09](E09-vm-guest-reclamation-and-stop-preserve-rules/) | E | same-id platform reclamation and Job stop preserve exact sibling rules | D6 lifecycle | S-GTI-06a/06b/12a/12b, DESIGN D6/D7, ADR-0089 | `pending` |

## Feature coverage

- **service-health-check-probes** — O01, O02, E01 (operator + e2e surfaces).
  The in-process behaviour is covered by the four test tiers; these capture
  the operator-observable and qualitative slice those tiers under-serve.
- **udp-service-support** — O03 (deploy-accepted + udp-intent), E02 (the K1
  reverse-path-VIP-source proof). The in-process logic and the Tier-3 wire
  path are covered by the test tiers (notably the passing
  `reverse_nat_udp_e2e.rs`); these capture the operator-observable deploy
  half and the qualitative end-to-end #163-guard slice those tiers
  under-serve. E02 is the design-time `why` for the
  `reverse_nat_udp_e2e.rs` regression alarm (Stabilize doctrine).
- **built-in-ca** (GH #28) — E03 (full chain verifies under `openssl verify`,
  the walking-skeleton K1 proof), O04 (refuse-to-start on root-key decrypt
  failure with an actionable, cause-distinct error — K3 guardrail / Earned
  Trust), O05 (issuance observable as an `issued_certificates` audit row; no
  silent issuance), D01 (root key never plaintext at rest — K3 byte-scan). The
  in-process logic (CertSpec single-URI-SAN policy, SimCa DST determinism,
  AEAD envelope roundtrip, the `Ca` trait host/sim equivalence) is covered by
  the gated `integration-tests` Rust tiers (`ca_cert_spec_policy.rs`,
  `sim_ca_deterministic.rs`, `rcgen_ca_*.rs`, `ca_equivalence.rs`,
  `ca_boot_and_audit.rs`, `schema_evolution/{root_ca_key,issued_certificate_row}.rs`);
  these four expectations capture the operator/reviewer-observable slice those
  tiers under-serve. All `pending` **by design**: the CA is library-complete and
  proven by the gated tiers, but is intentionally not wired into the operator
  binary this phase (D-CA-4). Unblocked by **#215** (boot-side: wire `boot_ca`
  into `overdrive serve` → D01/O04) + **#35** (consumer-side: SVID issuance on
  alloc-start → E03/O05). Executed at SHA `2f4eccd4`; see
  `docs/evolution/2026-06-06-built-in-ca.md`.
- **canonical-workload-address-inbound-tproxy** (GH #241) — E04 (a mesh
  workload reachable at its canonical `workload_addr:service_port` over mTLS,
  end to end, the K1 round-trip proof). The in-process round-trip through the
  PRODUCTION-installed inbound nft-TPROXY rule is covered by the Tier-3 keystone
  `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs`
  (with a test PKI seam); E04 captures the black-box operator-observable slice
  that tier under-serves. `pending` **by design**: the black-box mesh-mTLS
  E-surface capture needs a converged full-system two-workload deploy with the
  PRODUCTION workload-identity CA proven black-box, provided by **#227** (the
  disposable full-system Lima VM EDD harness) on **#75** (the Image Factory OS
  image). Neither has landed, so E04 cannot be captured against the built binary
  yet.
- **dial-by-name-responder** (GH #243, ADR-0072 REV-2) — E05 (two services dial
  each other by name; both counters advance on a ~10s cadence over a 60s window;
  each hop intercepted + mTLS'd — the operator-runnable bidirectional proof,
  K-DBN-3). The in-process dial-by-name loop is covered by the Tier-3 modules
  `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`
  (single-direction, GREEN) and `dns_responder_ping_pong.rs` (the bidirectional
  RED scaffold, with a test PKI seam); E05 captures the black-box
  operator-observable slice those tiers under-serve. `pending` **by design**: the
  black-box bidirectional mesh-mTLS E-surface capture needs a converged
  full-system two-workload deploy with the PRODUCTION workload-identity CA proven
  black-box, provided by **#227** (the disposable full-system Lima VM EDD
  harness) on **#75** (the Image Factory OS image) — the SAME precondition as
  E04. The example specs `examples/dial-by-name-responder/{a,b}.toml` + a real
  on-disk staged Rust ping-pong bin are READY for the capture; neither harness
  has landed, so E05 cannot be captured against the built binary yet.

- **probe-idx-per-role / ADR-0080 Stage 1** — O07 (a declared liveness probe
  reaches the reconciler's restart decision, observed black-box through
  `overdrive deploy` + `overdrive workload describe`). The three fixtures
  `examples/liveness-{absent,fails,holds}-service.toml` differ from each other by
  ONE input each, so the diff is attributable: no liveness probe → not
  restarted; a declared failing one → restarted. That is ADR-0080 § D1 seen from
  outside the binary, and it is the surface the in-process tiers structurally
  could not cover — the readiness/liveness acceptance suites hand-build a
  `ServiceAllocFact` the production hydrate cannot produce (ADR-0080 § "Why
  nothing caught it"). Status `pending` **by design**: evidence IS captured
  (`executed_in_lima: true`, SHA `86d6331b`), but sub-claims 1–3 pass while
  sub-claim 4 is **refuted** — a Service whose liveness probe targets its own
  demonstrably-serving listener is restarted too, because TCP probes are issued
  from the control plane's network namespace and the workload's socket lives in
  the allocation's own `ovd-ns-NNNN`. That reachability gap **predates ADR-0080**
  and Stage 1 did not cause it; Stage 1 is what makes it operator-visible. The
  verdict is left to a different-fox adversarial audit rather than self-stamped,
  and `partial`/`broken` are not claimed because the legend requires a linked
  issue and issue creation needs explicit user approval (CLAUDE.md § "Deferrals
  require GitHub issues"). O07 deliberately does NOT claim ADR-0080 § D4
  (`Stable` stops only the startup role): no allocation reaches `Stable` here,
  since the startup probe is unreachable for the same reason.

- **microvm-driver-cloud-hypervisor** (GH #42) — E06 (a `[job]` + `[vm]` deploy
  reaches Running through the production `VmDriver` path — S-VM-39 seen from
  outside the binary). This is the catalogue's **K4 instrument**, assigned by
  name in `feature-delta.md:2427`: *"A real `overdrive serve` + `overdrive
  deploy` in the verification catalogue — `verification/harness/run-expectation.sh`
  — Per slice"*. K4 is the feature's binary pass/fail bar, and the surface no
  in-process tier can cover: the Tier-3 witness
  `job_plus_vm_spec_is_accepted_and_its_allocation_reaches_running`
  (`crates/overdrive-cli/tests/integration/vm_boot_failure_vocabulary.rs`,
  step 03-04 / `4243e849`) does boot a real guest and reach Running, but it
  reaches the composition root through the `integration-tests`-gated helper
  `run_with_dataplane_and_vm_artifacts` — so by construction it cannot answer
  whether an *operator* can get there.

  **First expectation in this catalogue whose execution substrate is NOT
  Lima.** It boots a real Cloud Hypervisor guest, which needs x86_64 + nested
  KVM; Lima on Apple Silicon has neither, so the runner uses `cargo xtask metal
  run --` against `$OVERDRIVE_METAL_TARGET` (`.claude/rules/testing.md` §
  "bare-metal KVM box"). Consequence, stated rather than hidden:
  `verification.yaml`'s `executed_in_lima` field records only that the runner
  executed, so it reads `true` while nothing ran in Lima —
  `evidence/execution_substrate.txt` is the accurate record for E06, and the
  harness's Lima-shaped field should not be read as a Lima claim here.

  Status `satisfied` — capture SHA `fff9fe16`, `runner_exit_code: 0`, all four
  sub-claims pass and a different-fox adversarial audit (2026-08-19, reading
  only the `evidence/`) confirmed it. A `[job]` + `[vm]` deploy is accepted
  (`Accepted.`, exit 0), the allocation reaches `Running` within ~2s, a real
  `cloud-hypervisor` guest is spawned (`new_hypervisors=1 / new_run_dirs=1 /
  new_scopes=1`), and teardown leaks nothing. **K4 now reads MET** on the
  shipped binary's own argv. The road there is the K4 story, worth keeping:

  - SHA `655ac964` — **refuted**. The allocation sat at `Failed` for the full
    90s ceiling — *"driver internal error: no vm driver composed on this
    node"*. The shipped binary had no operator-reachable way to compose a `Vm`
    driver: the `[vm]` boot-artifact seam was `#[cfg(feature =
    "integration-tests")]`, reachable by no argv/env/config. Exactly the shape
    the feature's risk register predicted — *"the mechanism composes but no
    production path reaches it"* (`feature-delta.md:2445`).
  - SHA `6b6ffb12` — **green (pre-confinement)**. Step 03-07 (DWD-25 /
    ADR-0083 §§D3a-D3c) made VM composition unconditional (gated on
    `Vmm::probe`) and moved artifacts per-allocation into the spec's own
    `[vm]` block; the runner reached Running unmodified.
  - SHA `fff9fe16` — **green (post-confinement, current)**. The confinement
    work (ADR-0082 fourth amendment) then added two create-time preconditions
    on the operator data-dir — same filesystem as the rootfs master (FICLONE is
    intra-filesystem, fails closed on EXDEV) and traverse-by-the-dropped-uid
    (`OVERDRIVE_VMM_UID=4200`) — which the appliance meets by construction
    ("one VM data partition") but E06's ephemeral `mktemp` serve data-dir did
    not on the shared box. The runner (commit `fff9fe16`) co-locates the
    data-dir on the master's partition and grants `0711` traverse, modelling
    the appliance; the black-box operator surface is unchanged and the in-tree
    S-VM-39 witness passes at HEAD, so this is substrate-modelling, not a
    product accommodation. Both preconditions are temporary — overdrive-fs
    (GH #97) supersedes them.

- **guest-stack-transparent-mtls-intercept** (GH #222) — E07 captures the
  built-binary born-captured/D7/TLS outcome; E08 captures truthful pre-READY
  failure, total cleanup, and the post-READY status-78 complement; E09 captures
  the production-reachable same-allocation platform-reclamation route and exact
  `overdrive job stop <id>` sibling preservation. All three are `pending` stubs.
  Runtime evidence is accepted only from a native, non-virtualized x86_64 KVM
  host under the shared command-lifetime lease; nested KVM is forbidden and
  Lima is compile-only. Their READMEs pin commands, state/wire/kernel evidence,
  deadlines, cleanup, and preflight. The roadmap/DEVOPS handoff must assign and
  land the shared runner before capture.

## Adding an expectation

1. `mkdir verification/expectations/<SURFACE><NN>-<slug>/` with a `README.md`
   (scenario + `- Anchor:` lines + verification block + `Status: pending`).
2. Add an optional `runner.sh` that drives the **built** `overdrive` binary
   via the `od` helper (real commands; executed in Lima).
3. Add a row here.
4. Run `harness/run-expectation.sh <ID>`, review the evidence adversarially,
   then set the status in the expectation's `README.md`.
