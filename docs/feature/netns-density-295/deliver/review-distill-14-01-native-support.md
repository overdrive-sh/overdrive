# DISTILL review — STEP 14-01 existing GuestKit native support

## Review metadata

| Field | Value |
| --- | --- |
| Review ID | `STEP14-01-DISTILL-NATIVE-SUPPORT-REVIEW-01` |
| Feature / roadmap step | `netns-density-295` / `14-01` |
| Review date | 2026-10-29 |
| Reviewer | Fresh independent DISTILL source and evidence reviewer |
| Iterations | 1–5 |
| Implementation base | `822b533950fcfeb51f72444b574fe1f05b93cb97` |
| Scope | Existing native support for the six already-authored STEP 14-01 bodies, including nested GuestKit confinement, exact selection and accounting, compiler-only fixture fallout, durable receipts, and the claimed fixture-contract escalation |
| Design authority | Feature-delta V-6 and V-LEGB-SERVE; roadmap and dispatch STEP 14-01; current workspace `AGENTS.md` test-support ownership rule |
| Verdict | **APPROVED** |
| Open findings | 0 blocker, 0 high, 0 medium, 0 low |

This review applies the repository's current `AGENTS.md`, applicable project
rules, and the nWave DISTILL, acceptance-review, completeness, test-design and
BDD criteria. The repository's direct Rust scenarios and native-Markdown
review convention take precedence over the installed review definition's
generic `.feature` and YAML examples.

Only this review artifact was added by the reviewer. No production source,
test source, test-support source, acceptance body, assertion, Contract Shape
declaration, pending marker, roadmap, DES event, execution log, model, lane,
public API or commit was changed. Pre-existing dirty work, including
`AGENTS.md`, the author's support delta and the independently modified
`execution-log.json`, was preserved.

## Review boundary

The candidate package consists of:

- the uncommitted existing-runner extension in
  `crates/overdrive-testing/bin/fwd_guest_suite.rs`;
- the corresponding preparation changes in `xtask/src/guest_kit_prepare.rs`;
- the neutral E6 compiler fallout in
  `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs`;
- activation of one CLI and five control-plane bodies by deleting only their
  STEP 14-01 pending markers; and
- the source and durable-evidence handoff in
  `deliver/distill-14-01-native-support.md`.

The review does not ask for a new scenario, lane, group, executor, production
kernel-command-line token, production configuration seam or public API. It
also does not approve STEP 14-01 production behavior: the six authored bodies
must first execute through the corrected existing fixture and establish the
semantic RED or GREEN state that DELIVER owns.

## Iteration 1 reviewed artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| STEP 14-01 dispatch | `ef672d8849ff68fde6a1f4d2050e09957373030eeb4b580d7c1999c442d593f7` |
| `deliver/distill-14-01-native-support.md` | `ba1e072be987f87e07432ffa5519bb63d7c1dd56531848cde7f5b128a60b460f` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| `xtask/src/guest_kit_prepare.rs` | `e436cea4685646eb044606ccf5af1e8fe3f6f9e78581ad7cda3dac16d87a0e69` |
| E6 compiler fixture | `2b67117eff47b1252ab66348dbc0e8f6ad3cac86579bdbab97caee9cc0dbd341` |
| Activated CLI body | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| Activated control-plane bodies | `c8df77ed2880a3a56b249f6172ab97280155e07149d036c9f1bac4b5afadc937` |
| Existing `guest_kit.rs` | `9061cd378171fb864d4bc5fcfa51305604cf15dcd35af759f8305dc78f1dc54e` |
| Existing `guest_peer.rs` | `ea3de0694b0164c7b8c84b3a08db2d6f275b72b252e29b10ef8eed31fb6cf8ea` |
| Existing `vm_fixture.rs` | `520af706adfc446b16c9722e6f0fe924f7fd48c10b8dbc96dc8946110bd07e3a` |
| Accepted feature delta | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |

The candidate source identities match the handoff. The decisive retained
output and terminal records independently reproduce the handoff hashes:

| Receipt | Verified SHA-256 |
| --- | --- |
| `ec5b4b3df9b68361f01ad075/output.log` | `ac238d47aa51398378b467dec961ed13fd9c716352fcfafce1b1d273daa3c697` |
| `ec5b4b3df9b68361f01ad075/terminal.v1` | `70cfb5b240697600d32eb91bece3922072c50b4126fbde2c36d3e9d7d89dc747` |
| `8d02925499a5928ce99688e1/output.log` | `b202453830381e1ee1ab5bcc574987cc1a31e347dd85349240c235acfbf091dc` |

## Strengths

- The runner reuses the existing `cgroup-default` selector, existing
  `overdrive-fwd-guest-suite` executor and registered 16-CID GuestKit band. It
  adds neither a public selector nor a separate execution lane.
- STEP 14-01 runs inside two fresh disposable outer Cloud Hypervisor guests.
  The production test binaries then launch the real workload VMs through
  nested KVM. Kernel modules, cgroup v2, bpffs, securityfs, nft, routes,
  namespaces and inner VMMs remain inside the disposable outer guest.
- The nested support stages the already-used kernel, Cloud Hypervisor fork,
  prebuilt production init, BPF object, module sources, tools and XFS reflink
  volume. `/dev/kvm` and `/dev/vhost-vsock` retain the existing 0660
  device-owner-group posture before the production confined VMM opens them.
- Every target is selected by exact libtest identity and runs through the
  existing required-execution path: nonzero exit fails, the exact successful
  summary is required, and recognized runtime-skip diagnostics are rejected.
- The five control-plane identities were enumerated from the built binary by
  durable run `bcaa2904fa55d407c296ea8e`, which exited zero. The source
  manifest contains exactly those five identities plus the one CLI walking
  skeleton.
- All six bodies retain `/// CONTRACT_SHAPE: bounded-change.`. A diff from
  base `822b53395` contains exactly six deleted STEP 14-01 ignore markers and
  no body or oracle edit.
- The E6 changes are bounded compiler fallout. They remove a deleted wrapper,
  continue matching the typed `InterceptError::NftRuleInstallFailed` cause,
  and align the readback with the accepted one-set / three-prerouting /
  nine-output program identity. They do not claim to repair the two known E6
  runtime failures.
- The decisive native run is honest. It re-proves the unchanged 111-body
  boundary before entering the target body, reaches production `serve`, the
  public deploy handler, nested Cloud Hypervisor and the real inner stock
  kernel, then fails at a concrete fixture entrypoint. It is neither a compile
  failure nor a missing-KVM/runtime-skip result.
- The corrected physical-host audit `069c568cbf6b443ad733b611` exited zero
  with no live Cloud Hypervisor process, no physical-host nft table and no
  STEP 14-01 work directory. It records the unrelated pre-existing
  `ovd-ns-0000` namespace without claiming or deleting it.

## Exact selection, retained accounting and safe boundary

| Boundary | Static source | Executed evidence |
| --- | ---: | --- |
| Existing `cgroup-default` bodies | 111 | 111 passed, 0 failed, 0 ignored |
| STEP 14-01 CLI body | 1 exact identity | Entered once; failed at fixture entrypoint |
| STEP 14-01 control-plane bodies | 5 exact identities | Not reached after fail-fast CLI failure |
| STEP 14-01 total | 6 | Incomplete |

Independent full-stream accounting of run
`ec5b4b3df9b68361f01ad075` found 101 successful libtest summaries whose
declared counts sum to 111, every summary naming zero failures and zero
ignored bodies, and zero diagnostics recognized by the runner's runtime-skip
policy. The three retained pre-STEP-14 outer guests completed. The run then
entered the exact CLI body and exited 101, so it cannot establish either the
CLI outcome or any of the five control-plane outcomes.

The failure is nevertheless a valid bounded reproduction of the fixture
defect. The raw log records:

```text
state: Failed
reason: VmGuestExitUnreported { vmm_exit_code: Some(0), vmm_signal: None }
guest console: overdrive-guest-peer: no overdrive.testpeer= on the kernel command line
assertion: expected Terminated, observed Failed
```

