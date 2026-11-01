# DISTILL review — STEP 14-01 existing GuestKit native support

## Review metadata

| Field | Value |
| --- | --- |
| Review ID | `STEP14-01-DISTILL-NATIVE-SUPPORT-REVIEW-01` |
| Feature / roadmap step | `netns-density-295` / `14-01` |
| Review dates | 2026-10-29–30 |
| Reviewer | Fresh independent DISTILL source and evidence reviewer |
| Iterations | 1–9 |
| Implementation base | `822b533950fcfeb51f72444b574fe1f05b93cb97` |
| Scope | Existing native support for the six already-authored STEP 14-01 bodies, including nested GuestKit confinement, exact selection and accounting, compiler-only fixture fallout, V-6 acceptance completion in the existing P6-S13 body, and durable native evidence |
| Design authority | Feature-delta V-6 and V-LEGB-SERVE; roadmap and dispatch STEP 14-01; current workspace `AGENTS.md` test-support ownership rule |
| Verdict | **APPROVED; F-05 CLOSED IN SOURCE, PRODUCT V-6 REMAINS SEMANTIC RED** |
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

## Iteration 6 review — V-6 acceptance-coverage proposal

Iteration 6 reviews the audit and source-change proposal in
`deliver/distill-14-01-v6-coverage.md` against the exact accepted V-6 and
V-LEGB-SERVE contracts and the full post-commit DELIVER review. It assesses
the proposed acceptance shape only. No executable source, DESIGN artifact,
DES event, execution log or commit was changed by this reviewer.

### Reviewed identity

