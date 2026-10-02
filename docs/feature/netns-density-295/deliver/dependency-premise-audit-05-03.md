# Step 05-03 dependency premise audit

## Metadata

- Feature: `netns-density-295`
- Step: `05-03`
- Date: 2026-10-02
- Role: isolated solution architect, bounded documentary premise audit
- Verdict: **A — no contradiction of the accepted design**
- Scope: accepted dependency paragraphs and ADRs, Cargo manifests, lockfile comparison, and Cargo registry dependency metadata. No production or test source was read or changed.

## Question and inputs

The original crafter stopped during GREEN after adding the prescribed `command-fds` dependency. Cargo.lock gained `command-fds` 0.3.3 and a second `nix` version, 0.31.3, and replaced `libc` 0.2.185 with 0.2.189. The claimed blocker was that these results disproved an accepted whole-graph invariant that no new crate enters the lockfile.

The audit read the solution-architect role and loaded `nw-architecture-patterns` and `nw-sa-critique-dimensions`. Its decision evidence is:

- [Feature delta](../feature-delta.md), lines 769–789, 1074–1091, 15284–15286, and 15345–15348.
- [ADR-0128](../../../product/architecture/adr-0128-vmm-adapter-owns-per-launch-tap-queue-descriptor.md), accepted status at lines 5–7, queue ownership at lines 42–67, and dependency consequence at lines 98–99.
- [ADR-0129](../../../product/architecture/adr-0129-safe-descriptor-mapping-for-vmm-launch.md), accepted status at lines 5–7, decision at lines 56–88, alternatives at lines 98–109, and dependency consequences at lines 137–175.
- Workspace `Cargo.toml`, lines 65, 165, and 181; `crates/overdrive-host/Cargo.toml`, lines 45, 88, 91, and 111.
- Current `Cargo.lock`, compared with `git show HEAD:Cargo.lock`.
- Cached `command-fds` 0.3.3 Cargo manifest and the exact `nix` 0.31.3 registry dependency entry. Only dependency metadata was inspected.

## Normative scope finding

The feature delta explicitly authorizes a new dependency before discussing `libc`:

> `overdrive-host/Cargo.toml` gains `overdrive-netlink.workspace = true`, and `command-fds` as a workspace dependency with its `tokio` feature.

This is at lines 778–779. Lines 780–786 close its dependency-review gate through research F1.5, identify version 0.3.3, and assign compilation confirmation to DELIVER.

The subsequent sentence, at lines 786–789, has a different subject:

> `libc` moves from `overdrive-host`'s `[dev-dependencies]` to its `[dependencies]` (still `libc.workspace = true`) for the D-295-R22 program types and constants; no new crate enters the lockfile.

The clause after the semicolon qualifies the `libc` relocation and D-295-R22 program choice. Relocating an already-locked crate does not introduce a new crate. It does not forbid the `command-fds` addition expressly authorized in the preceding sentence.

Three accepted records independently resolve the scope:

1. The accepted D-295-R3 entry at feature-delta line 15286 requires `command-fds` mapping and records its dependency gate as closed. Lines 15345–15348 repeat that DELIVER must confirm its Tokio extension by compilation.
2. Accepted ADR-0129 lines 59–60 prescribe the `command-fds` extension. Its negative consequence at line 154 explicitly states: **“A new third-party dependency enters the security-sensitive VMM launch path.”** A whole-graph prohibition would contradict this accepted consequence as well as the prescribed mechanism.
3. ADR-0129 lines 166–175 expressly discuss the **`command-fds` 0.3.3 → `nix` 0.31.3** dependency chain. The second `nix` version is therefore already named in the accepted dependency analysis.

The D-295-R22 alternative comparison also supports the narrow reading. Feature-delta lines 1088–1091 say the hand-built seccomp program adds no dependency because it uses the already-locked `libc`, whereas `seccompiler` would introduce an unreviewed dependency. That is a comparison of seccomp program builders. It is not a prohibition on the independently reviewed D-295-R3 descriptor-mapping dependency.

**The alleged whole-dependency-graph invariant is not the contract these records establish.** This audit records the existing scope; it neither removes nor relaxes “no new crate enters the lockfile.”

## Reproduction and dependency facts