This production-reachable path is complete: `Lane::boot` builds an existing
`GuestImageSpec::workload()` image; `deploy_job` submits that rootfs and the
existing guest application through the public deploy handler; production
`serve` renders its accepted platform plus `overdrive.transport=` command
line; the real nested VMM boots the rootfs; and PID 1 takes the scripted-peer
branch instead of replacing itself with the embedded production init.

## Necessity of escalation

The handoff correctly rejects adding `overdrive.testinit=` or
`overdrive.testpeer=` to the production VM command line. That would be a
test-only production override and would violate the accepted command-line
contract.

The handoff incorrectly concludes that the only remaining path is an exact
replacement DESIGN fixture contract. Current `AGENTS.md` explicitly assigns
test-support surface, fixtures and test-local implementations to DISTILL and
states that a gap in test support is not a DESIGN blocker. The reproduced
failure is entirely inside the existing `overdrive-testing` fixture:

1. `GuestImageSpec::workload()` already declares `GuestInit::OverdriveInit`.
2. `GuestKit::image` places the existing test peer at `/sbin/init` and `/init`,
   and places the existing production init at
   `/lib/overdrive/overdrive-init` only for that workload-image mode.
3. The test peer already owns the required stock-kernel fixture preparation:
   proc/sys/dev mounts, loopback, vsock modules, `RLIMIT_NOFILE`, dummy and
   socket-diagnostic modules, then `exec` of the embedded production init.
4. Its current selection enters that production-init path only when the
   private `overdrive.testinit=` token is present. When production `serve`
   boots the same image without that private token, it falls through to the
   scripted-peer control-port requirement.

No production interface, typed error, wire format, config surface, lifecycle
meaning or observable outcome is unspecified. Escalation to DESIGN is not
necessary.

## Finding F-01 — the existing workload image has a correctable private entrypoint defect

**Severity:** blocker  
**Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — DISTILL test-support remediation  
**Status:** open

### Evidence

- `deliver/distill-14-01-native-support.md:12-28` labels the condition a
  blocking fixture-contract gap, and lines 185-188 require DISTILL/DESIGN to
  record a replacement contract.
- `crates/overdrive-testing/src/guest_kit.rs:355-364` already defines the
  workload image as `GuestInit::OverdriveInit`.
- `crates/overdrive-testing/src/guest_kit.rs:492-505` already distinguishes
  that image by embedding `/lib/overdrive/overdrive-init` together with the
  stock-kernel fixture modules.
- `crates/overdrive-testing/bin/guest_peer.rs:674-719` already contains the
  complete fixture-prerequisite-and-exec path, but line 703 reaches it only
  through the private command-line token.
- `crates/overdrive-testing/bin/guest_peer.rs:737-741` is the wrong branch
  observed in durable run `ec5b4b3df9b68361f01ad075`.
- `crates/overdrive-testing/src/vm_fixture.rs:123-143` and lines 793-801
  confirm that a production no-`init=` block-root boot reaches `/sbin/init`;
  the fixture image must therefore make its existing PID-1 wrapper choose the
  already-embedded production init without requiring a private production
  command-line token.

### Required bounded remediation

Keep the existing test-only PID-1 wrapper so it can load the stock-kernel
fixture prerequisites before production init runs. In
`crates/overdrive-testing/bin/guest_peer.rs`, make the program selected by the
existing production-fixture branch resolve in this order:

1. the current explicit `overdrive.testinit=` value, preserving all existing
   direct `GuestKit::boot` behavior; then
2. the existing fixed `OVERDRIVE_INIT_GUEST_PATH` when that file exists in the
   image.

The second case is an image-local mode discriminator, not a new API or
override: `GuestKit::image` installs that file only for the existing
`GuestInit::OverdriveInit` workload mode. It must reuse the current
`raise_nofile`, `load_production_fixture_prerequisites` and `exec` sequence.
If the file is absent and no explicit token exists, the existing scripted-peer
path and its required `overdrive.testpeer=` behavior remain unchanged.

The minimal source shape is equivalent to selecting:

```rust
test_init().or_else(|| {
    Path::new(OVERDRIVE_INIT_GUEST_PATH)
        .is_file()
        .then(|| OVERDRIVE_INIT_GUEST_PATH.to_owned())
})
```

using the already-public constant from the test-support crate. Do not replace
the wrapper with production init directly at `/sbin/init`: that would bypass
the existing dummy and socket-diagnostic module preparation required by the
stock-kernel fixture. Do not add a production cmdline token, production
config, public constructor field, new lane, selector, executor, scenario or
test-body edit.

After the correction:

- rerun the same existing Metal `cgroup-default` command inside the disposable
  GuestKit boundary;
- require exact exit zero, the retained 111-body total, all six STEP 14-01
  exact one-pass summaries, zero failures, zero ignored bodies and zero
  recognized runtime skips;
- require both fresh STEP 14-01 outer guests to finish and retain cleanup
  evidence for inner workload VMMs, outer VMMs, nft state and workdirs;
- rerun the scoped format/check/Clippy gate against the remediated source;
- keep all six authored bodies byte-identical to base except for their already
  removed pending markers; and
- update the handoff to classify the condition as the closed fixture
  entrypoint defect rather than a DESIGN contract ambiguity.

If the corrected existing wrapper reaches a new failure, classify that actual
failure from its production path and evidence. It does not justify a broader
mechanism in advance.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | The unchanged package contains a user-visible walking skeleton plus owner/watch/abort/reset complements. |
| GWT / one behavior | 9/10 | Each direct Rust body retains one concrete production trigger and observable outcome. |
| Domain language | 10/10 | Workload, allocation, owner, probe, relay, round trip and VMM lifecycle terms remain precise. |
| Coverage completeness | 4/10 | The runner selects all six identities, but one stops at fixture setup and five never execute. |
| Walking-skeleton user value | 10/10 | The CLI body still expresses the operator's serve + deploy journey and byte-exact TCP/UDP outcomes. |
| Priority validation | 4/10 | The reproduced failure is real, but routing an existing private fixture correction to DESIGN stops before the smallest authorized fix. |
| Observable behavior | 10/10 | The unchanged bodies assert operator rows, remote socket observations, console outcomes and VMM cleanup rather than private implementation state. |
| Traceability | 10/10 | The exact identities remain tied to P6-S13, V-6, V-LEGB-SERVE, U-D17-DIAL, R5-57, R5-63 and R5-41. |
| Walking-skeleton boundary | 4/10 | Production serve + deploy is reached, but the workload rootfs cannot yet enter production init under production's accepted cmdline. |

The deterministic reviewer rule rejects the package because multiple
dimensions remain below 5 and one blocker is open.

## Test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | The CLI body reaches production `serve` plus the public deploy handler with no override seam. |
| CM-B — domain abstraction | PASS | No acceptance vocabulary, assertion or authored body changed. |
| CM-C — complete user journey | PASS in design, unproved in execution | The walking skeleton is complete, but fixture setup prevents its outcome from being observed. |
| CM-D — fixture isolation | **FAIL** | The disposable boundary is sound, but the workload image's private PID-1 wrapper is incompatible with the production cmdline despite already containing its production init. |
| CM-E — universe-bound assertion | PASS / layer N/A | These are layer-4+ real-kernel integration bodies, where direct observable assertions are permitted. |
| CM-F — layer-dependent PBT | PASS | The native cells remain explicit examples; no PBT machinery was added. |
| CM-G — two-tier rich journey | N/A | This support package adds no new journey or Tier-B surface. |
| CM-H — explicit integration paths | PASS in source | All six paths are named examples, though the five control-plane bodies still need executed evidence. |

The C1-C7 taxonomy is unchanged because the candidate adds no scenario,
input class, state, operation, mode, semantic error or concurrency claim. The
canonical 15-item checklist remains 15/15 under its documented N/A rule for
unchanged categories. That scenario-set completeness does not waive CM-D or
the fail-for-the-right-reason fixture gate: the current failure occurs before
the authored behavior can run.

## Mechanical verification