| Artifact | SHA-256 |
| --- | --- |
| V-6 coverage audit/proposal | `09ceaf18b20f46206a67090fb2e19318bf4db83d577db68aa1b4ff2360fcaf5f` |
| DELIVER review iteration 1 | `f19e0fa48ece9ae8ad289b3dcae3676bb2ab164a532e76386209a1dd648122a7` |
| Accepted feature delta | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| Accepted roadmap | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| STEP 14-01 dispatch | `ef672d8849ff68fde6a1f4d2050e09957373030eeb4b580d7c1999c442d593f7` |
| Existing P6-S13 body | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| Existing CLI support | `8f5e46ce02d32820917e569a0a70c4d352435aabb308f84d75f274edd608ebba` |
| Existing through-serve complements | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` |
| Existing guest binary / shared protocol | `3d8c8d70da22f64f2c48c4ffd0b14efae503704e3ffbc4424328ee2872d69f39` / `9539a3388bc2004ab2e19d4b81878851307fc4c35e2a1f2689cbb1a3cc7e591d` |
| Source HEAD audited by proposal | `c9f2f6d6975b236fa4a049093550d3ea8dcf4242` |

### Audit accuracy and strengths

The audit's central conclusion is correct. Native receipt
`410a62758dd8e62f9f9d6418` honestly proves 117 bodies, but a green body proves
only the stimuli and assertions it executes. The current exact inventory does
not discharge the cumulative V-6 / V-LEGB-SERVE contract.

The traceability table accurately separates current evidence:

- current P6-S13 proves off-node non-empty TCP and UDP, the peer's node-side
  source address, guest console success and VMM disappearance;
- current through-serve complements prove the TCP probe and route-less
  same-node Relay/owner paths; and
- current selected bodies do not drive an empty datagram, gateway:53 DNS,
  mesh-name resolution, frontend/VIP TCP, Service-VIP UDP, remote inbound,
  HTTP probe state, with-default V-LEGB socket/uplink evidence, or the
  concurrent `IP_BIND_ADDRESS_NO_PORT` comparison.

The proposal correctly refuses to aggregate lower-level sim, adapter and
real-kernel mechanism tests into a false production-composition claim. Those
tests remain valuable supporting evidence but do not replace the required
`overdrive serve` + public deploy journey.

It is also correct to keep the executable inventory fixed at one CLI P6-S13
body plus the five existing control-plane complements. Completing the
already-authored P6-S13 scenario is a DISTILL correction to an accepted
scenario, not a seventh test, new scenario or outcome expansion. No additional
user approval is needed for private helper structure that stays inside this
contract.

The proposed safety boundaries are directionally sound:

- keep production `serve` + deploy and public describe/probe surfaces;
- use the real DNS, VIP, mTLS, probe and guest-flow owners rather than direct
  adapter calls;
- use a credential chaining to the same production workload CA for remote
  inbound and preserve peer verification;
- keep the production `IP_BIND_ADDRESS_NO_PORT` path fixed while making the
  unset comparison test-local;
- confine routes, captures, namespaces, modules and VMs to the disposable
  outer GuestKit guest; and
- retain the same six identities and 117-body runner accounting.

Extending the existing guest application vocabulary and private CLI helpers to
drive these already-accepted actions is test-support HOW. It is not a public
API, production override, new lane or new product mechanism.

### Contract matrix

| Accepted claim | Proposal disposition | Review result |
| --- | --- | --- |
| Empty UDP request and reply | Same connected guest socket; real zero-length send/receive; peer records source; existing non-empty exchange retained | Correct and concrete |
| VM Service resolves mesh peer by name through gateway:53 | Public production Service/DNS path proposed | **Actor kind remains ambiguous; see F-03** |
| Mesh frontend/VIP TCP with mTLS through leg F | Public VIP plus external TLS/no-cleartext oracle proposed | Correct boundary; oracle must be made fail-closed per F-04 |
| Issued Service VIP UDP through `connect4` | Public issued VIP and deployed UDP listener proposed | Correct boundary |
| Remote inbound leg C → leg S → `GuestStreamDial` | Route-disjoint real mTLS client with same workload CA and byte exchange proposed | Correct boundary, subject to exact credential/testability proof during implementation |
| TCP and HTTP probes follow application state | Public probe-result surface and healthy/unhealthy transition proposed | Correct boundary; expected transitions must be pinned per F-04 |
| Route-less same-node leg B | Existing through-serve complement retained | Correct |
| With-default same-node leg B; leg-C peer; zero uplink | Disposable default/uplink, socket observation and AF_PACKET capture proposed | Correct environment shape; capture oracle is incomplete per F-04 |
| Concurrent option-set/unset comparison | Production set path plus test-local unset control proposed | Accepted scope, but the population/exhaustion oracle is incomplete per F-04 |
| Existing security and cleanup | Same CA verification, no override, public stop, contained topology and cleanup proposed | Correct |

## Finding F-03 — the proposed dialing client is not pinned as the required VM Service

**Severity:** blocker

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — existing-scenario correction

**Status:** open

V-6 and roadmap criterion 3 name the actor precisely: a **VM Service** resolves
and dials the mesh peer by name. The proposal first deploys one real VM Service
as the listener/probe target, then says an “ordinary guest client” performs
the name and VIP dials. It does not require that dialing client to be submitted
as `SubmitSpecInput::Service`; retaining the current Job as that client would
repeat the same workload-kind imprecision iteration 4 exposed in R5-41.

The proposal must pin two roles without changing the scenario identity:

1. a client VM **Service**, submitted through the existing public deploy path,
   whose guest application performs the gateway-DNS, mesh-name, mesh-VIP TCP,
   Service-VIP UDP and off-node actions required of the V-6 actor; and
2. a peer VM Service exposing the required TCP/UDP application endpoints,
   probe state and remote-inbound target.

The existing off-node Job phase may remain as roadmap criterion 1's original
run-to-completion proof, but it cannot substitute for the VM Service actor in
V-6. The client Service may use any private, valid listener shape required by
Service admission; that is DISTILL fixture HOW and needs no user decision.

All name/VIP outcomes must be attributed to the client Service's allocation,
not to a helper process in the outer guest or to the peer Service itself.

## Finding F-04 — the zero-uplink, mTLS, probe-state and socket-option controls are not yet fail-closed

**Severity:** blocker

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — oracle specification

**Status:** open

The proposal names suitable tool families, but several absence/comparison
claims remain too loose to implement or independently review without accepting
vacuous evidence.

The revised proposal must pin these private test-support requirements:

### mTLS / no-cleartext

- identify the exact client-Service-to-peer-Service frontend/VIP tuple before
  capture;
- use the existing external socket observation to require the one held leg-F
  socket to report kTLS TX and RX (`tcp-ulp-tls`, `txconf`, `rxconf`) and the
  expected TLS version;
- arm capture before application bytes, require valid TLS records on the
  exact observed tuple, and require the byte-distinct application markers to
  be absent from captured wire payload; and
- make capture loss/truncation or tuple ambiguity fail the body.

A successful application reply alone does not prove mTLS and an empty capture
does not prove confidentiality.

### TCP and HTTP probes follow state

- define the application's healthy → unhealthy → healthy transition and the
  public command or guest action that drives it;
- identify both probe descriptors by role/index/mechanic on the public
  describe surface; and
- require each TCP and HTTP result to advance through the expected failure and
  recovery observations within explicit bounds. A direct prober call or a
  console-only claim remains insufficient.

### with-default no-uplink capture

- name and read back the disposable uplink/default route before the dial and
  remove/read back both afterward;
- arm the AF_PACKET observation before the same-node dial, filter/assert the
  exact tuple/interface/direction, and seal it only after the byte-exact round
  trip;
- read packet/drop statistics and fail on capture loss, truncation or parser
  ambiguity; and
- require zero matching uplink frames together with the held leg-C socket's
  `127.0.0.1:<ephemeral>` peer. Zero observed packets without loss accounting
  is not a closed oracle.

### `IP_BIND_ADDRESS_NO_PORT` set/unset comparison

- read the disposable guest's actual ephemeral range and state the exact
  concurrent population derived from it;
- use enough distinct registered destination tuples for the option-set run to
  exercise port sharing rather than repeating one identical four-tuple;
- hold both cohorts for observation and require the unset control to exhaust
  at the measured range while the production option-set path admits the
  accepted comparison population;
- observe the option state and socket tuples externally; do not add a
  production flag or alternate enforcement path; and
- bound and clean every socket, route, listener and temporary topology.

If the production-composed set cohort or same-CA remote credential cannot be
driven with existing public/product surfaces plus private test setup, the
implementation must stop with that exact testability gap. It must not add a
`ServerHandle` accessor, production CA hook, option toggle, identity bypass or
another test identity.

These are private HOW corrections to the existing proposal and require no
user approval. They do not add an outcome or weaken the accepted contract.

### Scope and authorization assessment

No part of the accepted missing coverage requires a DESIGN change. The
feature delta already pins every outcome and classifies V-6/V-LEGB-SERVE as
DELIVER confirmation of established mechanisms. Completing P6-S13 is within
the standing authority for test-support correction.

The following remain authorized within the existing scenario:

- extending existing test-only guest workload modes and the shared test
  protocol;
- adding private deploy/describe, CA-fixture, socket-table, route, AF_PACKET
  and probe-state helpers;
- creating disposable external topology and controls inside the outer
  GuestKit guest; and
- adding assertions for the exact already-accepted outcomes.

They do not constitute a new product mechanism or scenario when they retain
the existing P6-S13 identity and production entrypoints. The test-owned route,
uplink, remote peer and capture are environment/observation fixtures; they
must not install product nft/BPF/owner state or replace any production
adapter.

The proposal correctly keeps the five control-plane bodies unchanged and
must keep the runner selector/count unchanged. Any seventh body, production
override, public observation hook, new lane or contract relaxation remains
unauthorized.

### Iteration 6 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | The matrix includes success, application-state failure/recovery and route/option controls. |
| GWT / one behavior | 8/10 | One cumulative V-6 journey is retained, but actor roles need exact pinning. |
| Domain language | 10/10 | Service, mesh peer, frontend/VIP, probe state, inbound and uplink terms match the accepted contract. |
| Coverage completeness | 7/10 | Every missing accepted row is inventoried; two execution/oracle specifications remain incomplete. |
| Walking-skeleton user value | 9/10 | The proposed body exercises the complete operator journey through production surfaces. |
| Priority validation | 10/10 | The proposal addresses the DELIVER review's sole blocker without production redesign. |
| Observable behavior | 4/10 | Several absence/comparison claims lack closed, loss-accounted or population-pinned oracles. |
| Traceability | 10/10 | The audit maps each accepted V-6/V-LEGB-SERVE claim to current and proposed evidence. |
| Walking-skeleton boundary | 9/10 | Production serve/deploy and real owners are preserved; the client-Service role must be explicit. |

Observable behavior remains below the approval threshold and two blockers are
open.

### Iteration 6 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS with constraints | Proposed business stimuli enter through serve/deploy and public surfaces; private setup may not replace production adapters. |
| CM-B — domain abstraction | PASS | The proposal and expected assertions use accepted V-6 language. |
| CM-C — complete user journey | **FAIL pending F-03** | The required VM Service actor is not pinned as the dialing client. |
| CM-D — fixture isolation | PASS with constraints | All privileged setup remains disposable/test-owned; no production hook is proposed. |
| CM-E — universe-bound assertion | PASS / layer N/A | This is a layer-4+ native journey; direct external assertions are appropriate. |
| CM-F — layer-dependent PBT | PASS | No generative native execution is proposed. |
| CM-G — two-tier rich journey | N/A | The existing cumulative Tier-3 scenario is being completed. |
| CM-H — explicit integration paths | **FAIL pending F-04** | Absence and comparison paths are named but not yet mechanically fail-closed. |

The C1–C7 inventory is now correctly populated for the existing scenario:
empty/non-empty datagrams (C1/C3), healthy/unhealthy/recovered probe state
(C2), route and socket-option modes (C5), remote credential/refusal security
(C6), and concurrent port-range exhaustion plus environment cleanup (C7).
The canonical checklist cannot pass until the two findings' exact actor and
oracles are recorded.

## Iteration 6 verdict

**NEEDS REVISION.** The audit is materially correct: current evidence does
not close complete V-6, lower-level mechanism tests cannot be aggregated into
the required production-composition claim, and the smallest authorized remedy
is completion of the existing P6-S13 body and private helpers with the six
identities and 117 count unchanged.

No DESIGN or user decision is required. The proposal must first pin the
dialing workload as the required client VM Service and make the mTLS,
probe-state, no-uplink and option-comparison oracles mechanically fail-closed
as specified above. Those revisions remain private DISTILL HOW and do not
authorize a new test, scenario, lane, production API, override, mechanism or
contract weakening.

Return the revised proposal to this same reviewer before executable source is
changed. The existing `410a…` receipt remains valid only for its already
executed subclaims and must not be relabelled as complete V-6 evidence.

# NEEDS REVISION

## Iteration 7 re-review — revised V-6 acceptance proposal

Iteration 7 re-reviews the revised
`deliver/distill-14-01-v6-coverage.md`, limited to iteration-6 findings F-03
and F-04. This is approval of the exact acceptance-source proposal for
implementation, not evidence that the unimplemented V-6 coverage is already
green. The reviewer changed only this review artifact.

### Revised proposal identity

| Artifact | SHA-256 |
| --- | --- |
| Revised V-6 proposal | `e1a70a2bf7d26f4db09d0e7728baf497fd946fdbef6337030a458ba755ca8595` |
| DELIVER review iteration 1 | `f19e0fa48ece9ae8ad289b3dcae3676bb2ab164a532e76386209a1dd648122a7` |
| Accepted feature delta | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| Accepted roadmap | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |

The proposal remains documentation only. No executable source, scenario
identity, selector, DES event, roadmap, DESIGN artifact or commit changed in
this review cycle.

### F-03 remediation — exact VM Service actors

**Disposition: CLOSED.**

The proposal now pins two distinct workloads in the same production `serve`
lifetime:

- a **client VM Service**, submitted through the public deploy path, whose own
  guest application performs every gateway-DNS, mesh-name, frontend/VIP TCP,
  Service-VIP UDP and retained off-node dial; and
- a **peer VM Service**, separately submitted through the same public path,
  which owns the issued VIP/name and exposes the TCP, UDP, probe,
  state-control and remote-inbound endpoints.

Every dial result is attributed to the client Service's allocation and
console. An outer-guest helper, the peer Service, or the retained Job cannot
substitute for the accepted actor. The original run-to-completion Job remains
only roadmap criterion 1's off-node TCP/UDP proof.

This exactly matches V-6's required VM Service actor without adding an
identity. The client Service's private valid listener is admission shape; its
specific port and helper structure remain DISTILL HOW.

### F-04 remediation — mechanically closed oracles

**Disposition: CLOSED.**

#### mTLS and no cleartext

The revised proposal identifies the exact issued-VIP/application-port and
backend tuples before traffic. It arms the existing loopback AF_PACKET/TCP
reassembly support before the client Service can send unique request/response
markers. While the flow is held it requires exactly one correlated leg-F
socket and exactly one live in-process fd owner, then reuses the existing
closed kTLS predicate to require `tcp-ulp-tls`, TLS 1.3, `txconf: sw` and
`rxconf: sw`.

After the byte-exact reply it requires bidirectional TLS 1.3 application-data
records, zero cleartext-marker occurrences, zero packet drops, no truncation,
wire/snapshot-length mismatch, malformed reassembly or tuple ambiguity. Empty
or handshake-only capture fails. These complements prevent a successful
cleartext application reply or blind capture from receiving mTLS credit.

#### Public TCP/HTTP probe state

The peer Service declares exactly two ordered readiness descriptors:
TCP `(readiness, 0)` and HTTP `/healthz` `(readiness, 1)`, each on a dedicated
listener. A byte-distinct command sent by the client Service through the
ordinary production mesh path drives healthy → unhealthy → healthy; no host
file or direct prober call changes state.

The body matches descriptors and results by role/index/mechanic and the same
peer allocation, requires both Pass rows, then both newer Fail rows within
three intervals plus timeout, then both newer Pass rows within the same bound.
The peer allocation must remain unchanged. Missing, stale, partial or
replacement-allocation evidence fails.

#### with-default no-uplink and socket identity

The revised topology names one disposable veth uplink and sink namespace,
reads back link/address/ifindex and the exact default route, and arms capture
on that ifindex before the same-node dial. A test-owned positive-control frame
must be captured first, so a zero result cannot come from an inert observer.

The connection remains held until external socket-table observation finds
exactly one leg-C socket whose local tuple is the peer workload and whose peer
is `127.0.0.1:<nonzero ephemeral>`. After byte-exact completion, the capture
must show zero exact-tuple uplink frames, zero drops, no truncation or malformed
or ambiguous frame. Route, link and namespace are removed and their absence
read back on success and failure. This is a closed absence oracle and preserves
the production no-uplink outcome.

#### `IP_BIND_ADDRESS_NO_PORT` comparison

The revised proposal bounds the accepted comparison to a measured 64-port
subrange of the disposable guest's actual ephemeral range. It inventories
occupied source ports, derives `C` free ports and runs exactly `N = C + 1`
connections, with `2 <= N <= 65`, below the accepted production capacity
limits.

The unset control externally proves option value `0`, uses distinct
destinations, requires exactly `C` unique-source successes and an
ephemeral-exhaustion errno for connection `N`, and proves no row for the
failure. After that cohort is absent, the production client Service opens the
set cohort to `N` distinct registered peer-Service destination tuples. All
must remain byte-exact and held; externally correlated live fds must report
option value `1`, all `N` tuples, and fewer than `N` source ports. Zero or
multiple fd owners, an unregistered tuple or missing option evidence fails.

Both cohorts are bounded to 30 seconds. Every socket/listener is closed, the
original port range is restored and read back, and no cohort route,
namespace, interface or socket may remain. Production never gains an option
toggle or alternate enforcement path.

These oracles now meet iteration 6's fail-closed requirements. The exact
helper decomposition, RAII types and parsers remain private test-support HOW;
they do not require a user decision.

### Exact accepted-contract and scope check

The revised proposal covers every currently missing accepted row while
retaining the already-proved outcomes:

| Contract row | Revised proof boundary |
| --- | --- |
| Empty UDP | Real zero-length request/reply on the existing connected guest socket; source recorded; missing `recv_from(n == 0)` fails |
| gateway:53 DNS / mesh name | Client VM Service reads production resolver configuration, reports the gateway nameserver and resolves the derived peer Service name |
| Mesh frontend/VIP TCP | Client Service dials the public issued VIP; peer Service returns byte-distinct data; closed bidirectional mTLS/kTLS oracle applies |
| Service VIP UDP / `connect4` | Client Service sends to the same public issued VIP and declared UDP listener through the production cgroup path |
| Remote inbound | Route-disjoint real mTLS client, same production workload CA, verified handshake and byte exchange through leg C → leg S → `GuestStreamDial` |
| TCP and HTTP probes | Same peer allocation observed Pass → newer Fail → newer Pass for both public descriptors |
| V-LEGB route modes | Existing route-less production proof plus disposable with-default topology and byte-exact round trip |
| leg-C peer / no uplink | Exact held socket identity plus positive-control, loss-accounted zero-uplink capture |
| Concurrent set/unset | Ephemeral-range-derived, distinct-destination, externally observed bounded comparison with complete cleanup |

All business actions remain in the existing P6-S13 body and production
composition. The five `guest_flow_through_serve` complements remain unchanged.
The executable inventory stays exactly six identities, and the existing
GuestKit runner must still reconcile `111 + 1 + 5 = 117` with zero failure,
ignore or runtime skip.

The proposal adds no seventh body, public API, config/wire field, production
hook, adapter override, identity bypass, model, lane, selector, executor or
production option flag. Extending the existing test-only guest application,
shared protocol, deploy/describe helpers, socket observation, CA fixture,
capture and disposable topology is authorized completion of the accepted
scenario, not a product mechanism.

All privileged topology, capture, route, namespace and real-VM effects remain
inside the disposable outer GuestKit guest. The physical Metal host remains a
read-only cleanup-audit surface. Same-CA remote credential setup must continue
to use existing contracts and preserve peer verification; if that or the
production set cohort proves impossible without a new production surface, the
implementer must stop with that exact testability gap.

### Later implementation and evidence gate

This approval does not make the current `410a…` receipt complete V-6 evidence.
The existing receipt remains valid only for its executed subclaims. After
implementation, the same independent reviewer must verify:

- only existing P6-S13/private-support files changed within the documented
  bounded impact, with no new identity or weakened assertion;
- all new observations derive from real guest/public/kernel surfaces and no
  expected value is manufactured by a fixture;
- Metal-only format/check/Clippy for every changed target;
- rebuilt existing guest program and unchanged existing runner;
- exactly 117 passed, zero failed/ignored/runtime-SKIP bodies;
- retained raw metadata/output/terminal identities and per-row V-6 evidence,
  including packet/drop and cleanup complements; and
- zero physical-host VMM, nft table or STEP 14-01 workdir after the run.

The refreshed full STEP 14-01 DELIVER review remains mandatory after that
DISTILL evidence is approved.

### Finding dispositions

#### F-03 — client workload kind

**Disposition: CLOSED.** Distinct client and peer VM Services are now pinned,
and the client owns every required dial.

#### F-04 — fail-closed absence/comparison oracles

**Disposition: CLOSED.** mTLS, probes, uplink absence and option comparison
now carry exact positive, loss, identity, population, ordering and cleanup
complements.

#### New iteration 7 findings

None.

### Iteration 7 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Success, unhealthy/recovery, route mode and exhaustion controls are all pinned. |
| GWT / one behavior | 9/10 | One cumulative accepted V-6 journey retains explicit client and peer roles. |
| Domain language | 10/10 | Service, mesh peer, name, VIP, probe state, inbound, uplink and option terms are exact. |
| Coverage completeness | 10/10 | Every accepted V-6/V-LEGB-SERVE row maps to an executable production-boundary oracle. |
| Walking-skeleton user value | 10/10 | The proposal drives the complete serve/deploy journey and observable application outcomes. |
| Priority validation | 10/10 | It closes the DELIVER review blocker without product redesign or scope expansion. |
| Observable behavior | 10/10 | Presence, absence, confidentiality, state transitions and exhaustion are all fail-closed. |
| Traceability | 10/10 | Every current and proposed claim maps to the accepted contract and fixed identity ledger. |
| Walking-skeleton boundary | 10/10 | Real production owners remain composed; fixtures provide only external environment and observation. |

All dimensions meet the proposal-approval threshold and no blocker remains.

### Iteration 7 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Client/peer actions enter through public deploy and real production owners; no adapter replacement. |
| CM-B — domain abstraction | PASS | The scenario remains expressed by V-6 business outcomes; technical machinery stays in private helpers. |
| CM-C — complete user journey | PASS in proposal | Both Service roles and every accepted phase/outcome are pinned in one scenario. |
| CM-D — fixture isolation | PASS | Test-owned topology/controls are external, disposable and cleanup-bound; production state is installed only by production. |
| CM-E — universe-bound assertion | PASS / layer N/A | This is a layer-4+ native journey with direct public/kernel/wire assertions. |
| CM-F — layer-dependent PBT | PASS | Native execution remains one explicit example. |
| CM-G — two-tier rich journey | N/A | No new scenario or in-memory tier is introduced. |
| CM-H — explicit integration paths | PASS in proposal | Every named path has a concrete positive, negative/loss and cleanup oracle. |

The C1–C7 coverage populated in iteration 6 is now mechanically specified:
empty/non-empty cardinality, probe state transitions, route/option modes,
credential/confidentiality robustness, concurrent exhaustion and interruption
cleanup. The canonical checklist is complete at proposal level; executable
completion remains subject to the later native evidence gate.

## Iteration 7 verdict

**APPROVED FOR DISTILL IMPLEMENTATION.** The revised proposal now matches the
exact V-6 and V-LEGB-SERVE contract, pins the required client VM Service and
peer VM Service roles, and closes every absence/comparison oracle without a
production hook or outcome weakening.

The correction remains completion of the existing P6-S13 body and private
test support. It retains the five control-plane bodies, six exact identities,
117-body runner count, production security boundaries and physical-host
containment. No new test, scenario, API, lane, override, product mechanism or
user approval is required.

This verdict authorizes the documented test-source work only. Complete V-6
approval still requires source review, exact Metal evidence and the refreshed
DELIVER review after implementation.

# APPROVED

## Iteration 8 re-review — implemented V-6 completion at semantic RED

Iteration 8 reviews the executable implementation of the iteration-7-approved
V-6/V-LEGB-SERVE proposal and the native record that stops at the accepted
Service-VIP UDP action. The controlling question is whether this is an honest
production semantic RED reached by the existing P6-S13 body, or a support
failure that must be repaired before production work resumes.

This verdict confirms the authored Service-VIP UDP semantic RED while
requiring one bounded correction to a later, unexecuted population oracle. It
does not mark V-6 GREEN, credit any assertion that was not executed, or
approve the pending production correction.

### Reviewed source and evidence identity

| Artifact | SHA-256 / identity |
| --- | --- |
| Existing P6-S13 body | `6d1b3637a6751967e80c140a0bd7a754ea1b9d57fe65f19f0432d3054c4512c3` |
| CLI private native support | `cdbf3f430f49a1d64a7ec1ebdbdf0e4f33f48c24b6241a2e2e65c39aa48f9435` |
| Existing guest workload program | `93af00ab2c4a6824687b2ec247afd4af89bed174779af5ed9b96b087a7ae6029` |
| Shared test protocol / remote peer | `baa0ec4a2bb6256156366346092c234fed15a16d724d13e1198c676a9e7a3776` |
| Unchanged existing GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| Unchanged five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` |
| V-6 implementation handoff | `43a08985495f0ec5c4598c397a0457a47707420e408f055c329b4c0c48f782da` |
| Revised V-6 proposal and evidence reconciliation | `a58327ac89c0b1ce45125880d9df772daa667a779217fda6e4479a3e95fd1649` |
| Native record | `012109f99448d662b35c1760`, source digest `62f6c1832d836a9536a84d89d821f14bd43357fbb5f8a2125f5f8efb16ca3728` |
| Native output / terminal | `409cc1acae243bfe2106c8203e836152fdf266d659e7a5f8803ab267782368d4` / `e5fda4bc0fe719377f8516ae4b0c7315bf676c7961da9386fb0591fabe46311c` |

