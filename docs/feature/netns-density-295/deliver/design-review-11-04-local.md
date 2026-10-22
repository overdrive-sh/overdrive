# D21-LOCAL / STEP 11-04 design review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `D21-LOCAL-STEP11-04-ARCH-REVIEW-01` |
| Reviewer | `nw-solution-architect-reviewer` |
| Iteration | 1 |
| Scope | Only the D21-LOCAL local fork artifact provisioning amendment in `feature-delta.md`, its STEP 11-04 roadmap reconciliation, and the bounded design handoff |
| User authority | "publish? what are u referring to? just keep it local wtf."; source-only clarification: "u can push" |
| Verdict | **APPROVED** |
| Blocking findings | 0 critical, 0 high |
| Informational findings | 1 documentation wording mismatch |

This approval covers the local source/build/stage/install substitution only. It
does not approve V-1(c), change STEP3-SPLIT, complete STEP 11-04, or authorize a
tag, GitHub release, draft release, uploaded asset, or other binary
publication.

## Artifacts reviewed

- `docs/feature/netns-density-295/feature-delta.md`: D21-LOCAL ruling, D21
  decision row, roadmap impact, and the VMM fork local provisioning contract.
- `docs/feature/netns-density-295/deliver/roadmap.json`: STEP 11-04 criterion,
  file scope, verification commands, and implementation notes.
- `docs/product/architecture/adr-0161-vendored-cloud-hypervisor-fork-is-built-pinned-and-provisioned-as-a-checksummed-release.md`.
- `docs/product/architecture/adr-0182-a-guest-reset-ends-the-vmm.md`.
- `.context/netns-density-295/11-04-design/local-artifact-provisioning-reconciliation.md`.
- `.context/netns-density-295/11-04/fork-release-candidate.md`, as explicitly
  superseded history only.
- Current provisioning owners: `infra/provision/versions.env`,
  `infra/provision/common-system.sh`, `infra/provision/common-user.sh`,
  `infra/lima/overdrive-dev.yaml`, and `infra/metal/provision.sh`.
- Current fork revision, remote branch, concrete local artifacts, and bounded
  native/Lima evidence listed below.

## Direct evidence

| Question | Evidence | Assessment |
|---|---|---|
| Is the source pin concrete and remotely available without publishing a binary? | `origin/overdrive/vhost-kernel-vsock` resolves to `f252bb20b0d141d59e4c15a9ac09c5c2dd7c350d`; the commit parent is `41a619d191f6472f79a3f22bcb9576c2a516906c`, and its only source change is the two-line RESET-EXIT dispatch in `vmm/src/lib.rs`. | Yes. The already-authorized source push is sufficient input for local provisioning. |
| Was any release publication created? | Targeted `git ls-remote` found no `v53.0-overdrive.*` tag, and `gh release list --repo overdrive-sh/cloud-hypervisor` returned no release or draft. | No tag, release, draft, or asset exists. |
| Does the checked-out fork identify the required source? | The fork worktree is at `f252bb20...`; the parent repository index still records `41a619d19...`, which is expected pre-implementation fallout and is explicitly required to advance by the design. | The source revision is available; STEP 11-04 must commit the parent submodule pointer and its mechanical consistency check. |
| Is the canonical source metadata-free and tied to the pinned commit? | On both metal and Lima, `<rev>/source/.git` is absent. SHA-256 for `vmm/src/lib.rs`, `Cargo.lock`, and `cloud-hypervisor/build.rs` is respectively `dc195c83...`, `a6594941...`, and `59066e93...`, exactly matching `git show f252bb20:<path>` locally. | Yes. The sampled identity-critical source files match the pinned tree, and no checkout history can affect `git describe`. |
| Does the exact x86_64 artifact exist at the designed path and identity? | `/var/tmp/overdrive/cloud-hypervisor/<rev>/artifacts/x86_64-unknown-linux-musl/cloud-hypervisor` is stripped static PIE, reports `cloud-hypervisor v53.0.0`, and hashes to `6d54fbd2611e692fda00e476fc0723e9502bfd8acd53c8bc46edd96e84f3e842`. | Yes. |
| Does the exact aarch64 artifact exist at the designed path and identity? | `/var/tmp/overdrive/cloud-hypervisor/<rev>/artifacts/aarch64-unknown-linux-musl/cloud-hypervisor` is stripped static aarch64 ELF, reports `cloud-hypervisor v53.0.0`, and hashes to `74195594db430695d4fbd9a8ecbc1d6d4b196f17c52afa6a3e96c967d86a91cc`. | Yes; compile-only status remains unchanged. |
| Is the toolchain/build substrate concrete? | Both hosts report Rust `1.89.0` (`29483883e`, LLVM 20.1.7), native GCC/musl wrapper 15.2.0, and GNU strip 2.46. The feature delta pins the target-specific Cargo arguments and exact `CARGO_TARGET_DIR`. | Yes for the two supported current hosts. A different result fails the checksum gate instead of being silently accepted. |
| Does the canonical x86_64 artifact satisfy existing native evidence? | With the canonical artifact first on `PATH`, the existing `vsock_vmm_device` selection passed 7/7, including the separately user-approved `post_boot_resets_end_the_vmm_and_reset_live_host_flows`; the four selected real `CloudHypervisorVmm`/host-state equivalence cells passed 4/4. | Yes. This is direct native-metal evidence against the canonical `6d54...` bytes. |
| Why is the earlier `30c56c...` artifact not the install identity? | It remains present under `/tmp/overdrive-295-rust189-bin` and differs from the canonical `6d54...` binary despite the same source/toolchain/target recipe. The canonical build fixes the source and target directories under `/var/tmp`. | The design correctly pins the proven canonical bytes and treats the earlier path-dependent candidate as superseded evidence. |
| Is the provisioning defect reachable now? | `common-system.sh` and the Lima system block still download stock v53.0. The currently installed metal and Lima binaries hash to `448af3d4...` and `f192b510...`, not the designed fork hashes. | Yes. The local provisioning substitution fixes a current, concrete identity mismatch. |