| Check | Result |
| --- | --- |
| Scoped diff | PASS — runner, preparer, neutral compiler fixture and six marker deletions; no production source change. |
| Exact STEP 14-01 manifest | PASS — one CLI plus five control-plane identities, each expected once. |
| Contract Shape | PASS — six bounded-change declarations, zero remaining STEP 14-01 ignores. |
| Test-body integrity | PASS — exactly six ignore-line deletions from `822b53395`; no body or assertion change. |
| Retained boundary | PASS — 101 successful summaries sum to 111 passed, 0 failed, 0 ignored. |
| Runtime-skip audit | PASS — zero recognized diagnostics in the decisive run. |
| Nested real-VM reachability | PASS — production serve launches the workload VMM and the inner stock kernel boots. |
| STEP 14-01 CLI outcome | FAIL — fixture PID 1 chooses scripted-peer mode. |
| STEP 14-01 control-plane outcomes | NOT EXECUTED — fail-fast after the CLI body. |
| Safe physical-host boundary | PASS — only the outer confined GuestKit VMM runs on Metal; read-only audit finds no live VMM, nft table or STEP 14-01 workdir. |
| Source format/check/Clippy | PASS for candidate source — durable run `8d02925499a5928ce99688e1`, exit 0. Must be refreshed after remediation. |
| Durable raw identity | PASS — decisive output and terminal hashes independently reproduced. |
| `git diff --check` | PASS. |
| New production API / config / cmdline / scenario / lane / selector / executor / model | NONE. |
| Mutation testing | NOT RUN — explicitly unauthorized. |

## Iteration 1 verdict

**NEEDS REVISION.** The runner and staging support are well bounded, safely
confined and evidence-backed. They preserve the 111-body baseline, select the
six exact authored identities, keep the test bodies honest and reach the real
nested workload VM through production `serve` and deploy.

The claimed DESIGN blocker is not necessary. The existing workload image
already contains a fixed production-init path and the existing test-only PID-1
wrapper already owns every required stock-kernel prerequisite before `exec`.
DISTILL must make that wrapper select the embedded production init when the
private direct-GuestKit token is absent, while leaving scripted images and the
production command line unchanged. This is a private fixture-entrypoint
correction within DISTILL ownership.

Return the bounded remediation and refreshed exact native evidence to this
same reviewer. STEP 14-01 DELIVER must not resume until this DISTILL review
records an approved iteration.

# NEEDS REVISION

## Iteration 2 re-review

Iteration 2 reviews only the original finding's remediation and the refreshed
evidence needed to classify the new failure. All iteration-1 strengths and
scope constraints remain in force. The reviewer made no source, test, DES,
execution-log or commit change.

### Iteration 2 artifact identity

| Artifact | SHA-256 at re-review |
| --- | --- |
| Remediated handoff | `920515d578173363d951f06423a810d330d2deb4c26e4e6c1285e14df294d04b` |
| `overdrive-testing/bin/guest_peer.rs` | `3d8c8d70da22f64f2c48c4ffd0b14efae503704e3ffbc4424328ee2872d69f39` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| `xtask/src/guest_kit_prepare.rs` | `e436cea4685646eb044606ccf5af1e8fe3f6f9e78581ad7cda3dac16d87a0e69` |
| E6 compiler fixture | `2b67117eff47b1252ab66348dbc0e8f6ad3cac86579bdbab97caee9cc0dbd341` |
| Activated CLI body | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| Activated control-plane bodies | `c8df77ed2880a3a56b249f6172ab97280155e07149d036c9f1bac4b5afadc937` |
| Accepted feature delta | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |

The remediation changed only the existing test-support PID-1 binary after the
iteration-1 handoff. The runner, preparer, compiler fixture and six activated
test sources retain their iteration-1 identities. The remediated peer's
Git-blob identity is `33e373c916d61fc8df34885cb4dc822a9568add3`.

### F-01 remediation verification

The implementation matches the required bounded correction exactly:

- `production_init()` first calls the existing `test_init()` parser. Rust's
  lazy `Option::or_else` means an explicit `overdrive.testinit=` value always
  wins, including an explicitly invalid or missing path; the new fallback
  cannot silently replace it.
- Only when that token is absent does the helper test the existing fixed
  `OVERDRIVE_INIT_GUEST_PATH` with `Path::is_file()` and select it.
- `GuestKit::image` still installs that file only for the existing
  `GuestInit::OverdriveInit` mode. A `GuestInit::ScriptedPeer` image therefore
  still falls through to the existing required `overdrive.testpeer=` branch.
- The selected program still enters the same `raise_nofile`,
  `load_production_fixture_prerequisites` and `Command::exec` sequence. The
  dummy and socket-diagnostic module preparation remains before production
  init; the correction does not bypass the stock-kernel fixture envelope.
- No production crate, public type, constructor field, config, kernel token,
  test body, oracle, scenario, lane, selector or executor changed.

The prior wrong branch is absent from the refreshed native log. The inner
console now contains the kernel's `Run /sbin/init as init process`, vsock
registration and production `overdrive-init`'s own typed fatal diagnostic.
This proves the image-local fallback is exercised in the exact production
`serve` + deploy path, rather than only compiling or passing through a direct
`GuestKit::boot` call.

### Semantic RED classification

Durable run `25c755136da086ec4ce18c0a` is a valid
`MISSING_FUNCTIONALITY` RED, not a fixture, compilation or infrastructure
failure.

The evidence chain is exact:

1. The CLI fixture configures `NodeVsockPortConfig::kernel_assigned()`, so the
   production beacon, stream and control listeners bind kernel-assigned ports.
2. Production `serve` successfully composes and binds the real beacon intake,
   and the production VM driver launches the inner VM with the accepted
   `overdrive.transport=vsock2,...,beacon=<bound-port>,stream=<bound-port>,control=<bound-port>`
   token.
3. The remediated fixture loads its stock-kernel prerequisites and execs the
   existing production init. KVM, nested Cloud Hypervisor, the stock kernel,
   vsock protocol and PID-1 entrypoint therefore all work.
4. The accepted GUEST-PORTS contract states that `overdrive-init` connects the
   beacon, control session and every flow only to the parsed token ports and
   holds no host-port constant.
5. Current production source still calls `connect_beacon` before parsing the
   transport token, and `connect_beacon_once` constructs the address with
   `BEACON_VSOCK_PORT` (`1234`). It therefore dials a different port from the
   kernel-assigned intake and receives `ECONNRESET`.
6. The unchanged walking-skeleton oracle observes the resulting allocation as
   `Failed` with `VmGuestExitUnreported` rather than the required
   `Terminated` state. The assertion, production entrypoint and owner path are
   all real; no test consequence was fabricated.

The raw failure is:

```text
Run /sbin/init as init process
NET: Registered PF_VSOCK protocol family
overdrive-init: fatal: could not connect the beacon vsock socket to the host
after repeated retries: ECONNRESET: Connection reset by peer
```

The runner collected the target body and reached its unchanged assertion. The
failure is the exact accepted production contract still missing from the
implementation. DISTILL must stop at that semantic RED; making the product
use the token's beacon port belongs to the STEP 14-01 crafter's RED → GREEN →
COMMIT cycle. Requiring all six bodies to pass before approving the test
support would incorrectly require DISTILL to implement product behavior. The
iteration-1 post-remediation request for six successful summaries is therefore
superseded by this fail-for-the-right-reason evidence.

The five control-plane bodies remain selected exactly once and are not reached
because the existing group intentionally fails fast on the walking skeleton's
semantic RED. That is correct at the DELIVER RED entry gate. Once the crafter
implements the accepted dynamic-beacon contract, the same unchanged runner
continues into those bodies and applies the existing exact-summary and
runtime-skip gates.

### Refreshed native evidence

| Evidence | Independent result |
| --- | --- |
| Semantic RED run | `25c755136da086ec4ce18c0a`, source digest `8c81ee137d589a74d8c961656d2634c0648390888e82c3016741abc5b257894b`, exit 101, `command_returned` |
| Retained summaries | 101 successful summaries declaring 111 passed, 0 failed and 0 ignored |
| Recognized runtime skips | 0 |
| STEP 14-01 CLI starts | 1 exact identity |
| Old scripted-peer missing-token diagnostic | 0 |
| Production-init beacon fatal diagnostic | 1 |
| Semantic-RED output | 397,879 bytes; SHA-256 `f618ec0ca1c8462df94c0b82875df9d08a28c686ceaaa7b623fe8e41e7913662` |
| Semantic-RED terminal | SHA-256 `ebeac03a837887c4ba2030f89d54949276d9f2b26e3a55dace5ae7d258d83e5d` |
| Final format/check/Clippy | `85de600c150b2c60d426ff53`, matching source digest, exit 0 |
| Final source-gate output | SHA-256 `56e9c27277ca1fb38eb640d9ce8c87e0c581026e697c42548f6c55b4ae87429c` |
| Physical-host cleanup | `8a64bea495949838e9a0c174`, matching source digest, exit 0 |
| Cleanup observations | `live_cloud_hypervisor=0`; no nft table; no STEP 14-01 workdir |
| `git diff --check` | PASS |