The record metadata names commit `c9f2f6d6975b236fa4a049093550d3ea8dcf4242`,
the source digest above, privileged execution and terminal
`command_returned` exit 101. The implementation changes only the existing
P6-S13 body and private/shared test support, plus two
`overdrive-cli` dev-dependencies used by the private certificate verifier.
No production crate, public product API, wire/config/persistence shape, model,
DES event, runner selector, lane or production override changed.

The runner still names exactly one CLI identity and the same five
`guest_flow_through_serve` identities. Its count remains
`111 + 1 + 5 = 117`. The five control-plane bodies are byte-identical to the
reviewed GREEN source. P6-S13 remains one body with its existing
`CONTRACT_SHAPE: bounded-change.` declaration; completing that body did not
create a seventh scenario.

### Proposal-to-source conformance

| Accepted V-6/V-LEGB-SERVE item | Implemented production-boundary stimulus and closed oracle | Assessment |
| --- | --- | --- |
| Original Job TCP, non-empty UDP, empty UDP and cleanup | The retained Job still deploys through the public handler. Its connected UDP socket now performs a real zero-length send/receive; `RemotePeer` records only an actual `recv_from(n == 0)`; P6-S13 retains byte/source/console checks and the post-terminal VMM absence check. | Conforms; executed before the later RED. |
| Distinct client and peer VM Services | `deploy_v6_peer` and `deploy_v6_client` each submit a separate one-replica Service through the public deploy handler. The client Service owns every DNS/name/VIP/off-node dial; the peer Service owns the issued name/VIP, TCP/UDP applications, probes and remote-inbound target. | Conforms. |
| Gateway DNS and mesh name | The client reads the production `/etc/resolv.conf`, resolves `v6-peer.svc.overdrive.local`, then requires the peer's byte-distinct name response. The body requires the production gateway nameserver and resolved-name console facts. | Conforms; executed through resolution and byte exchange. |
| Issued-VIP TCP with mTLS/no cleartext | The issued VIP comes from public describe; the client sends unique request/response records and holds the connection. The pre-armed loopback AF_PACKET observer requires zero drops/truncation/parse ambiguity, one exact backend kTLS fd with TLS 1.3 software TX/RX, application-data records in both directions and no cleartext marker. | Conforms in source; the byte-exact VIP TCP action executed, while the later sealed-capture oracle remains uncredited. |
| Service-VIP UDP through `connect4` | The same client Service connects a real `UdpSocket` to the public issued VIP and the separately declared UDP port, sends `V6_UDP_REQUEST`, accepts only `V6_UDP_RESPONSE`, and publishes `vip-udp ok` only after the byte-exact reply. | Conforms and is the current semantic RED. |
| Route-disjoint same-CA remote inbound | Private setup issues a real client SVID from the same persisted production workload CA. The verifier checks the trust chain, time, signature and one syntactically valid SPIFFE URI; the client authenticates with its SVID and exchanges distinct bytes through the registered workload tuple. It does not install an identity bypass or replace the production CA owner. | Conforms; executed before client deployment. |
| TCP/HTTP probe state | The peer declares the exact ordered readiness descriptors. The client sends acknowledged state commands over the ordinary Service path. Public describe must show Pass → newer Fail → newer Pass for both indices on the same peer allocation. | Conforms and remains fail-closed behind `paths-ready`; no transition credit from this run. |
| With-default no-uplink | A disposable veth/default route is read back, capture is bound before client deployment, and a test-owned positive-control datagram proves the observer live. The held same-node flow must have one backend-local leg-C socket with a loopback peer; sealed capture requires zero exact backend tuple frames with zero loss/truncation/ambiguity. RAII removes route/link/namespace on failure, and the successful path also reads back absence. | Conforms and remains uncredited. |
| `IP_BIND_ADDRESS_NO_PORT` set/unset comparison | A measured 64-port subrange derives `C` and `N=C+1`. The unset control reads option 0, uses distinct destinations, requires exactly `C` unique source ports and the accepted exhaustion errno at `N`. The production cohort requires every byte-distinct acknowledgement, retains `N` sockets, correlates each selected row to one live fd with option 1, and requires fewer than `N` source ports. | **Needs revision per F-05:** the external selector is port-only, it does not prove the production source ports lie in the narrowed range, and neither cohort has the approved external post-close absence oracle. |

