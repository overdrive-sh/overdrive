//! S-VM-85 (step 03-09, DWD-26 / ADR-0083 §§D3f-D3h, GH #42) — the
//! clone-index link outlives the clone it points at, on every
//! interleaving. `VmDriver`'s component-scope acceptance suite for the
//! clone-index ordering invariant, against `SimVmm` over a REAL
//! filesystem.
//!
//! Component scope, the same carve-out ADR-0082 §D4 already justifies for
//! S-VM-76 (`vm_driver_stop_totality.rs`): `SimVmm` is injected at the
//! `Vmm` port boundary and a real `tempfile::TempDir` pair supplies the
//! clone-index directory and the operator rootfs-master directory as two
//! distinct real directories. There is NO guest boot — nothing dials the
//! beacon, nothing spawns cloud-hypervisor — so this is deliberately NOT
//! `@requires-kvm` and runs under Lima in the default lane, exactly like
//! `vm_driver_stop_totality.rs`. It is registered in
//! `crates/overdrive-worker/tests/acceptance.rs`.
//!
//! RED scaffold pending step 03-09. The production clone-index surface it
//! pins — the platform-owned index directory
//! (`clone_index_dir(data_dir)` = `<data_dir>/vm/clone-index/`, ADR-0083
//! §D3g), the create-before / remove-after symlink ordering (§D3f), and
//! the `discard_artifacts` read-the-link resolution (§D3h) — does not
//! exist at current HEAD. The scaffold therefore panics BEFORE touching
//! any of it, per `.claude/rules/testing.md` § "RED scaffolds and
//! intentionally-failing commits", and stays discoverable via
//! `grep -rn 'should_panic.*RED scaffold' crates/`. DELIVER (step 03-09)
//! replaces the `panic!` body with the real assertions and swaps `#[test]`
//! for whichever runner the enumeration wants.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

/// S-VM-85 / `@contract-shape:unbounded-preservation` `@ac-08` `@tier3`
/// `@real-io` `@property` `@mandatory:mutation_target` — the clone-index
/// link's lifetime CONTAINS the clone's, so a clone that exists always has
/// a link pointing at it, at every interruption point.
///
/// ```gherkin
/// Given a VmDriver whose clone index directory and rootfs master
///   directory are distinct real directories
/// When the start and stop paths are interrupted at each filesystem step
///   in turn -- after the index link, after the clone, after the clone's
///   removal, after the link's removal
/// Then at no interruption point does a rootfs clone exist without an
///   index link pointing at it
/// And every residue left by an interruption is either nothing, or a
///   dangling index link that a subsequent VmHostState observe reports and
///   discard_artifacts removes idempotently
/// And discard_artifacts never derives the clone's path -- it resolves it
///   by reading the link
/// ```
///
/// ## The invariant, and why it is the mutation target
///
/// ADR-0083 §D3f names the contract: **the link is created before the
/// clone, and removed after the clone.** Therefore at every instant a
/// clone that exists has a link that exists — contrapositive *no link ⇒
/// no clone* — so enumerating links enumerates a SUPERSET of live clones
/// and the reclamation sweep cannot miss one. The two crash windows both
/// converge rather than leak (§D3f's table):
///
/// | Crash point | Residue |
/// |---|---|
/// | after link, before clone | dangling link, no clone |
/// | after clone removal, before link removal (in `stop`) | dangling link, no clone |
/// | after clone, before link | **unreachable by construction** |
///
/// This is THE mandatory mutation target. A mutation that swaps either
/// ordering — create-clone-before-link on the start path, or
/// remove-link-before-clone on the stop path — reopens exactly the
/// invisible-orphan leak S-VM-84 closes, and MUST be killed here.
/// S-VM-84 alone cannot catch it, because the offending residue (a clone
/// with no link) is only observable on the interrupted interleavings this
/// scenario drives — never at the quiescent end states S-VM-84 asserts on.
///
/// ## Activation plan (step 03-09)
///
/// * Mirror `vm_driver_stop_totality.rs`'s component-scope fixtures:
///   `build_layout` / `build_spec` / `build_driver` over `SimVmm` and a
///   real `TempDir`. Give the driver a clone-index directory and an
///   operator rootfs-master directory that are DISTINCT real directories
///   (the master beside which `RootfsPlan::for_alloc` reflinks the clone,
///   and the platform-owned `clone_index_dir` that holds the symlink).
/// * Interrupt the start and stop filesystem sequences at each of the four
///   named steps in turn (after the index link, after the clone, after the
///   clone's removal, after the link's removal). `@property`
///   (`unbounded-preservation`) quantifies over interruption points, not
///   over an enumerable delta — the four points ARE the mutation-killing
///   set; the crafter picks property-vs-parametrize per the paradigm-match
///   rule, but every point must be exercised.
/// * At EACH interruption point assert the invariant directly: there is no
///   clone without a link. Any residue is either nothing or a dangling
///   link.
/// * Then drive a `VmHostState` observe + `discard_artifacts` over the
///   residue and assert it converges idempotently: the dangling link is
///   reported by `observe`, `discard_artifacts` removes the (absent)
///   target and the link, both `NotFound`-tolerant, and a second
///   `discard_artifacts` is a no-op.
/// * The LAST clause is a STRUCTURAL assertion, not a behavioural one, and
///   it is what stops the re-derivation defect (§D3f's root cause) from
///   being reintroduced: `discard_artifacts` must resolve the clone's path
///   by `read_link` on the index entry, NEVER by re-deriving it from
///   `parent([vm] rootfs)` + `AllocationId` (which an operator spec-edit
///   or workload deletion can destroy while the clone survives).
///
/// The scaffold panics today because none of that production surface
/// exists at HEAD; it is delivered by step 03-09.
#[test]
#[should_panic(expected = "RED scaffold")]
fn no_clone_index_link_implies_no_clone_at_every_interruption_point() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-85 / step 03-09 -- the clone-index link is \
         created before the per-launch rootfs clone and removed after it, so at no interruption \
         point does a clone exist without a link; any residue is a dangling link that a \
         VmHostState observe reports and discard_artifacts removes idempotently by reading the \
         link, never by re-deriving the clone path)"
    );
}