The failed body reports its teardown bound and the empty workload-scope
complement before the outer process ends. The disposable `GuestKitVm` and
`WorkDir` guards then destroy the outer guest and staged artifacts. The
separate read-only Metal audit confirms no physical-host VMM, nft or workdir
leak after that unwind. No physical-host production test binary or
kernel-mutating fixture ran.

### Finding disposition

#### F-01 — existing workload image private entrypoint defect

**Disposition: CLOSED.**

The exact private fallback is present, preserves explicit-token precedence and
scripted-image behavior, reuses the existing prerequisite-loading path, and is
proved through the real nested production composition. The previous
`overdrive-guest-peer: no overdrive.testpeer=` fixture failure is gone.

The resulting fixed-port beacon failure is not a new DISTILL finding. It is
the expected semantic RED for the accepted GUEST-PORTS production contract
and is assigned to the STEP 14-01 crafter.

#### New iteration 2 findings

None.

### Iteration 2 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | The unchanged package contains the walking skeleton plus owner/watch/abort/reset complements. |
| GWT / one behavior | 9/10 | The first body reaches one production behavior and fails on its exact missing contract. |
| Domain language | 10/10 | Workload, allocation, beacon intake, owner, probe, relay and VMM lifecycle terms remain precise. |
| Coverage completeness | 9/10 | All six bodies are selected exactly; fail-fast stops at the first honest shared-path RED as the DELIVER cycle requires. |
| Walking-skeleton user value | 10/10 | The CLI body remains the operator's production serve + deploy journey with TCP/UDP and cleanup outcomes. |
| Priority validation | 10/10 | Remediation fixes only the reproduced private fixture defect and exposes the accepted missing product behavior. |
| Observable behavior | 10/10 | The RED is the unchanged allocation-state oracle plus production init's real console diagnostic. |
| Traceability | 10/10 | The exact bodies and failure map to P6-S13, V-6, PORTS-CONFIG and GUEST-PORTS. |
| Walking-skeleton boundary | 10/10 | Production serve, public deploy, real listener ownership, real VM launch and production init are all reached with no override seam. |

Every dimension now meets the approval threshold and no blocker remains.

### Iteration 2 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | The semantic RED runs through production `serve`, the public deploy handler, the real VMM and production init. |
| CM-B — domain abstraction | PASS | No authored body or assertion changed; private fixture selection remains below the acceptance vocabulary. |
| CM-C — complete user journey | PASS as RED | The complete journey is authored and reaches the exact missing product contract that prevents its outcome. |
| CM-D — fixture isolation | PASS | The PID-1 correction is private, image-local, confined to the disposable guest and production-cmdline compatible. |
| CM-E — universe-bound assertion | PASS / layer N/A | These are layer-4+ real-kernel integration bodies with direct observable assertions. |
| CM-F — layer-dependent PBT | PASS | The native cells remain explicit examples. |
| CM-G — two-tier rich journey | N/A | The remediation adds no journey or Tier-B surface. |
| CM-H — explicit integration paths | PASS | Six named paths remain selected; the first supplies the required semantic RED. |

The C1–C7 taxonomy and canonical 15-item checklist remain unchanged and
complete under their N/A rule because iteration 2 adds no scenario or contract
claim. The fail-for-the-right-reason gate now passes: the collected body
reaches its unchanged observable assertion and fails solely because accepted
production functionality is missing.

## Iteration 2 verdict

**APPROVED.** The private PID-1 fallback closes the sole iteration-1 finding
without changing production surface or authored acceptance behavior. Explicit
`overdrive.testinit=` retains precedence, only a workload image carrying the
existing embedded production init takes the fallback, scripted-peer images
remain unchanged, and the stock-kernel prerequisite-loading sequence remains
intact.

The refreshed native run proves the correction through the real nested path
and reaches an honest semantic RED: production `overdrive-init` still dials
fixed beacon port 1234 while the accepted token names the kernel-assigned
beacon intake. The unchanged walking-skeleton oracle therefore observes
`Failed` instead of `Terminated`. That is the STEP 14-01 product work DELIVER
must now implement, not another fixture defect.

There are no open DISTILL findings. The approved uncommitted support package
may return to the STEP 14-01 crafter for the exact accepted RED → GREEN →
COMMIT cycle. This verdict does not pre-approve the later implementation
review.

# APPROVED

## Iteration 4 assessment — R5-41 workload-kind fixture mismatch

Iteration 4 assesses the native timeout in the exact authored R5-41
through-`serve` body. It reconciles the accepted R5-41 outcome, the test's
fixture precondition, the production `WorkloadLifecycle` kind contract and the
retained native/ordinary/seeded evidence. The reviewer changed only this
review artifact.

### Evidence identity

| Artifact | Identity at review |
| --- | --- |
| Diagnosis | `.context/netns-density-295/14-01/cp-replacement-diagnosis.md`; SHA-256 `a7947ddfdf1137ca28f8b1d3e2cee9c513259c3bd503660a900be97444bfe894` |
| Reachability run | `d42014b2202f9df175307503`; output SHA-256 `6a3f64583c1e767313edf5c0636efa27c5e26df2133e7aff19f91dab9650fd5b`; terminal SHA-256 `49365c9dd5cbe44ade0578a7d5bafe2cc52e5d9528f6776ddeaea495186c7e26` |
| Job no-restart Metal evidence | `8e2dc8fe0ae5242845a89b85`; collected output SHA-256 `f6f25fd4234445a1bf4e6ef136494c3c8ce4bd1689ad5900ac12eca0439bc080` |
| Restart-SSOT seeded evidence | `21b992e20519e96eeb2b7d58`; collected output SHA-256 `653b0680c5c32d1df0216b8d76efc732cdb144cc0e224ca0daab1bbf69bb81c6` |
| Clean-source Quint baseline | `89464443d81ec57fd109df1e`, seed `0x1127719f`, exit 100; retained log SHA-256 `05ae41b52ec22e25eb8a7bf752d49ab2b22663e3bad0f8e160dcbb1cd710771d` |
| Authored control-plane source | `c8df77ed2880a3a56b249f6172ab97280155e07149d036c9f1bac4b5afadc937` |

The R5-41 source still differs from base `822b53395` only by its original
pending-marker deletion. No body, helper call, assertion or expected outcome
has yet been remediated.

### Native reachability and exact blocking state

The failure is reproduced through the real production entrypoint and owner
path; it is not an imagined schedule:

1. The existing GuestKit runner completes the retained 111-body boundary.
2. The CLI walking skeleton passes, and the first control-plane body passes in
   the same native command.
3. The next exact identity,
   `a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted`,
   starts inside the disposable control-plane outer guest.
4. `Node::deploy_job` sends `SubmitSpecInput::Job(JobSpecInput { .. })` through
   the public HTTPS workload handler with the existing VM image and the
   long-running `hold` application.
5. The first allocation reaches Running, its session and held flow are
   observed, and the test kills its real Cloud Hypervisor process with
   `SIGKILL`.
6. Production writes the only allocation row as `Failed` with
   `WorkloadCrashedImmediately { signal: Some(9) }`.
7. `running_alloc_except` then waits the full 120-second bound for a different
   Running allocation. None is placed, so the test remains blocked until the
   outer GuestPeer execution bound reports the timeout.

The runner's `no reply to EXEC` message is downstream of the in-guest Rust
test still polling `running_alloc_except`; it is not a GuestKit transport or
command-start failure. The prior body passed through the same outer-guest
boundary, and the failing body reached the public rows that expose the exact
terminal predecessor.

### Accepted outcome reconciliation