No success line is emitted before its corresponding byte exchange. The later
body assertions cannot be skipped after the UDP failure because the client
never emits `paths-ready`, and the body waits specifically for that line
before it can evaluate mTLS capture, probe transitions, population, no-uplink
and final cleanup claims. The mTLS, probe and uplink oracles retain their
accepted negative complements. The population oracle has the bounded
source/absence omissions recorded as F-05.

### Bounded retry and socket-honesty review

The support corrections do not convert a forbidden outcome into success:

- DNS returns only after a real IPv4 answer and the subsequent name-targeted
  byte exchange; a fabricated console answer cannot satisfy the peer's unique
  response.
- Mesh-name and VIP TCP retries open real sockets and still require the exact
  peer response. The initial ten-second identity-convergence wait supplies no
  result and cannot bypass transport or authentication.
- Service-VIP UDP uses one real connected socket per client allocation,
  bounded reads and a 30-second retry window. A timeout, I/O error or
  non-matching response remains failure; the exact success marker precedes
  `paths-ready`.
- Service replacement supplies fresh VM allocations and fresh guest sockets.
  It does not reuse a test adapter, inject an expected reply, or alter a
  production listener/map.

The 64-port control is not a substitute data path. It changes only the
disposable outer guest's kernel range, the unset cohort closes before the
client Service starts, and TCP and UDP use separate ephemeral-port spaces.
The Service-VIP UDP attempt therefore remains the production-composed path the
contract requires.

