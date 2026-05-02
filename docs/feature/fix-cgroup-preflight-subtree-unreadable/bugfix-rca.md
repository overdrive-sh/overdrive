# Bugfix RCA: cgroup preflight conflates step-4 `cgroup.subtree_control` I/O errors with `DelegationMissing`

**Status**: User-approved 2026-04-29. **Approved fix shape: Option B** — every `io::Error` from the step-4 `cgroup.subtree_control` read (including `NotFound`) surfaces via a new `SubtreeControlUnreadable { slice: PathBuf, source: io::Error }` variant. NotFound is NOT mapped to `DelegationMissing` via String::new fallthrough. Rationale: the kernel guarantees `cgroup.subtree_control` exists under every cgroup-v2 directory, so its absence indicates the enclosing slice path is not a cgroup directory at all — structurally distinct from "valid path with zero delegated controllers."

This RCA is the second worked example of the `.claude/rules/development.md` § Errors rule "Distinct failure modes get distinct error variants. Never silently absorb a `Result<_, io::Error>` into a default value" — the rule that landed in commit `d28e56f` alongside the step-1 fix (`fix-cgroup-preflight-procfs-unreadable`).

---

## Bug summary

`run_preflight_at` step-4 read at `crates/overdrive-control-plane/src/cgroup_preflight.rs:273` calls `std::fs::read_to_string(&subtree_control).unwrap_or_default()`. The `unwrap_or_default()` collapses every `io::Error` into the empty string. The next lines token-scan `contents` for `cpu` and `memory`; neither is found in the empty string, so both push into the missing list, and `DelegationMissing { slice, missing: ["cpu", "memory"], … }` returns with this Display message:

> cgroup v2 delegation required.
>
> Overdrive serve needs the cpu and memory controllers delegated to UID …
>
> Detected: cgroup v2 IS available, BUT cpu and memory are not in
> the subtree_control of …
>
> Try one of:
>
>   1. Run via the bundled systemd unit (production):
>        systemctl --user start overdrive
>
>   2. Grant delegation manually (one-time):
>        sudo systemctl set-property user-{uid}.slice Delegate=yes
>        systemctl --user daemon-reload
> …

…regardless of whether the actual failure was a `PermissionDenied` on the file, an `EIO` from the cgroupfs, a transient I/O error, or a `NotFound` indicating the enclosing slice path doesn't exist as a cgroup directory at all. "Run `systemctl set-property … Delegate=yes`" does not fix any of those.

The shape mirrors the just-landed step-1 fix (`fix-cgroup-preflight-procfs-unreadable`, commit `941fb3e`): same anti-pattern, same file, different failure mode. The reviewer of that PR explicitly flagged this line as a follow-up candidate.

## Root cause chain (3 compounding causes — same shape as step-1)

### A. Production code absorbs every `io::Error` into the empty string

- Line 273: `let contents = std::fs::read_to_string(&subtree_control).unwrap_or_default();`
- `unwrap_or_default()` on `Result<String, io::Error>` returns `String::new()` for **any** `io::ErrorKind` — `PermissionDenied`, `Unsupported`, `Interrupted`, `NotFound`, `IsADirectory`, custom `Other`, the lot. There is no branch that distinguishes them.
- The next check (lines 275-280) cannot tell "successfully read empty file" from "could not read at all."
- The structurally adjacent step-1 read (now lines 226-231 post-fix) and the step-4 `proc_self_cgroup` read (lines 253-254) handle their I/O correctly via discrete variants. Only this read remains on the absorbing pattern.

### B. The error variant's docstring describes one cause; its triggering path fires for many

- `CgroupPreflightError::DelegationMissing`'s docstring (`cgroup_preflight.rs:81-84`) and Display message (lines 85-110) name a specific cause: *the parent slice's `subtree_control` lacks one or both of `cpu`/`memory`*.
- The actual triggering condition in `run_preflight_at` is "`contents` is empty OR doesn't contain `cpu` AND/OR doesn't contain `memory`." Because of cause (A), "empty" includes every I/O failure mode — not the cause the variant claims to represent.
- This is the precise smell the new `development.md` rule names: a variant whose docstring describes one failure mode but whose triggering code path fires for several. The variant has become a catch-all.

### C. No regression test exercises the unreadable-`subtree_control` shape