No redundant build was performed. The review reused the existing canonical
artifacts and ran only the bounded read-only identity checks and relevant
existing native cells above.

## Contract and architecture assessment

### No-weakening assessment

D21-LOCAL changes the transport of the build artifact, which the user's exact
direction expressly replaced. The following ADR-0161 invariants remain exact:

- one full source SHA carrying both `backend=vhost-kernel` and RESET-EXIT;
- the parent submodule pointer and provisioning pin agree;
- one exact artifact identity per supported architecture;
- SHA-256 gates staged bytes before install and installed bytes after install;
- no stock fallback and no acceptance by version string;
- no runtime fork-identification probe, no new probe stage, and no
  `VmmProbeError` variant;
- x86_64 retains native evidence and aarch64 remains compile-only.

The amendment strengthens reproducibility relative to an unconstrained local
build by fixing the metadata-free source directory, target directory,
toolchain, Cargo arguments, native strip step, staging path, and exact output
hashes. Rebuilt bytes that differ are refused and require a separately reviewed
checksum decision. No `zero`, `never`, `must`, `exactly`, fail-closed, or
forbidden-state outcome is relaxed apart from the explicitly superseded
release publication/download mechanism.

### Ownership and lifecycle

| Owner | Exact responsibility | Review result |
|---|---|---|
| `versions.env` | Metal source revision, Rust toolchain, repository, and both architecture hashes | Complete. |
| Lima parameter block | The same hand-maintained revision, toolchain, repository, and both hashes | Complete; the existing duplication model is preserved. |
| `common-system.sh` | Install system prerequisites; remove the upstream Cloud Hypervisor download and provide no stock fallback | Complete. The current upstream block proves the owner that must change. |
| `common-user.sh` | After system prerequisites and user Rust setup, build/stage/check/install on metal from the exact revision | Complete. Its existing repo-relative user lifecycle is the bounded owner; no new service or distribution subsystem is introduced. |
| Lima existing user provision block | Build/stage/check/install the aarch64 artifact locally after the system block supplies prerequisites | Complete. `musl-tools` is necessary compiler fallout in the existing package block, not a new mechanism. |
| Parent submodule plus consistency check | Make `vendors/cloud-hypervisor` equal `CLOUD_HYPERVISOR_FORK_REV` without contacting a release API | Complete and exact. |
| `/var/tmp/overdrive/cloud-hypervisor/<rev>` | Disposable source/build/artifact cache; never committed or uploaded | Complete and bounded. |
| `/usr/local/bin/cloud-hypervisor` | The sole installed destination, reusable only on an architecture-selected checksum match | Complete and unchanged from ADR-0161. |

The provisioning operation is a bounded change over the revision-scoped cache,
the pinned user toolchain/target, and `/usr/local/bin/cloud-hypervisor`. Its
declared complement excludes tags, releases, uploads, stock binaries, runtime
probe stages, product CLI, lanes, ports, and application APIs. The existing
roadmap exemption is appropriate for these external provisioning effects and
single-shot native checks; it does not weaken their fail-closed observable
contract.

### Public API and error contract

The only new public configuration surface is named exactly:
`CLOUD_HYPERVISOR_FORK_REPO`, `CLOUD_HYPERVISOR_FORK_REV`,
`CLOUD_HYPERVISOR_BUILD_TOOLCHAIN`,
`CLOUD_HYPERVISOR_SHA256_X86_64`, and
`CLOUD_HYPERVISOR_SHA256_AARCH64`, with the corresponding existing Lima
parameter entries. No method, type, trait, enum variant, CLI option, lane, wire
format, or persisted format is added.

The relevant interface is shell provisioning, whose established failure
representation is a nonzero exit with a diagnostic. The semantic set and
mapping are complete:

| Designed condition | Required result at the provisioning interface |
|---|---|
| Unsupported host architecture | Refuse with nonzero exit before build/install. |
| Exact revision unavailable or verified `HEAD` differs | Refuse with nonzero exit; do not substitute a branch tip or stock source. |
| Rust target/toolchain, native compiler, build, strip, or staging failure | Refuse with nonzero exit; do not install build-directory bytes. |
| Staged artifact missing or architecture checksum differs | Refuse with nonzero exit; do not re-pin or install. |
| Existing installed binary checksum differs | Do not accept it; run the pinned local build path, then install only verified staged bytes. |
| Install failure or installed checksum differs | Refuse with nonzero exit; do not report provisioning complete. |
| Installed bytes match | Reuse is allowed, then run the existing `--landlock` presence check. |

The design does not require a typed application error for these shell outcomes.
It explicitly preserves the application typed-error boundary: no provisioning
error variant and no `VmmProbeError` variant identify the fork. There is no
`SPECIFICATION_AMBIGUITY`.

## Architecture review dimensions

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass | The amendment introduces no technology or subsystem. It uses the existing source repository, provisioning scripts, rustup/Cargo, Lima block, and install path. |
| Decision quality | Pass | Context is the user's rejection of publication; alternatives and retained invariants remain in ADR-0161; consequences and revision lifecycle are explicit in the amendment. |
| Completeness | Pass | Both architectures, exact hashes, source/build/stage/install paths, owners, refusal set, cache lifecycle, submodule agreement, and no-runtime-probe boundary are specified. |
| Feasibility and testability | Pass | Both canonical artifacts exist; x86_64 native cells pass 7/7 plus 4/4; aarch64 builds at the exact path and has the pinned checksum. |
| Priority/data justification | Pass | Current stock installs have the wrong identities; the canonical fork artifacts and source pin are directly observed. The amendment addresses only the proven provisioning gap. |

## Roadmap checks for the D21-LOCAL amendment

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | Metal and Lima verification commands observe the installed binary at `/usr/local/bin/cloud-hypervisor`; the native suite invokes the production `CloudHypervisorVmm`. |
| Acceptance-criterion coupling | Pass | Exact paths, pins, owners, toolchain, build arguments, and checksums are external provisioning and artifact-identity contracts. No private method or invented application type is prescribed. |
| Step decomposition | Pass | One bounded substitution spans the existing pin, system, user, Lima, metal, and submodule owners; splitting it would permit an intermediate stock or unchecked install. |
| Implementation code in roadmap | Pass | The roadmap carries a reproducible build recipe and black-box verification commands, not an implementation algorithm or method body. |
| Concision and precision | Pass for this amendment | The criterion is long because every checksum, path, owner, refusal, and publication prohibition is contract-bearing. Removing them would make the accepted local substitution ambiguous. |
| Test boundary | Pass | Installed-file checksum checks are black-box provisioning evidence; native KVM cells use production composition; the amendment adds no Rust-test/process-boundary violation. |

## Findings and dispositions

### I-01 — stale authorization wording in STEP 11-04 notes

- **Severity:** Informational; non-blocking.
- **Claim:** STEP 11-04 currently calls
  `post_boot_resets_end_the_vmm_and_reset_live_host_flows` a
  "still-proposal-only post-boot reset cell" and says the run approves neither
  pending proposal. The user separately approved this one RESET-EXIT test; only
  V-1(c) remains unapproved.
- **Reachability/evidence:** The test is present and live in
  `vsock_vmm_device.rs`; `.context/netns-density-295/11-04/fork-source-push.md`
  records the approval and RED/GREEN history; the recorded run passed 1/1, and
  this review's canonical-artifact selection passed 7/7 including that cell.
- **Reproduction:** Read the STEP 11-04 `implementation_notes`, then compare the
  quoted phrase with the source-push record and the live test selection.
- **Impact:** Documentation only. It does not make the executed test
  unauthorized, does not change D21-LOCAL, and does not approve V-1(c).
- **Remediation:** When the roadmap note is next reconciled, replace the stale
  phrase with "user-approved post-boot reset cell" and state specifically that
  V-1(c) remains unapproved. No test, production, API, or local provisioning
  change follows from this correction.
- **Disposition:** Accepted as follow-up wording cleanup; does not block design
  approval or STEP 11-04 resumption.

No critical, high, or medium finding was proven. Speculative cross-distribution
reproducibility hardening, a binary distribution service, an additional
runtime capability probe, and new typed provisioning errors have no reproduced
need in the current production/provisioning path and are outside the approved
scope.

## Verdict

**APPROVED.** D21-LOCAL is an exact, feasible, fail-closed local substitution
for ADR-0161's release publication/download mechanism. The authoritative
feature-delta amendment and STEP 11-04 reconciliation preserve the required
source pin/submodule agreement, both architecture identities, checksum gate,
stock-fallback prohibition, and runtime API/error boundary. The original STEP
11-04 crafter may implement this approved amendment within the existing step.