### Native reachability and semantic-RED classification

**Disposition: honest production semantic RED; no DISTILL support remediation
is required before the original crafter traces the product path.**

The failure is reached after the necessary fixture preconditions:

1. The peer guest binds UDP `0.0.0.0:18081` synchronously before it spawns
   the UDP server and emits `v6-peer: ready`; the public Service spec
   separately declares that port with protocol `udp`.
2. P6-S13 waits for the peer's ready console, reads its issued VIP and backend
   from public describe, observes the initial public TCP and HTTP probe
   results, and completes the route-disjoint same-CA remote mTLS exchange.
3. The client Service then proves the same production guest/owner is live:
   it reports `nameserver=100.95.0.1`, completes off-node TCP, non-empty UDP
   and empty UDP, resolves the peer name to issued VIP `10.98.0.1`, and
   completes byte-exact TCP to that VIP.
4. Only the next action, a real connected UDP exchange to
   `10.98.0.1:18081`, does not return the peer's distinct response.

The raw record contains 101 successful libtest summaries totaling exactly
111 passed, zero failed and zero ignored before P6-S13. It invokes the exact
P6-S13 identity once. Client allocations `alloc-v6-client-0` and
`alloc-v6-client-1` both reach `Failed` with
`WorkloadCrashedImmediately { exit_code: Some(21) }`; exit 21 is the
test-protocol `UDP_TIMEOUT`. Successor `alloc-v6-client-2` remains
`Running` after the same nameserver, off-node, name resolution and VIP-TCP
sequence, but produces neither `vip-udp ok` nor `paths-ready` inside the
120-second body bound. The teardown bound then kills the remaining peer and
client scopes rather than allowing the body to hang.