There is no DESIGN contradiction requiring a user choice.

R5-41 is stated in workload-kind-neutral terms: an allocation's VM is killed,
the CID's next allocation is placed and boots, the dead launch's buffered
connections are rejected as stale, the new launch's session/flows/READY are
accepted, and the next launch draws a new token. Its Tier-3 assignment is the
kill-and-reuse shape through production `serve` + deploy. R5-41 does not say
that the killed allocation is a `Job`, does not weaken Job terminality and
does not claim every workload kind restarts.

The accepted workload-kind contract independently says the opposite for a
Job:

- `WorkloadLifecycle` finalizes a Job on its first natural clean or crashed
  exit and emits no restart action;
- `restart_after_failure(WorkloadKind::Job, ..)` returns `false` for that
  natural exit; and
- the contract describes a Job as run once, until it exits.

This is current, executed behavior rather than prose-only inference. Metal run
`8e2dc8fe0ae5242845a89b85` passes
`attempt_failed_first_attempt_of_a_job_will_not_restart`. Seeded run
`21b992e20519e96eeb2b7d58` passes
`restart_after_failure_agrees_with_one_reconcile`, including workload-kind
coverage. Changing production so the killed Job restarts would violate those
accepted outcomes and is forbidden.

The R5-41 replacement precondition therefore requires the existing
long-running `Service` kind. Selecting `Job` was a private acceptance-fixture
choice, not an interface or outcome pinned by DESIGN. Correcting that choice
is DISTILL-owned test support under the workspace's explicit fixture-ownership
rule.

The clean-source Quint failure at seed `0x1127719f` is preserved as an
independent baseline counterexample. It concerns the existing forward-pair
guest projection and reproduces without the current STEP 14-01 edits. The
R5-41 workload-kind correction neither fixes nor waives it, changes no model,
and draws no production requirement from it.

## Finding F-02 — R5-41 submits a terminal Job while requiring Service replacement

**Severity:** blocker  
**Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — DISTILL fixture correction  
**Status:** open

### Evidence

- Feature-delta R5-41 requires a next allocation and next VM launch after the
  first VMM is killed, while naming no workload kind.
- The authored body and its rustdoc preserve that same generic outcome: “the
  workload's next allocation is placed and its VM boots.”
- `Node::deploy_job` constructs `JobSpecInput` and submits
  `SubmitSpecInput::Job` through the public API.
- The body calls that helper before it waits for the replacement.
- Production `workload_lifecycle.rs` deliberately finalizes the crashed Job
  and `restart_after_failure` explicitly refuses its natural-exit restart.
- Native run `d42014b2202f9df175307503` observes exactly that reachable state:
  one failed Job row and no successor for 120 seconds.

### Exact authorized correction

Keep the R5-41 scenario, stimulus and all owner assertions unchanged. Change
only its private workload-kind fixture:

1. Add or reuse a private `Node` helper that submits a
   `SubmitSpecInput::Service(ServiceSpecInput { .. })` through the same public
   HTTPS handler.
2. Preserve replicas `1`, resources, VM command, exact `hold` arguments,
   kernel, rootfs, workload id and remote peer from the current R5-41 call.
3. Because the accepted Service ingress requires at least one listener,
   declare the existing non-reserved TCP `SERVICE_PORT` in that Service spec.
   Keep startup, readiness and liveness probe arrays empty: R5-41 observes
   launch attribution and the held outbound flow, not listener health.
4. Change only the R5-41 body's submission call from the Job helper to that
   Service helper. Do not change the other Job fixtures: the inventory,
   probe/relay-client and CLI walking-skeleton bodies use Job semantics for
   their own outcomes and do not require crash replacement.
5. Retain every existing R5-41 assertion: first session/flow present; real VMM
   kill; distinct next allocation; new launch token; new session on CID reuse;
   no dead-launch unread connection; exactly one live pair; current forwarder
   generation; and empty owner state after stop.

The required listener is admission shape only. The guest application remains
the existing `hold` command so the exact outbound flow under attribution is
unchanged; no listener/probe outcome is added to the scenario. Production may
install its ordinary Service listener-derived state, but the test adds no
rule, bind, map, address, port seam or effect that production would not derive
from the submitted Service itself.

Do not change the Job restart policy, special-case this workload id, alter
`running_alloc_except`, manufacture a row, broaden the timeout, or accept the
failed predecessor as the “next launch.” Those approaches would weaken or
bypass R5-41 rather than correct its precondition.

No new public API, production mechanism, scenario, test binary, lane, model,
error variant or DES event is needed. The helper and one call site are private
DISTILL fixture HOW.

### Required remediation evidence

- the R5-41 source diff changes only the private Service submission fixture
  and its one call; every stimulus and assertion remains byte-identical;
- scoped format/check/Clippy for the kvm-gated control-plane integration
  target passes on Metal;
- the exact existing GuestKit runner reaches and passes the corrected R5-41
  identity with one successful libtest summary and zero runtime-skip
  diagnostics, after retaining the 111-body, CLI and preceding control-plane
  evidence;
- the new allocation is distinct, the owner assertions remain non-vacuous and
  cleanup still empties the owner; and
- a read-only physical-host audit retains zero live Cloud Hypervisor
  processes, physical-host nft tables and STEP 14-01 workdirs.

Any later failure in a different exact body remains separately classified; it
does not authorize weakening this correction or silently claiming the entire
six-body STEP 14-01 boundary GREEN.

### Iteration 4 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | R5-41 retains dead-launch rejection, live-successor acceptance and teardown complements. |
| GWT / one behavior | 9/10 | The body still expresses one kill-and-reuse launch-attribution behavior. |
| Domain language | 10/10 | Allocation, launch, CID, session, token, flow, pair and generation remain exact. |
| Coverage completeness | 4/10 | The required successor state is unreachable under the authored Job precondition. |
| Walking-skeleton user value | N/A | This is the focused R5-41 through-serve complement, not P6-S13. |
| Priority validation | 10/10 | Native evidence isolates the exact fixture-kind mismatch; no production restart change is needed. |
| Observable behavior | 9/10 | Public rows and owner observations are correct, but the wrong kind prevents the successor assertions from running. |
| Traceability | 10/10 | The body maps directly to R5-41 / P-37 / ATTR-TOKEN and the production restart contract. |
| Production boundary | 10/10 | HTTPS submit, real VMM kill, public allocation rows and production owner are all reached. |

Coverage completeness remains below the approval threshold and one blocker is
open.

### Iteration 4 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | The failure runs through public HTTPS submit and the composed production owners. |
| CM-B — domain abstraction | PASS | The assertions remain in R5-41's launch-attribution vocabulary. |
| CM-C — complete user journey | **FAIL at fixture precondition** | Job terminality prevents the required next launch and the second half never executes. |
| CM-D — fixture isolation | **FAIL** | The private helper selects a workload kind whose accepted lifecycle contradicts the scenario's replacement precondition. |
| CM-E — universe-bound assertion | PASS / layer N/A | This is a layer-4+ real-kernel integration body with direct observations. |
| CM-F — layer-dependent PBT | PASS | The body remains an explicit native example. |
| CM-G — two-tier rich journey | N/A | No new journey is introduced. |
| CM-H — explicit integration paths | FAIL pending correction | The named path is real and non-vacuous but cannot reach its accepted successor outcome. |

The C1–C7 taxonomy gains no new scenario class. This is a precondition-shape
error in an existing R5-41 cell, so the canonical 15-item checklist's scenario
inventory remains unchanged while its executable-completeness gate fails.

## Iteration 4 verdict

**NEEDS REVISION.** The native timeout is the exact consequence of an authored
fixture selecting `Job` for a scenario that requires replacement. Production
correctly treats the crashed Job as terminal, as the source, ordinary Metal
cell and seeded restart-SSOT invariant all confirm. R5-41 does not mandate Job
replacement; it is workload-kind neutral and requires a next allocation.

DISTILL is authorized to submit the same long-running VM workload as a
Service, using the existing required listener shape and no probes, while
preserving the VMM kill, held-flow stimulus and every owner/token/session/pair
assertion. No DESIGN decision or user approval is needed, and Job terminality
must remain unchanged.