The existing working-tree lockfile change was reproduced mechanically by comparing parsed package identities against HEAD and inspecting the exact resolved edges. This audit did not rerun dependency resolution, compile code, or execute a runtime spike; no runtime, security, or public API failure was alleged in this blocker.

| Fact | Evidence | Assessment |
|---|---|---|
| `command-fds` 0.3.3 is newly locked | `Cargo.lock:754–761`; lockfile diff | Direct implementation of the accepted addition. Its resolved dependencies are `nix` 0.31.3, existing `thiserror` 2.0.18, and existing `tokio`. |
| `nix` 0.30.1 remains; 0.31.3 is added | `Cargo.lock:1981–2002`; lockfile diff | Workspace `nix` remains `0.30` at `Cargo.toml:181`. The cached `command-fds` manifest requires `nix` `0.31.2` at lines 48–50; that requirement selects the 0.31 series and cannot use 0.30.1. ADR-0129 already records 0.31.3. |
| The existing `libc` package changes version | `Cargo.lock:1690–1693`; HEAD had 0.2.185 | It remains the same crate, with one locked version. The new version is 0.2.189. |
| The old `libc` version cannot satisfy the selected `nix` | Cached registry entry for `nix` 0.31.3: normal `libc` dependency requirement `^0.2.186` | 0.2.185 is below the required minimum. An update is necessary for this resolved dependency chain. |
| The workspace permits this `libc` update | `Cargo.toml:165`: `libc = "0.2"`; host line 88: `libc.workspace = true` | 0.2.189 satisfies the workspace requirement. The design's reference to the then-locked 0.2.185 is not an exact-version constraint in the specified workspace manifest surface. |

The complete package-identity comparison found additions `command-fds` 0.3.3, `nix` 0.31.3, and `libc` 0.2.189, and removal only of `libc` 0.2.185. Thus the change adds one new crate name, one additional version of an existing crate, and updates one existing crate. No other package update appears in the inspected lockfile diff.

The exact 0.2.189 patch is not asserted to be the only eligible `libc` version. The evidence establishes that retaining 0.2.185 with the selected `nix` 0.31.3 is impossible and that the actual replacement satisfies the accepted workspace constraint. It does not establish an avoidable churn defect requiring remediation, a version substitution, or a separate dependency policy.

Registry metadata location: `/Users/marcus/.cargo/registry/index/index.crates.io-1949cf8c6b5b557f/.cache/3/n/nix`, entry `vers = 0.31.3`, normal dependency `name = libc`, `req = ^0.2.186`. Cached manifest location: `/Users/marcus/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/command-fds-0.3.3/Cargo.toml`.

## Invariants and disposition

The original crafter **can resume step 05-03 against the exact accepted implementation contract**, with the prescribed dependency additions and their necessary Cargo resolution fallout. This audit grants no new API surface and does not approve the partial implementation; the step's implementation and dedicated review gates still apply.

The accepted requirements remain:

- The TAP remains administratively down through the specified boot barrier, with zero frames before activation (feature-delta lines 766–768 and accepted R1 at line 15284).
- `CloudHypervisorVmm::create` owns one per-launch queue and drops the command holding its parent copy before any further await, on every branch (ADR-0128 lines 42–59; feature-delta lines 769–775).
- With a queue, the child inherits exactly descriptors 0–3; without one, exactly descriptors 0–2. Mapping, close-on-exec, and filter installation retain the accepted order (ADR-0129 lines 56–88).
- `VmNetworkAttachment`, `VmConfig`, and the `Vmm` trait retain the accepted shapes and ownership boundaries (ADR-0128 lines 61–67).
- The approved `command-fds` choice and the existing `libc.workspace = true` contract remain in force.

The reproduced dependency additions do not falsify an actual accepted premise. **Native-falsification invalidation is not triggered:** no dependent decision or roadmap validation needs to be returned to pending on this evidence, and no replacement contract or new approval is needed. The alternative rejections remain applicable; in particular, ADR-0129 rejects an in-crate `dup2` for collision handling and unnecessary additional audited unsafe code, not because the chosen dependency would add nothing to the lockfile.

Only this audit artifact was written. All pre-existing dirty files, partial step 05-03 work, DES events, roadmap validation, and normative design text were preserved. No phase event or commit was created, and this audit did not advance the workflow.