The earlier diagnostic record `a5eb43472f5dc625d0c29e86` also contains the
peer's remote, name and VIP-TCP request lines and no UDP-request line.
Its metadata records source digest
`1e0fd4bd7a7781ba8c1fbddee9a1c1ab5bb55ced5bbe18779ce5bf6296648ada`,
so it is corroborating diagnostic evidence, not an exact-source receipt for
the final implementation. The classification above does not depend on it;
`012109…` alone reproduces the missing accepted outcome across two failed
successors and a third bounded stall at the exact action.

This is concrete reachability through the public deploy handler, production
`serve`, the issued Service VIP, the real client VM socket and the declared
peer VM listener. It is not a compiler, GuestKit image, KVM, readiness,
identity, DNS, TCP or manufactured-observer failure. Determining which
production `connect4`/UDP association stage drops the datagram belongs to
the original STEP 14-01 crafter; this review does not prescribe a product
mechanism.

### Cleanup, confinement and evidence limits

The current failed body cannot run its success-path public stop and final
socket/capture complements. The existing `TeardownBound` nevertheless
records and kills the two remaining workload scopes when the in-process
serve cannot drain within its 30-second failure bound. The handoff's
read-only physical-host audit reports no live Cloud Hypervisor process, host
nft table, `alloc-v6-*` scope or `/run/overdrive/vm/alloc-v6-*` directory,
and no Metal job remains active.

All route, namespace, packet-socket, ephemeral-range and nested-VM effects are
owned inside the disposable GuestKit guest. Their RAII destructors restore or
remove test-owned state during unwind. No physical-host mutation or new
native execution was performed by this reviewer.

The following claims remain deliberately unproved by `012109…`:

- sealed bidirectional mTLS/no-cleartext capture;
- healthy → unhealthy → healthy TCP/HTTP public probe results;
- the production option-set population and its set/unset comparison;
- positive-control, loss-accounted zero-uplink observation; and
- success-path Service stop, final socket absence and graceful serve
  shutdown.

Their source is fail-closed except for F-05's population tuple/range/absence
gap, and only a later exact-source GREEN native run may receive evidence
credit. That run must follow F-05 remediation and also supply the iteration-7
format/check/Clippy, exact 117-pass, runtime-SKIP, durable raw identity and
physical-host cleanup gates before the refreshed DELIVER review.

### Finding dispositions

#### Iteration-6 F-03 — exact client and peer Service actors

**Disposition: CLOSED IN SOURCE.** The implementation uses two distinct public
Service deployments, and the client VM Service owns every required dial.

#### Iteration-6 F-04 — fail-closed absence/comparison oracles

**Disposition: PARTIALLY REOPENED BY F-05.** The mTLS, probe and no-uplink
oracles retain the approved positive, loss/accounting, identity, ordering and
cleanup complements. The option-population oracle retains the bounded
population, option and fd observations but omits exact tuple/range and
post-close absence complements. Because UDP fails before `paths-ready`,
iteration 8 grants none of these later claims native proof credit.

#### New iteration-8 findings

##### F-05 — High: the option-population oracle does not close the approved exact tuple/range and cleanup complements

**Evidence.** `vsock_support.rs:841-859` implements
`assert_population_option_set(population)` by selecting every established
socket whose peer *port* lies in
`V6_POPULATION_BASE_PORT..+population`. It does not receive or compare the
peer allocation's leased `peer_addr`, and it does not require the selected
local source ports to fall inside the narrowed 64-port range. The helper
therefore does not externally correlate the `N` observed rows to the exact
registered peer-Service backend tuples on which the accepted sharing
comparison depends.

`run_unset_control` (`vsock_support.rs:638-690`) closes its successful
clients/listeners and explicitly closes the failed raw fd, but it never
performs the approved external
socket-table complement proving the unset cohort and connection `N` have no
live row before the production cohort begins. After stopping the Services and
shutting down `serve`, P6-S13
(`vsock_walking_skeleton.rs:183-216`) checks VMM disappearance but does not
query the socket table for absence of the production population. These
omissions are unconditional in the current source; a later GREEN run could
complete without ever testing the accepted no-row/no-cohort residue claim.

**Required bounded remediation.** Keep the same body, actors, population and
private helpers. Correlate the production rows to the exact expected backend
set `{(peer_addr, V6_POPULATION_BASE_PORT + i) | 0 <= i < N}`, require every
selected local source port to be inside the retained narrowed range, and
externally require no live unset-control row after that cohort closes and no
live production-population row after Service/serve cleanup. Connection `N`'s
accepted exhaustion must likewise leave no live row. The existing fd option
checks, `N` byte acknowledgements, `N` tuples, fewer-than-`N` source
ports, bounds and range restore remain unchanged.

This is private DISTILL HOW already required by iteration 7. It needs no
DESIGN or user decision and does not authorize a new body, scenario, lane,
production hook or product change. It does not invalidate the earlier
Service-VIP UDP semantic RED, so product tracing may continue independently.

The differing source digest of `a5eb…` only narrows its evidence label; it
does not create a second finding because the final-source `012109…` record
independently reproduces the semantic failure.

### Iteration 8 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The source retains success, application-state failure/recovery, route and exhaustion controls, while current native evidence honestly stops at the first missing outcome. |
| GWT / one behavior | 9/10 | The cumulative V-6 journey remains one P6-S13 identity with explicit Job, client Service and peer Service phases. |
| Domain language | 10/10 | Service, issued VIP, mesh name, probe result, remote inbound, leg C, uplink and option-state terms match the accepted contract. |
| Coverage completeness | 9/10 in source | Every accepted stimulus exists, but the option-population row lacks exact tuple/range and post-close absence complements. |
| Walking-skeleton user value | 10/10 | The body exercises the public operator journey and stops on the missing stakeholder-visible UDP result. |
| Priority validation | 10/10 | It completes the existing acceptance inventory without creating a scenario or product mechanism. |
| Observable behavior | 7/10 in source | mTLS, probes and uplink are closed; F-05 leaves the option-population identity and cleanup observations incomplete. |
| Traceability | 10/10 | Source, runner identity, raw receipt and accepted outcomes are explicitly reconciled. |
| Walking-skeleton boundary | 10/10 | Real production owners remain composed; fixtures supply only guest workloads, external environment and observation. |