Return the bounded fixture correction and exact native evidence to this same
reviewer. This verdict does not prescribe a production change and does not
waive the independent clean-source Quint baseline failure.

# NEEDS REVISION

## Iteration 3 re-review — accepted-cleanup console observation

Iteration 3 reviews only the newly bounded, test-local console-observation
correction in `crates/overdrive-cli/tests/integration/vsock_support.rs` and
the refreshed source/native/cleanup evidence. It does not re-review or
pre-approve the crafter's production implementation, and it does not convert
the later control-plane timeout into a finding about this correction.

The reviewer appended only this native-Markdown iteration. No source, test
body, handoff, DES event, execution log or commit was changed.

### Iteration 3 artifact identity

| Artifact | SHA-256 at re-review |
| --- | --- |
| Current handoff | `97be5d967277b9ce2a2e2c77ff840ca4f205895b610b45c9e17f71c47a001ffa` |
| `overdrive-cli/tests/integration/vsock_support.rs` | `8f5e46ce02d32820917e569a0a70c4d352435aabb308f84d75f274edd608ebba` |
| Walking-skeleton body | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| Control-plane bodies | `c8df77ed2880a3a56b249f6172ab97280155e07149d036c9f1bac4b5afadc937` |
| Production VM-driver cleanup source | `d7fa0708a281d5bfe7223255bb3f828a8e44adae2f642702fc8e0274892f0194` |
| Crafter-owned `overdrive-init/src/main.rs` | `35d5b02d6fa1f119a05806aad4ab665bbfeb6c2becd76ff456a49676f9b37941` |
| Crafter-owned `overdrive-init/src/guest_flow/owner.rs` | `6c6f0e9e02a56ddc7ff91244dcbd4e24820c696fb9c8656d969f4dcb26e02932` |

The console-support Git blob is
`83462af10276c0882bc036454a0188bd1398ca09`. The walking-skeleton and
control-plane source identities remain unchanged from iteration 2. A diff of
the six authored STEP 14-01 bodies against base `822b53395` still contains
only the six original pending-marker deletions; no fixture call, stimulus,
assertion, expected value or cleanup complement changed.

### Why the live path cannot satisfy the unchanged post-terminal assertion

The handoff's cleanup premise matches current production source and its
accepted ordering:

1. `run_exit_watcher` drains the guest report and awaits the
   Running-confirmed gate.
2. The watcher claims the originating allocation's unique `LiveVm` through
   `try_begin_ending`.
3. `cleanup_driver_artifacts` awaits cgroup kill, workload-scope removal,
   removal of the complete `VmRunDir`, and rootfs clone/index cleanup.
4. Only after those cleanup calls finish does the watcher construct and send
   `ExitEvent`.
5. The control-plane observer derives the terminal allocation row from that
   `ExitEvent`.

`VmRunDir` owns `console.log`, so an assertion that first waits for the
`Terminated` row and then opens the live path necessarily runs after the
accepted cleanup attempt. Production `vm_driver.rs` has no source diff in the
candidate. Retaining `VmRunDir` until after terminal observation, copying the
console into a production store, or adding a production console hook would
change ownership and weaken the cleanup order. None is necessary for this
existing acceptance oracle.

### ConsoleCapture source review

The correction is a bounded test-support observer:

- `Lane::boot` starts `ConsoleCapture` after production `serve` and the
  workload image are prepared, but before the test deploys any workload.
- Its named test thread only reads existing allocation directories and
  `console.log` files under the existing `VM_RUN_ROOT`. It creates, modifies,
  renames, holds open or restores no production artifact.
- It retains the longest bytes actually read per allocation in a process-local
  `BTreeMap`. It does not parse, add or substitute the expected TCP/UDP marker
  strings.
- The map is cleared when the serial lane starts, before deployment. The
  STEP 14-01 walking skeleton has one allocation, so the retained entry cannot
  come from another launch in that journey.
- `console_of` continues to prefer the live production file. It consults the
  retained snapshot only when the live read is absent or empty after cleanup.
  If neither surface contains the guest output, it returns an empty string and
  the unchanged assertions fail.
- `ConsoleCapture::drop` sets its stop flag and joins the observer thread. Its
  lifetime is owned by `Lane`; it does not outlive the test fixture.

The support is private to the kvm-gated CLI integration module. It adds no
production API, persistence path, observation row, hook, port, adapter,
configuration, lane, selector, executor or scenario. It also leaves the
production cleanup owner and ordering untouched.

The static cache is shared with later `vsock_fault_matrix` and
`vsock_density` bodies through the existing support module, but every current
consumer is in the same `#[serial(cgroup)]` group and creates one `Lane` per
test. No concurrent current caller can clear another lane's snapshot. This
review makes no claim about a future non-serial or same-allocation-id reuse
shape; none is part of STEP 14-01 and no such failure was reproduced.

### Unchanged walking-skeleton oracle

The walking-skeleton body remains ordered as follows:

1. deploy one VM Job through production `serve` and the public deploy
   handler;
2. require a terminal row and then require `Terminated`;
3. require exactly one TCP request and one UDP datagram at the independent
   remote peer, with the expected tag;
4. require the remote peer to observe the node-side address for both
   protocols;
5. require the captured real guest console to contain both exact guest-emitted
   lines, `round-trip: tcp ok` and `round-trip: udp ok`; and
6. require `hypervisor_of(&alloc).is_none()` after termination.

No assertion was removed, reordered or relaxed. `ConsoleCapture` cannot make
the remote peer, allocation state or process-table assertions pass. It also
cannot synthesize the two console lines: the cache contains only bytes read
from the production VMM's existing console file. A successful body therefore
still proves byte-exact TCP and UDP completion, the node-address observation,
the guest's own two success reports, and VMM disappearance.

### Refreshed build and native evidence

All three final receipts carry the same source digest
`7eda43f46739334fd4ee0867bcea2c11b3f8194f03c31a740de1c5ef56a8328d`.

| Evidence | Independent result |
| --- | --- |
| Scoped source gate | `83e0f2f61de179d79994b496`, exit 0, `command_returned` |
| Source-gate scope | `cargo fmt --check`; CLI kvm-gated integration `cargo check`; same target Clippy with `-D warnings` |
| Source-gate output | 304 bytes; SHA-256 `9c1a7b04d1ad76b5b25a9ea18a4b94d7db6aff952c04b3fde5f0bae7a2999d42` |
| Native GuestKit run | `d42014b2202f9df175307503`, exit 101, `command_returned` |
| Retained pre-STEP-14 boundary | 101 successful summaries declaring exactly 111 passed, 0 failed, 0 ignored |
| STEP 14-01 CLI walking skeleton | Exact identity passed once: `1 passed; 0 failed; 0 ignored`, 3.71 s |
| First control-plane body | Exact identity passed once: `1 passed; 0 failed; 0 ignored`, 8.22 s |
| Successful-summary total before later failure | 103 summaries declaring 113 passed, 0 failed, 0 ignored |
| Recognized runtime skips | 0 |
| Native output | 446,641 bytes; SHA-256 `6a3f64583c1e767313edf5c0636efa27c5e26df2133e7aff19f91dab9650fd5b` |
| Native terminal | SHA-256 `49365c9dd5cbe44ade0578a7d5bafe2cc52e5d9528f6776ddeaea495186c7e26` |
| Physical-host cleanup | `fdc735fa7064532ba2d65e49`, exit 0, `command_returned` |
| Cleanup output | SHA-256 `d974c82513bb82ed89667af941343be1f51a6b70f54f0638c3f07836b9af95db` |
| Cleanup observations | `live_cloud_hypervisor=0`; no physical-host nft table; no STEP 14-01 workdir |
| `git diff --check` | PASS |

The CLI libtest pass is load-bearing evidence because every assertion in the
unchanged body executes before the test can report `ok`. Libtest does not
print the two console strings on success, so their absence from the outer raw
log is expected; the exact source plus the successful result establishes that
the retained bytes contained both real guest-emitted markers. The independent
remote-peer and process-table checks execute in the same body after the
terminal-row assertion and before shutdown.

