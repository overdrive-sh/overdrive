# Bugfix RCA: cgroup preflight conflates `/proc/filesystems` I/O errors with cgroup-v1-only host

**Status**: User-approved 2026-04-29. Approved fix shape is **propagate I/O errors via a new `ProcFilesystemsUnreadable` variant; map `io::ErrorKind::NotFound` to the existing `NoCgroupV2` (NotFound IS the v1-host signal); every other `ErrorKind` surfaces as the new variant**.

User also codified the underlying principle into `.claude/rules/development.md` (Errors section): "Distinct failure modes get distinct error variants. Never silently absorb a `Result<_, io::Error>` into a default value." This RCA is the worked example of that rule.

---

## Bug summary

`cgroup_preflight::run_preflight_at` (`crates/overdrive-control-plane/src/cgroup_preflight.rs:189`) reads `/proc/filesystems` via `std::fs::read_to_string(proc_filesystems).unwrap_or_default()`. The `unwrap_or_default()` collapses **every** `io::Error` into the empty string. The next line — `proc_fs.lines().any(|line| line.contains("cgroup2"))` — returns `false` on an empty string, so `cgroup_v2_available` is `false` and the function returns `CgroupPreflightError::NoCgroupV2 { kernel: uname_release() }`.

The operator then sees a message that names the wrong cause and prescribes the wrong remediation:

> cgroup v2 not available on this kernel (uname: …).
>
> Detected: /proc/filesystems does not list `cgroup2`.
> Phase 1 of Overdrive requires cgroup v2; cgroup v1 hosts are not supported.
>
> Try one of:
>
>   1. Boot a kernel with cgroup v2 unified hierarchy enabled …

…when the *actual* failure was a `PermissionDenied` (or `EIO`, or "/proc not mounted in this container", or any other transient procfs failure). "Boot a newer kernel" does not fix a permissions error.

The shape mirrors the existing-and-correct handling in step 4 (`cgroup_preflight.rs:211-212`), which converts the `read_to_string` error from `/proc/self/cgroup` into `CgroupPreflightError::CgroupPathDiscoveryFailed { source }`. Step 1 was written with `unwrap_or_default()` — likely because the author conflated "file is missing" (a legitimate cgroup-v1-only-or-stripped-kernel signal) with "file unreadable" (a separate failure shape needing its own diagnosis).

## Root cause chain (3 compounding causes)

### A. Production code absorbs every `io::Error` into the empty string

- `run_preflight_at` line 189: `let proc_fs = std::fs::read_to_string(proc_filesystems).unwrap_or_default();`
- `unwrap_or_default()` on `Result<String, io::Error>` returns `String::new()` for **any** `io::ErrorKind` — `PermissionDenied`, `Unsupported`, `Interrupted`, custom `Other`, the lot. There is no branch that distinguishes them.
- The next check (line 190) cannot see the difference between "successfully read but no cgroup2 line" and "could not read at all" — both produce `false`.
- Step 4 elsewhere in the same file (line 211-212) handles the analogous read of `/proc/self/cgroup` correctly via `.map_err(|err| CgroupPathDiscoveryFailed { source: err })?`. The two reads use opposite patterns; only one is right.

### B. The error variant's docstring describes one cause; its triggering path fires for many

- `CgroupPreflightError::NoCgroupV2`'s docstring (`cgroup_preflight.rs:38-39`) and Display message both name a single cause: `/proc/filesystems` does not list `cgroup2`.
- The actual triggering condition in `run_preflight_at` is "`proc_fs` is empty OR contains no `cgroup2` line." Because of cause (A), "empty" includes every I/O failure mode, which has nothing to do with the kernel exposing cgroup v2.
- This is the precise smell the new development.md rule names: a variant whose docstring describes one failure mode but whose triggering code path fires for several. The variant has become a catch-all.

### C. No regression test exercises the unreadable-procfs shape

- `tests/integration/cgroup_isolation/preflight_v1_host.rs` writes a *valid* `/proc/filesystems` analogue without `cgroup2` (a real v1-host signal) and asserts `NoCgroupV2`. This is correct behaviour and remains correct under the fix.
- No test fabricates an *unreadable* `/proc/filesystems` (e.g., `chmod 0o000` inside a `tempfile::TempDir`, or a path pointing at a directory rather than a file). So the I/O-error path is structurally invisible to the test suite.
- The reviewer's catch-the-bug mechanism was code review, not the test suite — exactly the "tests pass, but they don't actually defend the thing that matters" shape that motivates `cargo-mutants` discipline.

## Approved fix: new `ProcFilesystemsUnreadable` variant; `NotFound` keeps the v1-host fallthrough

**Rejected: "treat every `io::Error` as `ProcFilesystemsUnreadable`"** — would re-route `NotFound` (file genuinely absent) into the new variant. A missing `/proc/filesystems` is itself the cgroup-v1-host signal on a stripped or non-procfs kernel, and the existing `preflight_v1_host.rs` test relies on that semantics. Mapping `NotFound` to `NoCgroupV2` matches operator intuition and keeps the existing test green.

**Rejected: "use `?` and a `From<io::Error>` blanket impl"** — would propagate every error including `NotFound`, with the same problem as above, and would force the variant to encode every distinct kind in its `Display` message. The match-on-kind shape gives the application explicit control over the semantic mapping.

### Code changes

1. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — new error variant `ProcFilesystemsUnreadable`:**
   - Wraps `std::io::Error` via `#[source]` (parallel to `CgroupPathDiscoveryFailed`).
   - Display form names the failure ("could not read /proc/filesystems"), the cause (the wrapped I/O error), and surfaces `--allow-no-cgroups` as the dev escape hatch — same shape as the other variants per `nw-ux-tui-patterns`.
   - The Display does **not** prescribe "boot a newer kernel" — that is the specific misdiagnosis we are correcting.

2. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — replace step 1 read at line 189:**

   ```rust
   // Before
   let proc_fs = std::fs::read_to_string(proc_filesystems).unwrap_or_default();

   // After
   let proc_fs = match std::fs::read_to_string(proc_filesystems) {
       Ok(s) => s,
       // NotFound IS the v1-host signal — fall through to the
       // cgroup_v2_available = false branch below, which returns
       // NoCgroupV2 with the kernel-upgrade remediation.
       Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
       Err(err) => return Err(CgroupPreflightError::ProcFilesystemsUnreadable { source: err }),
   };
   ```

   The downstream `cgroup_v2_available` check is unchanged — `NotFound` continues to flow through `NoCgroupV2`; every other `io::ErrorKind` exits early via the new variant.

### Test additions (new — must defend the fix)

3. **NEW: `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_proc_filesystems_unreadable.rs`** — fabricate an unreadable `/proc/filesystems` analogue inside a `tempfile::TempDir`. The cleanest fixture shape is to write the file then `chmod 0o000` (under `#[cfg(target_os = "linux")]`, which the integration test module already requires). Run preflight with `uid = 1000` (non-root, so the path is reachable; root would short-circuit before step 1 anyway — no, wait, step 3 short-circuits step 4, not step 1; root still runs step 1 the same way). The assertion: `match err { ProcFilesystemsUnreadable { source } => { assert!(source.kind() == PermissionDenied || source.kind() == Other, "...") } other => panic!("expected ProcFilesystemsUnreadable, got {other:?}") }`. Verify the rendered message NAMES the I/O error and does NOT mention "boot a newer kernel".

   Edge case: under root the `chmod 0o000` doesn't actually deny reads. The test must either run as a non-root UID (which is fine for an integration test under nextest) OR fabricate the unreadability via a different shape — pointing `proc_filesystems` at a directory path (so `read_to_string` returns `IsADirectory` / `Other`) is the portable shape that works regardless of test-process UID.

   **Decision**: use the directory-path fixture. It is portable across nextest's UID surface (Lima root, CI non-root, macOS dev `--no-run`), gives a deterministic `io::ErrorKind`, and exercises the same code path the bug report describes.

### Test that must stay green (existing)

4. **`crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_v1_host.rs`** — unchanged. The fixture writes a valid `/proc/filesystems` analogue without `cgroup2`; `read_to_string` returns `Ok(non_empty_string)`; `cgroup_v2_available` is `false`; `NoCgroupV2` fires. The fix does not touch this path.

   Belt-and-braces: also confirm `NotFound` (i.e., the proc_filesystems path does not exist at all) still flows to `NoCgroupV2` — either by extending `preflight_v1_host.rs` with a second `#[test]` that points `proc_filesystems` at a path that does not exist, or by adding a new file. The first is lighter; do it inline.

## Risk assessment

**LOW.** Strict refinement:

- Previously-silently-swallowed I/O errors now produce a discrete diagnosis with the actual cause in the rendered message — strictly more informative.
- `NotFound` continues to flow through `NoCgroupV2`, so the existing `preflight_v1_host.rs` test passes unchanged.
- No public API surface change: the new variant is added to a non-exhaustive (or about-to-be-non-exhaustive — confirm during implementation) error enum; existing match arms continue to compile.
- No behaviour change for callers that already trip `NoCgroupV2` legitimately (cgroup-v1-only host, stripped kernel without cgroup2 line in a readable procfs).

The only operators who see different output after the fix are those who were *already* hitting an I/O error on `/proc/filesystems` and getting misled by the v1-host message. They will now see the correct diagnosis and the correct remediation.

## Files affected

- `crates/overdrive-control-plane/src/cgroup_preflight.rs` — new variant + line-189 match.
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_proc_filesystems_unreadable.rs` — NEW.
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_v1_host.rs` — optionally extend with a `NotFound` belt-and-braces test (`#[test]` named e.g. `preflight_treats_missing_proc_filesystems_as_v1_host`).
- `crates/overdrive-control-plane/tests/integration.rs` — register the new module under `mod cgroup_isolation { … }`.

No changes to:
- The `CgroupPathDiscoveryFailed` variant (already correct).
- The `NoCgroupV2` variant Display (still correct for its actual cause).
- The `run_preflight()` production wrapper (already passes `Path::new("/proc/filesystems")`).
- Any callers of `run_preflight_at` outside the test surface.

## Cross-references

- `.claude/rules/development.md` § Errors — the new "Distinct failure modes get distinct error variants" rule cites this exact incident (`cgroup_preflight.rs` line 189) as the worked example.
- `.claude/rules/testing.md` § "Integration vs unit gating" — the new test goes under `tests/integration/cgroup_isolation/` and is gated by the crate's `integration-tests` feature, same as its siblings.
- ADR-0028 — the cgroup-delegation pre-flight design doc; this fix improves diagnostic accuracy without changing the §4 step-1 semantics.
- Sibling RCA: `docs/feature/fix-cgroup-preflight-wrong-slice/bugfix-rca.md` — different bug in the same file (step 4 reads the wrong slice). That fix landed at `4d65fb8`. This RCA is independent of it.