### Iteration 8 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Public deploy/describe/stop and production `serve` own product state; private support does not replace a production adapter. |
| CM-B — domain abstraction | PASS | The body asserts accepted V-6 outcomes; socket/capture mechanics remain private helpers. |
| CM-C — complete user journey | PASS in source / RED in product | Both Service actors and every accepted phase are present; Service-VIP UDP currently prevents completion. |
| CM-D — fixture isolation | PASS | Guest/topology/capture/credential setup is test-owned, disposable and confined to the outer GuestKit guest. |
| CM-E — universe-bound assertion | PASS / layer N/A | Direct public, guest, kernel and wire observations are appropriate for this Tier-3 journey. |
| CM-F — layer-dependent PBT | PASS | No generative native body is introduced. |
| CM-G — two-tier rich journey | N/A | The existing cumulative Tier-3 scenario is retained. |
| CM-H — explicit integration paths | **FAIL pending F-05** | The population path lacks its exact backend/range correlation and external absence complements. |

## Iteration 8 verdict

**NEEDS REVISION FOR F-05; SERVICE-VIP UDP SEMANTIC RED CONFIRMED.** The
implemented completion preserves the existing P6-S13 identity, unchanged five
control-plane bodies, six exact identities, 117-body runner count, production
security boundaries and Metal/GuestKit confinement. It adds no test, scenario,
lane, product API, override, adapter replacement or production mechanism.
Its current native stop is an honest semantic RED, but its later
option-population oracle does not yet implement every iteration-7 complement.

The Service-VIP UDP failure is a reproduced missing accepted product outcome,
not a fixture-contract gap. The original STEP 14-01 crafter may implement the
exact accepted production behavior and rerun the same body. DISTILL must not
weaken, skip or replace the UDP action to obtain GREEN.

Return F-05 to the same DISTILL author as a private test-support correction;
no user approval is required. Complete V-6/V-LEGB-SERVE approval remains
pending that correction and the same body reaching all later fail-closed
assertions, exactly 117 passes with zero failure/ignore/runtime-SKIP, the
required source gates and cleanup evidence, followed by the refreshed
independent DELIVER review.

# NEEDS REVISION

## Iteration 9 re-review — F-05 population-oracle remediation

Iteration 9 is limited to iteration-8 finding F-05. It reviews the private
DISTILL corrections to the existing unset-control and production
`IP_BIND_ADDRESS_NO_PORT` observations. It does not re-review or grant native
credit to the still-unreached V-6 phases, and it does not require the accepted
Service-VIP UDP product behavior to be GREEN before this source-only finding
can close.

The reviewer changed only this review artifact. No executable source, DES
event, execution log, roadmap, DESIGN artifact, receipt or commit was changed
by the reviewer.

### Iteration 9 source identity and bounded scope

| Artifact | SHA-256 / disposition |
| --- | --- |
| Existing P6-S13 body with F-05 calls | `840bb1f182778eb50c1c40e0bac097770cb15a35639d3fca0975d1fd69b29df3` |
| CLI private native support with F-05 helpers | `327ae9997fee5dc316d8b47131078a78a437f54bdbb381459eaa168cc860dad1` |
| Guest workload program | `93af00ab2c4a6824687b2ec247afd4af89bed174779af5ed9b96b087a7ae6029` — unchanged from iteration 8 |
| Shared test protocol / remote peer | `baa0ec4a2bb6256156366346092c234fed15a16d724d13e1198c676a9e7a3776` — unchanged |
| Existing GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` — unchanged |
| Five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` — unchanged |

#### Post-review rustfmt identity addendum

The two iteration-9 identities originally recorded before the Metal source
gate were:

- `vsock_support.rs`:
  `44a237e2aa84d458ff0a371bad632077f5059500d997450007db830f3f53bc79`;
  and
- `vsock_walking_skeleton.rs`:
  `b6d94055618a1d04fea9e8e0227743983a6aa8aa225079db88a2b8790b2e62bc`.

Metal record `615498436b6a58d31a576a79` (source digest
`502c882fbb837f416d41b998f2f34a005bf93a9b94a98629d2816c8ca6d57e27`,
exit 1) reported exactly three rustfmt-only differences: one private
function-signature wrap, one iterator-chain wrap and the P6-S13 private-helper
import wrap. Applying those mechanical corrections produced the current
hashes in the table above. The F-05 predicates, state set, expected tuples,
bounds, call ordering and assertions are semantically unchanged.

The follow-up Metal record `f73ed9057b4f5426240e1111` (source digest
`d38b41c6c561e5f926e94f82a0dd272ec1707f497d687b09b3ad5b4f4b2a4c42`,
exit 101) passed rustfmt and the scoped `overdrive-cli` integration cargo
check. Its output then reached Clippy and stopped on
`clippy::result_large_err` at `guest_app.rs:704`, where the broader
pre-F-05 `PeerLog` is the `Err` payload. `guest_app.rs` remains
`93af00ab2c4a6824687b2ec247afd4af89bed174779af5ed9b96b087a7ae6029`,
the same source reviewed in iterations 8 and 9.

The follow-up is therefore compile evidence for the formatted F-05 source,
not a clean full source gate. No Clippy waiver or adjacent `PeerLog`
correction is claimed or authorized by this addendum. The rustfmt-only
identity refresh does not reopen F-05.

| Metal record | Output SHA-256 | Terminal SHA-256 | Honest disposition |
| --- | --- | --- | --- |
| `615498436b6a58d31a576a79` | `4df5ac5547dc3920258ff212d5ed4fb24030fc900792b6495718d9bfe75a5fd0` | `b3ce135d9d48c7dfd0fc4d1af02bffb17afb509db2b3234cd6b20302ae31c8f7` | rustfmt differences only; exit 1 |
| `f73ed9057b4f5426240e1111` | `d557bdd534848a3c61a05ee1b7e0430c247cb758bb11b34a22e4e7291b1cbf39` | `9852ac99f1e620bdf6c97b2f840e5d0b166b19aec1ff696b4f10ca916de28af8` | fmt and scoped check pass; unrelated pre-F-05 Clippy finding; exit 101 |

The correction is confined to private support in
`vsock_support.rs:650-717,848-952` and the existing P6-S13 body's strengthened
helper call and post-shutdown absence assertion at
`vsock_walking_skeleton.rs:182-218`. It adds no body, identity, selector,
lane, public product API, config/wire/persistence shape, production hook,
adapter, option toggle or product mechanism.

The runner still selects the one existing P6-S13 identity and the same five
control-plane identities, with the fixed `111 + 1 + 5 = 117` count. The
guest workload, peer protocol and five control-plane bodies are byte-identical
to iteration 8. The existing P6-S13 business stimuli and outcome assertions
are unchanged; only F-05's external observation and cleanup complements are
stronger.

### Exact F-05 closure

#### Unset connection N has no live row

**Disposition: CLOSED.**

After the unset cohort reaches the accepted
`EADDRINUSE | EADDRNOTAVAIL` exhaustion at connection `N`,
`run_unset_control` obtains that exact listener's IPv4 destination and calls
`assert_no_live_tcp_rows` before the listeners are dropped. The observer
reads `/proc/net/tcp`, parses both endpoints and the kernel TCP-state code,
and matches the failed destination as either local or peer. The listener's
own LISTEN row cannot manufacture a failure because state `0A` is
deliberately excluded.