The STEP 14-01 CLI outer guest reaches `GuestRun::finish`, prints its
completion header, powers off, and is reaped. The subsequent control-plane
outer guest does not finish normally because of the later timeout, but its
drop path and the read-only physical-host audit show no surviving VMM, nft
table or STEP 14-01 work directory.

### Later control-plane timeout

After the walking skeleton and the first control-plane body passed, the next
independently-authored control-plane body timed out waiting for a replacement
allocation after intentionally killing its first VMM. Its last row remained
`WorkloadCrashedImmediately { signal: Some(9) }`, and the outer executor
reported no reply within its bound.

This is an honest failure in the same full native run, so the receipt cannot be
described as a complete six-body STEP 14-01 GREEN. It is also independent of
the console correction:

- `ConsoleCapture` is compiled into the CLI integration binary only;
- the failing body is in the separate control-plane integration binary;
- the CLI outer guest had already completed and been reaped before the
  control-plane guest ran; and
- the failing assertion concerns allocation replacement after VMM kill, not
  console observation.

This DISTILL review therefore neither suppresses the timeout nor treats it as
evidence against the bounded console observer. Its root cause and any
production disposition remain outside this fixture-correction review. No
production remediation is prescribed here, and this approval does not mark
the overall STEP 14-01 delivery GREEN or reviewed.

### Finding and remediation dispositions

#### Iteration 1 F-01 — workload-image PID-1 entrypoint

**Disposition: remains CLOSED.** The iteration-2 evidence and source are
unchanged by the console correction.

#### Iteration 2 semantic RED handoff

**Disposition: CONSUMED by the crafter; outside this review.** The current
native run demonstrates that the walking skeleton now passes the production
path. This DISTILL review assesses only the observer needed to retain its
unchanged console oracle across accepted cleanup.

#### Iteration 3 console-observation correction

**Disposition: CLOSED / APPROVED.** The observer records only bytes exposed by
the existing production console before its owner removes `VmRunDir`, preserves
all original assertions and cleanup, and is proved by the exact native
walking-skeleton pass.

#### New iteration 3 findings

None.

### Iteration 3 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | The correction preserves the walking skeleton and the independently selected owner/fault complements. |
| GWT / one behavior | 10/10 | The unchanged body retains one deploy-and-round-trip journey with one terminal outcome. |
| Domain language | 10/10 | Workload, round trip, remote peer, node address, console report and VMM disappearance remain precise. |
| Coverage completeness | 10/10 for correction | Every console-dependent assertion in the walking skeleton executes and passes; the later control-plane timeout is recorded separately. |
| Walking-skeleton user value | 10/10 | The operator-driven TCP/UDP journey and cleanup outcome remain unchanged. |
| Priority validation | 10/10 | The change addresses only the reproduced post-cleanup observation mismatch without altering production lifecycle. |
| Observable behavior | 10/10 | Cached bytes originate from the real VMM console; remote-peer, allocation and process outcomes remain independent. |
| Traceability | 10/10 | The exact body remains P6-S13 / V-6 and the correction stays in its existing shared support module. |
| Walking-skeleton boundary | 10/10 | Production serve, deploy, real VM, TCP/UDP, cleanup and VMM disappearance are all exercised with no override seam. |

All dimensions meet the approval threshold and no blocker remains within the
bounded correction.

### Iteration 3 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | The test still enters production `serve` and deploy; the observer is read-only test support. |
| CM-B — domain abstraction | PASS | No authored assertion or domain vocabulary changed. |
| CM-C — complete user journey | PASS | TCP, UDP, source identity, guest reports, terminal state and VMM cleanup all remain required. |
| CM-D — fixture isolation | PASS | The observer is process-local, test-owned and leaves production cleanup/ownership unchanged. |
| CM-E — universe-bound assertion | PASS / layer N/A | This is a layer-4+ real-kernel integration body with direct observable assertions. |
| CM-F — layer-dependent PBT | PASS | The body remains one explicit native example. |
| CM-G — two-tier rich journey | N/A | No new journey or Tier-B surface is introduced. |
| CM-H — explicit integration paths | PASS | The exact native walking skeleton passes non-vacuously; the later independent failure remains visible. |

The correction adds no scenario, input partition, state transition, mode,
semantic error or concurrency claim, so the C1–C7 taxonomy and canonical
15-item checklist remain unchanged and complete under their N/A rule. Its
delta-completeness question is observation honesty: the data must come from
the real console, every original oracle must remain, production cleanup must
remain unchanged, and missing data must still fail. All four conditions pass.

## Iteration 3 verdict

**APPROVED.** `ConsoleCapture` is an honest test-local read observer for an
ephemeral production artifact whose accepted owner removes it before terminal
state becomes observable. It caches only bytes actually read from the real VMM
console, prefers the live file while present, supplies no expected content,
and stops with the existing `Lane` fixture. Production cleanup still removes
the complete `VmRunDir` before `ExitEvent`; no production persistence, API,
hook, lane, scenario or owner was added.

The authored walking-skeleton body remains marker-only changed and retains its
TCP, UDP, node-address, exact console-line and no-live-VMM assertions. Native
run `d42014b2202f9df175307503` passes that exact body and the following
control-plane inventory body after the unchanged 111-body boundary. Cleanup
run `fdc735fa7064532ba2d65e49` finds no physical-host VMM, nft table or
STEP 14-01 workdir.

The later replacement-allocation timeout is retained as real evidence but is
outside this correction and this verdict. This approval closes only the
DISTILL console-observation support; it does not approve or complete the
overall STEP 14-01 DELIVER implementation.

# APPROVED

## Current terminal verdict after iteration 4

The iteration-3 approval above remains the completed disposition of the
console-observation correction only. Iteration 4 opened F-02 for the distinct
R5-41 workload-kind fixture mismatch recorded earlier in this artifact. The
review's current controlling metadata and verdict are therefore **NEEDS
REVISION**, with one blocker open.

# NEEDS REVISION

## Iteration 5 re-review — F-02 Service fixture remediation

Iteration 5 reviews only the exact iteration-4 F-02 correction and the
refreshed full native evidence. The reviewer appended this Markdown iteration
without changing source, handoff, DES, execution log or commit state.

### Iteration 5 artifact and receipt identity