- Existing step-4 tests (`preflight_no_delegation.rs`, `preflight_missing_cpu.rs`, `preflight_reads_enclosing_slice.rs`) write a *valid* `cgroup.subtree_control` file with various contents (empty, missing one controller, all controllers). All three paths flow through `Ok(non_empty | empty_string)`. None fabricate an *unreadable* `cgroup.subtree_control`.
- The reviewer's catch-the-bug mechanism on the step-1 fix was code review; the same pattern survived step-4 unflagged for the same reason. The structural fix is to land regression tests that close the test-surface gap.

## NotFound semantics — why Option B (and not Option C)

The kernel guarantees `cgroup.subtree_control` exists under every cgroup-v2 directory (it's a kernel-created interface file per `Documentation/admin-guide/cgroup-v2.rst`). Therefore `Err(NotFound)` on this read does NOT mean "no controllers delegated" — it means **the path is not a cgroup directory**.

The asymmetry with the step-1 fix:

| Case | step-1 (`/proc/filesystems`) | step-4 (`cgroup.subtree_control`) |
|---|---|---|
| `Ok(non_empty)` with feature line | feature available | controllers delegated |
| `Ok(empty)` / `Ok(no feature line)` | feature absent (= v1-host signal) | no controllers delegated (= `DelegationMissing`) |
| `Err(NotFound)` | feature absent on stripped kernel — **legitimate v1-host fallthrough** | path is not a cgroup directory — **structurally anomalous** |
| `Err(other)` | I/O error on procfs | I/O error on cgroupfs |

`/proc/filesystems` can legitimately be absent (stripped kernel without procfs entries for cgroup support) — that is genuinely the same application-semantic state as "cgroup2 not listed." So step-1 maps `NotFound → NoCgroupV2` via the empty-string fallthrough, and the new development.md rule explicitly authorises that absorption.

`cgroup.subtree_control` cannot legitimately be absent under a real cgroup directory. Its absence indicates either (a) a race between the `/proc/self/cgroup` read and the slice being unmounted/destroyed, or (b) `cgroup_root` is misconfigured (pointing at a non-cgroupfs path), or (c) the parsed enclosing-slice path doesn't correspond to a real directory. None of these are "delegation missing" — they're "your enclosing slice is not a cgroup directory."

The new development.md rule's escape clause says: absorb a specific `ErrorKind` into a default *only when application semantics legitimately treat that kind the same as the default*. NotFound on `cgroup.subtree_control` and an empty `cgroup.subtree_control` are NOT the same application-semantic state. Therefore Option B (surface NotFound as the new variant) is the correct application of the rule.

## Approved fix: new `SubtreeControlUnreadable` variant; every `io::Error` surfaces via the new variant

**Rejected: Option C** ("NotFound flows to `DelegationMissing` via `String::new()` fallthrough, mirroring step-1's NotFound→NoCgroupV2 shape") — pragmatically the operator-facing remediation overlaps, but the diagnosis is wrong. The new development.md rule's escape clause does not authorise this absorption because the semantics differ.

### Code changes

1. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — new error variant `SubtreeControlUnreadable`:**
   - Wraps `std::io::Error` via `#[source]` AND captures the slice path in a `slice: PathBuf` field (parallel to `DelegationMissing.slice` for operator triage).
   - Display form names the failure ("could not read `<slice>/cgroup.subtree_control`"), embeds the source via `{source}`, and surfaces remediation appropriate to the failure shape (verify cgroupfs mount; check that the enclosing slice exists; run as root or use `--allow-no-cgroups`).
   - **Critically** the Display message does NOT mention "`Delegate=yes`" or "delegation required" — those phrases are reserved for `DelegationMissing` and are exactly the misdiagnosis we are correcting.

2. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — replace step-4 read at line 273:**

   ```rust
   // Before
   let contents = std::fs::read_to_string(&subtree_control).unwrap_or_default();

   // After (Option B)
   let contents = std::fs::read_to_string(&subtree_control).map_err(|err| {
       CgroupPreflightError::SubtreeControlUnreadable {
           slice: enclosing_abs.clone(),
           source: err,
       }
   })?;
   ```

   The downstream missing-controllers check (lines 275-294) is unchanged — it now only ever runs against an actually-readable `cgroup.subtree_control`.

### Test additions (new — must defend the fix)

3. **NEW: `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_subtree_control_unreadable.rs`** — fabricate an unreadable `cgroup.subtree_control` analogue inside a `tempfile::TempDir` using the directory-path fixture (`mkdir <tmp>/<slice>/cgroup.subtree_control` instead of `write`). The fixture also seeds `<tmp>/cgroup.controllers` (step-2 existence check) and a fake `proc_self_cgroup` containing `0::/<slice>`. Run with non-zero `uid`. Assert via match that the error is `SubtreeControlUnreadable { slice, source }`, `source.kind() != NotFound` (anything but NotFound — IsADirectory on Linux 6+, Other on older), `slice` equals the expected enclosing-slice path. Assert the rendered message contains `--allow-no-cgroups` and `docs.overdrive.sh`. Assert NEGATIVELY that the message does NOT contain "`Delegate=yes`" or "delegation required" (those are reserved for `DelegationMissing`).

4. **NEW (or extend `preflight_no_delegation.rs`): NotFound regression test** — fabricate a fixture where the enclosing-slice directory exists but `cgroup.subtree_control` does NOT (don't create the file). Assert `SubtreeControlUnreadable { source }` fires with `source.kind() == NotFound`, NOT `DelegationMissing`. This is the test that pins Option B.

   Decision: place this in a separate file `preflight_subtree_control_missing_is_not_delegation.rs` adjacent to the I/O-error test rather than extending `preflight_no_delegation.rs`. Keeping the two regression tests adjacent and named for what they actually defend is clearer than overloading a file whose existing test asserts a different variant.

### Tests that must stay green (existing)

5. **`preflight_no_delegation.rs`**, **`preflight_missing_cpu.rs`**, **`preflight_reads_enclosing_slice.rs`** — unchanged. All write a real `cgroup.subtree_control` file with various contents; the `Ok` branch flows naturally through the missing-controllers logic.
6. **`preflight_v1_host.rs`**, **`preflight_proc_self_cgroup_malformed.rs`**, **`preflight_proc_filesystems_unreadable.rs`** — unchanged. Step 4 is never reached in these tests.

## Risk assessment

**MEDIUM** under Option B (the approved shape).

- The behaviour change for operators in the NotFound corner case (enclosing-slice path discovered from `/proc/self/cgroup` does not contain a `cgroup.subtree_control` file) is from "wrong remediation" (`Delegate=yes`) to "right remediation" (verify cgroupfs configuration). This is a refinement, not a regression — but it IS observable.
- Operators in the PermissionDenied / EIO / IsADirectory / Other corner cases get strictly more informative output — pure refinement.
- Operators in the regular case (file readable, contents inspected) see no change at all — unchanged behaviour.
- The `--allow-no-cgroups` dev escape hatch remains the universal off-ramp regardless of which variant fires.

The "MEDIUM" rating reflects the NotFound semantic shift, which the user explicitly opted into (Option B over Option C). All other shifts are LOW-risk refinements.

## Files affected

- `crates/overdrive-control-plane/src/cgroup_preflight.rs` — new variant + line 273 match.
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_subtree_control_unreadable.rs` — NEW.
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_subtree_control_missing_is_not_delegation.rs` — NEW (NotFound regression test pinning Option B).
- `crates/overdrive-control-plane/tests/integration.rs` — register the two new modules under `mod cgroup_isolation { … }`.

No changes to:
- `DelegationMissing` variant (still correct for its actual cause: empty or partial subtree_control contents).
- `CgroupPathDiscoveryFailed` (already correct).
- `ProcFilesystemsUnreadable` (just-landed, correct).
- `NoCgroupV2` / `NotMounted` (unrelated).
- The `run_preflight()` production wrapper.
- Any callers of `run_preflight_at` outside the test surface.

## Cross-references

- `.claude/rules/development.md` § Errors — "Distinct failure modes get distinct error variants" rule. This RCA is the second worked example.
- Sibling RCA: `docs/feature/fix-cgroup-preflight-procfs-unreadable/bugfix-rca.md` — same file, same anti-pattern, step 1 instead of step 4. Landed in commits `d28e56f` (RED) and `941fb3e` (GREEN).
- ADR-0028 — the cgroup-delegation pre-flight design doc; this fix improves diagnostic accuracy of step 4 without changing its §4 step-4 semantics (controllers-missing → `DelegationMissing`).