The live-state predicate excludes only:

- `06` — TIME_WAIT;
- `07` — CLOSED; and
- `0A` — LISTEN.

It therefore remains fail-closed for ESTABLISHED, SYN_SENT, SYN_RECV,
FIN_WAIT1, FIN_WAIT2, CLOSE_WAIT, LAST_ACK, CLOSING and any other non-excluded
live/transitional row. The observer waits at most two seconds with ten
millisecond polling, so an active row must actually drain rather than being
ignored. This exactly closes the accepted “connection N creates no live row”
complement without treating harmless TIME_WAIT residue or the test listener
as a live connection.

#### Closed unset cohort has no live row

**Disposition: CLOSED.**

Before dropping anything, the helper inventories every exact
`127.0.0.2:20000+i` listener destination. It then drops all client sockets,
accepted sockets and listeners and invokes the same bounded live-state
observer against the complete destination set. A live row in either direction
for any cohort destination fails. The destination inventory is captured while
the listener objects still exist, so cleanup cannot erase the expected set
the oracle must inspect.

This is an external `/proc/net/tcp` complement. It does not infer cleanup
solely from Rust object drop and it does not accept ESTABLISHED or closing
states as absence.

#### Production population has exact backend and source-range identity

**Disposition: CLOSED.**

`population_backends(peer_addr, population)` constructs the exact expected
set:

`{ (peer_addr, V6_POPULATION_BASE_PORT + i) | 0 <= i < N }`.

`assert_population_option_set` now receives both the retained
`EphemeralRangeGuard` and the peer allocation's leased `peer_addr`. It
selects established rows only when their complete peer tuple belongs to that
expected set, then requires:

- exactly `N` selected rows;
- every selected local source port inside the retained
  `low..=high` narrowed range;
- exactly one live in-process fd for each inode;
- `IP_BIND_ADDRESS_NO_PORT == 1` on every selected fd;
- the observed destination set equal to the exact expected backend set; and
- fewer than `N` distinct source ports.

The exact-set equality prevents a duplicate, missing destination, wrong
backend address or port-only collision from satisfying the population
oracle. The range assertion ties the production cohort to the same measured
64-port universe whose unset capacity produced `N=C+1`. The existing
byte-distinct guest acknowledgements and held population remain prerequisites
before this external observation runs.

#### Closed production population has no live row before range restore

**Disposition: CLOSED.**

The body still stops both Services through the public operator verb, waits for
the named client and peer allocations to become terminal and requires both
VMMs to be absent. It then gracefully shuts down the production `serve`
composition. Only after those owner boundaries complete does
`assert_population_absent(peer_addr, population)` apply the same bounded
live-state observer to the exact production backend set.

The original ephemeral range is restored and read back only after that
absence assertion succeeds. A production socket left ESTABLISHED or in any
live closing state therefore fails before range restoration can hide the
cohort. TIME_WAIT is appropriately tolerated because it has no live owner,
fd or reusable connection state.

### Test honesty, ownership and unchanged semantic RED

The remediation observes kernel state from the disposable outer GuestKit
guest. It does not alter the production option, install product state, reach
into a production owner or manufacture a socket result. `pub(super)`
helpers and direct access to the private range bounds remain test-module
support owned by DISTILL.

The exact Service-VIP UDP action and its bounded failure behavior are
untouched. The native record `012109f99448d662b35c1760` still stops before
`paths-ready`, so it remains valid evidence only for the iteration-8
semantic RED and the earlier executed subclaims. It provides no execution
credit for the remediated population helpers. The two post-review Metal
source-gate records above add format and compile evidence only; they do not
run the product scenario or support a GREEN claim.

This separation is honest: F-05 concerned deterministic source omissions in
an unexecuted later oracle. Closing those omissions does not require weakening
the UDP outcome or waiting for the production crafter to make the preceding
accepted action succeed. A later exact-source native run must still execute
the population checks and every other post-`paths-ready` oracle before V-6
can be approved as complete.

### Finding dispositions

#### F-05 — option-population exact tuple/range and cleanup complements

**Disposition: CLOSED.** Exact backend-set selection, narrowed source-range
membership, connection-N absence, closed unset-cohort absence and post-serve
production-population absence are all present and fail-closed. The
remediation preserves the existing option/fd/population assertions and
requires no user or DESIGN decision.

#### New iteration-9 findings

None.

### Iteration 9 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Exhaustion, held sharing and both cleanup complements are explicit. |
| GWT / one behavior | 9/10 | The corrections remain observations within the one cumulative P6-S13 journey. |
| Domain language | 10/10 | Unset cohort, production population, leased backend, source range and live TCP state are precise. |
| Coverage completeness | 10/10 in source | Every iteration-7 population identity, range, option, capacity and cleanup complement is implemented. |
| Walking-skeleton user value | 10/10 | The correction protects the accepted concurrency outcome without changing the operator journey. |
| Priority validation | 10/10 | It closes only F-05 and does not expand into product remediation. |
| Observable behavior | 10/10 in source | Exact tuples, range membership, fd option values, live-state absence and bounded drain are externally observed. |
| Traceability | 10/10 | Every F-05 clause maps to a concrete helper and call site. |
| Walking-skeleton boundary | 10/10 | Product owners remain production-composed; private support only observes the disposable guest kernel. |

### Iteration 9 test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Public Service stop and serve shutdown own cleanup; test support observes their kernel complement. |
| CM-B — domain abstraction | PASS | Population behavior remains in P6-S13; parsing and state codes remain private helpers. |
| CM-C — complete user journey | PASS in source / RED in product | The source journey is complete; the earlier Service-VIP UDP product action remains RED. |
| CM-D — fixture isolation | PASS | All range and `/proc/net/tcp` work is confined to the disposable GuestKit guest. |
| CM-E — universe-bound assertion | PASS / layer N/A | Exact kernel rows and complete backend/control sets bound the observation universe. |
| CM-F — layer-dependent PBT | PASS | No generative body is added. |
| CM-G — two-tier rich journey | N/A | No new scenario or tier is introduced. |
| CM-H — explicit integration paths | PASS in source | Both cohorts now have exact identity and post-close complements. |

## Iteration 9 verdict

**APPROVED.** F-05 is closed by the exact private DISTILL remediation. The
existing P6-S13 identity now carries the approved exact backend/range,
option/fd, population and cleanup observations without a new body, scenario,
lane, API, override or product mechanism.

This source-only approval does not claim product GREEN or native proof for the
later assertions. Affected STEP 14-01 execution is now paused for DESIGN
reconciliation of the Service-VIP UDP contract; this review neither resumes
that execution nor asks for another native product run. If and when the
reconciled design authorizes DELIVER to resume, complete V-6/V-LEGB-SERVE
approval will still require the remediated exact source to execute all
applicable post-`paths-ready` assertions, the fixed 117-body result and
required source, durable-receipt and cleanup gates, followed by the refreshed
independent DELIVER review.

# APPROVED