| Artifact | Identity at re-review |
| --- | --- |
| Current handoff | SHA-256 `499f27afcf682f37f93160bd64cf1d322d5d5ff66d827894d777eefb1e919308` |
| `guest_flow_through_serve.rs` | SHA-256 `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18`; Git blob `e1060ace599d4d740cb22ff3ee128068754d28fc` |
| Walking-skeleton body | SHA-256 `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| Console support | SHA-256 `8f5e46ce02d32820917e569a0a70c4d352435aabb308f84d75f274edd608ebba` |
| Workload lifecycle | SHA-256 `dd925c45e42b44a83a3de46e0aef4a00f2223c17d7b9c1ef386a30226bd4996c` |
| Guest init | SHA-256 `35d5b02d6fa1f119a05806aad4ab665bbfeb6c2becd76ff456a49676f9b37941` |
| Guest owner | SHA-256 `6c6f0e9e02a56ddc7ff91244dcbd4e24820c696fb9c8656d969f4dcb26e02932` |

All final receipts carry source digest
`d3fe8c43abf2447f87621f8d6b96ea863525d95c27f7e5784190ee36b7fbe6ce`:

| Receipt | Result and retained identity |
| --- | --- |
| Scoped source gate `41dbac5813ccba60893161ad` | Exit 0, `command_returned`; output 1,539 bytes, SHA-256 `492a535214e853805686b7db6ff6d1c26dde91496c81a44ea7a3e6d4ff1df6dd`; terminal SHA-256 `e65ab97152d01187b307bf0391a36ac89f52099afecfddfb8291267f0dc1b380` |
| Full GuestKit run `410a62758dd8e62f9f9d6418` | Exit 0, `command_returned`; output 457,157 bytes, SHA-256 `df6b8f0439e8908dd7bd6862967732906245276d0809f05cdb1fff5eb9a273f4`; terminal SHA-256 `22e48c84b552885091dce2c89793901e770027dbaa8ea6f678a1b55eef586e12` |
| Physical-host audit `be0ea50bafad180437fa1117` | Exit 0, `command_returned`; output SHA-256 `44d821901e4ed659775951f3cfe9301e0fce9229a81cd32c00529daa9983b69a`; terminal SHA-256 `3405eae28febe848f003a7e39518cd2958403c1407d79fdf2faa9ef2e780f6f8` |

### Exact source correction

The source matches the authorized DISTILL correction:

- `Node::deploy_hold_service` is private test support in the existing
  control-plane integration module.
- It submits `SubmitSpecInput::Service` through the same public HTTPS handler.
- It preserves workload id, replicas `1`, CPU and memory resources, the
  existing guest-peer VM command, the caller-supplied exact arguments, kernel
  and rootfs.
- It declares the existing non-reserved TCP `SERVICE_PORT` listener required
  by Service admission and declares no startup, readiness or liveness probes.
- The R5-41 body changes only one call, from `deploy_job` to
  `deploy_hold_service`. Its `hold` command, remote peer, first-allocation
  observation, real VMM `SIGKILL`, replacement wait and every subsequent
  assertion remain byte-identical.
- `deploy_hold_service` has exactly one caller. The other STEP 14-01 bodies
  retain their original Job or listening-Service fixture choices; existing
  `deploy_job` calls remain at their original sites.

The correction changes a private precondition only. It does not add a public
method, wire field, production branch, restart exception, listener mechanism,
probe outcome, model action, scenario, test binary, lane, selector or
executor. Production Job no-restart and Service restart behavior are source
unchanged. The CLI walking skeleton remains a run-to-completion Job, and no
failed Job is made restartable.

### R5-41 outcome and oracle integrity

The corrected workload kind now agrees with both accepted contracts:

- R5-41 requires the killed launch's replacement, new token/session/flows and
  stale-launch exclusion without pinning a workload kind.
- The existing long-running Service lifecycle is the platform shape that
  places a successor after a crash, while a Job remains terminal on its first
  clean or crashed exit.

No R5-41 outcome was removed or relaxed. After the helper call, the body still
requires:

1. a first Running allocation whose launch holds one live session and one
   paired flow;
2. a successful real `SIGKILL` of that allocation's Cloud Hypervisor process;
3. a different Running allocation for the same workload;
4. the dead launch absent and the successor session plus exactly one paired
   flow present;
5. a different launch token, and a different session when the CID is reused;
6. no unread dead-launch connection;
7. exactly one pair, each successor flow paired in the current forwarder
   generation; and
8. empty owner CID state after the Service is stopped.

The required Service listener is admission shape only. The guest process still
runs the exact outbound `hold` application, so the pair/session/token behavior
under test is unchanged. Empty probe arrays ensure no listener-health gate is
substituted for R5-41's owner observations.

### Full native result and non-vacuity

Independent accounting of `410a62758dd8e62f9f9d6418` found 107 successful
libtest summaries declaring exactly 117 passed bodies, zero failed and zero
ignored. The runner's line-oriented detector found zero recognized runtime
SKIP diagnostics. All five disposable guest lifecycles completed: the three
retained `cgroup-default` guests, the CLI guest and the control-plane guest.

The exact STEP 14-01 sequence is complete:

| Exact body | Result |
| --- | --- |
| CLI walking skeleton | PASS once, 3.70 s |
| Forwarder inventory counts held flows | PASS once, 9.26 s |
| R5-41 killed launch is replaced and accepted | PASS once, 5.60 s |
| Probe and relay watches through serve | PASS once, 14.02 s |
| Dropped probe stream aborts at guest | PASS once, 9.44 s |
| VMM exit under held relay resets client | PASS once, 7.47 s |

R5-41's pass is non-vacuous. The test cannot report `ok` before the distinct
successor is Running; every token/session/unread/pair/generation assertion and
the final empty-owner assertion executes afterward. The helper neither seeds
an observation row nor injects an owner state. The replacement and owner facts
come from the production Service lifecycle, VM driver, guest-flow owner and
public observation surface.

The aggregate reconciles as `111 + 1 CLI + 5 control-plane = 117`, exactly the
runner's static manifest. No target is hidden by an ignore, filter mismatch or
runtime bailout.

### Source gate and cleanup

Scoped Metal run `41dbac5813ccba60893161ad` completed:

- `cargo fmt --all -- --check`;
- `cargo check -p overdrive-control-plane --test integration --features
  integration-tests,kvm-tests`; and
- the same integration target under Clippy with `-D warnings`.

Read-only audit `be0ea50bafad180437fa1117` records:

```text
live_cloud_hypervisor=0
nft_begin
nft_end
workdirs_begin
workdirs_end
```

Thus the complete successful runner left no physical-host Cloud Hypervisor
process, nft table or STEP 14-01 work directory. Kernel-sensitive and nested-VM
effects remained inside the existing disposable GuestKit boundary.
`git diff --check` also passes.

### Finding and remediation dispositions

#### F-02 — R5-41 Job/Service fixture mismatch

**Disposition: CLOSED.**

The private helper now submits the same long-running VM workload as a Service,
the R5-41 body changes only its helper call, every stimulus and assertion is
preserved, and the exact identity passes through the full native production
path. Job terminality and Service restart behavior remain unchanged.

#### Earlier findings

Iteration-1 F-01, the iteration-3 console-observation correction and all their
evidence remain closed and unchanged.

#### New iteration 5 findings

None.

### Iteration 5 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | R5-41 proves both dead-launch exclusion and live-successor acceptance, then cleanup. |
| GWT / one behavior | 10/10 | One VMM kill drives one complete launch-replacement attribution journey. |
| Domain language | 10/10 | Allocation, launch, CID, session, token, flow, pair and generation remain exact. |
| Coverage completeness | 10/10 | The corrected precondition reaches and passes every successor assertion; all six STEP 14-01 bodies pass. |
| Walking-skeleton user value | 10/10 N/A for R5-41 | The separate P6-S13 walking skeleton also passes unchanged in the same run. |
| Priority validation | 10/10 | The correction is limited to the proven private kind mismatch and preserves Job terminality. |
| Observable behavior | 10/10 | Public rows and owner observations prove the distinct successor and stale-launch exclusion. |
| Traceability | 10/10 | The body remains R5-41 / P-37 / ATTR-TOKEN through serve + deploy. |
| Production boundary | 10/10 | Public HTTPS submit, real VMM kill, production replacement and owner observation all execute. |

Every dimension meets the approval threshold and no blocker remains.

### Iteration 5 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | The corrected Service still enters through public HTTPS submit and production composition. |
| CM-B — domain abstraction | PASS | No owner assertion or scenario vocabulary changed. |
| CM-C — complete user journey | PASS | First launch, VMM death, successor, attribution checks and teardown all execute. |
| CM-D — fixture isolation | PASS | Only the private workload-kind fixture and one call changed; production lifecycle code is untouched. |
| CM-E — universe-bound assertion | PASS / layer N/A | This is a layer-4+ real-kernel integration body with direct observations. |
| CM-F — layer-dependent PBT | PASS | The Tier-3 body remains one explicit native example. |
| CM-G — two-tier rich journey | N/A | No new journey or in-memory tier was introduced. |
| CM-H — explicit integration paths | PASS | R5-41 and all other STEP 14-01 exact paths pass with zero ignore or runtime skip. |

The correction adds no scenario class or contract claim, so the C1–C7
taxonomy and canonical 15-item inventory remain unchanged. Executable
completeness now passes: the intended replacement state is reachable under the
correct existing Service lifecycle, every original oracle runs, and the full
117-body boundary closes cleanly.

## Iteration 5 verdict

**APPROVED.** F-02 is closed by the exact authorized DISTILL fixture change.
The R5-41 workload is submitted as a Service with the existing required TCP
listener and no probes, while its long-running `hold` command, real VMM kill
and every launch-attribution assertion remain unchanged. No Job restart
contract, production API, mechanism, scenario or lane changed.

The scoped source gate is green, the existing runner proves all 117 bodies
with zero failure, ignore or recognized runtime skip, the exact R5-41 identity
passes non-vacuously, and the physical-host audit is clean. There are no open
DISTILL findings.

The approved support and authored acceptance package may return to the STEP
14-01 crafter for its remaining DES/commit and dedicated implementation-review
gates. This verdict does not replace or pre-approve that DELIVER review.

# APPROVED
