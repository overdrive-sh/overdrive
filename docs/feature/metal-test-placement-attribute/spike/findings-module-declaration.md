# Probe findings — a per-module placement declaration on stable Rust (V-1)

**Verdict (a): DOESN'T-WORK.** An attribute proc macro cannot be applied to a
file module (`#[attr] mod x;`), to an inline module that contains a file module
(`#[attr] mod outer { mod y; }`), or as an inner `#![attr]` at the top of a
file module, on the pinned stable toolchain.

**Verdict (b): WORKS.** A function-like macro invoked once in a module, plus a
no-argument `#[overdrive_test]` that names the module's hidden const by plain
path, gives a per-module declaration on stable. The placement record reaches
the test binary and the Metal wrapper places the test from it.

Gate recommendation: **PROMOTE** the function-like-macro-plus-const form as the
module declaration; **DISCARD** any design that needs an attribute on a module.

Probe sources and raw evidence:
`spike-scratch/metal-test-placement-module-decl/increment-a/` (`evidence/*.txt`),
commit `de8759343`.

## Substrate

- `rust-toolchain.toml` pins `channel = "1.95.0"`. The probe's `build.rs` ran
  `$RUSTC -V` on the Metal box: `rustc 1.95.0 (59807616e 2026-04-14)` (stable,
  no nightly features).
- Probe crate: edition 2024, a self-contained workspace with a proc-macro crate
  (`macros/`, syn 2) and a test crate. `autotests = false`; the repo layout is
  mirrored by `tests/integration.rs` with `mod integration { mod alpha; … }`.
- All builds and runs through ordinary `cargo xtask metal run --` (no
  `--exclusive`, no infra edits). The probe carries the same nextest wrapper
  block as the shared-guest probe in its own `.config/nextest.toml`. `rustc -V`
  is not in the ordinary grammar, hence the `build.rs`. The `readelf`
  inspection was a plain `ssh` read of the built binary on the box
  (`evidence/b4-readelf.txt`).
- The probe macro has no guest hand-off, so test bodies ran on the host side of
  the guest-group placement. V-1 is about the declaration and the record.

## (a) Attribute on a module — predicted vs actual

Predicted: rejected with E0658. Actual: confirmed.

`#[probe_macros::module_placement(guest = "probe")] mod m;`
(`evidence/a1-attr-file-mod.txt`):

```text
error[E0658]: file modules in proc macro input are unstable
 --> tests/neg_attr_file_mod.rs:3:5
  |
3 |     mod m;
  |     ^^^^^^
  = note: see issue #54727 <https://github.com/rust-lang/rust/issues/54727> for more information
```

`#[attr] mod outer { mod y; }` (`evidence/a2-attr-nested-file-mod.txt`): the
same error, pointing at the nested `mod y;`.

Inner `#![attr]` at the top of a file module
(`evidence/a3-inner-attr-file-mod.txt`):

```text
error[E0658]: inner macro attributes are unstable
 --> tests/neg_inner_attr_file_mod/m.rs:1:4
  = note: see issue #54726 <https://github.com/rust-lang/rust/issues/54726> for more information

error[E0658]: file modules in proc macro input are unstable
 --> tests/neg_inner_attr_file_mod.rs:2:5
  = note: see issue #54727 ...
```

Edge: an attribute on an inline module with no file-module children compiles,
but it cannot reach a file module, so it does not serve the repo layout.

## (b) Function-like macro + hidden const + argument-free attribute

`shared_guest!(guest = "cgroup-default")` expands to a private
`#[doc(hidden)] const __OVERDRIVE_MODULE_PLACEMENT: &str = "cgroup-default";`.
`#[overdrive_test]` (no arguments) prepends a plain-path reference to that
const, builds the record, and adds `#[test]` when no test attribute is present.

### (b1) Compiles; the body sees the module's group at run time — held

`cargo check --all-targets` exit 0 (`evidence/b-check.txt`). Run
(`evidence/b1-run.txt`, run `92c59af8f7a59af37ab4c56c`): 8 tests, 8 passed.

```text
PLACEMENT_PROBE test=integration::integration::alpha::ours_then_tokio group=cgroup-default
PLACEMENT_PROBE test=integration::integration::beta::resolves_its_own_group group=probe
PLACEMENT_PROBE test=integration::integration::gamma::outer_test group=core
```

### (b2) No declaration in the module — fails to compile

`evidence/b2-nodecl.txt`:

```text
error[E0425]: cannot find value `__OVERDRIVE_MODULE_PLACEMENT` in this scope
 --> tests/neg_nodecl/m.rs:3:1
  |
3 | #[overdrive_test]
  | ^^^^^^^^^^^^^^^^^ not found in this scope
```

A `compile_error!` is not achievable: a proc macro cannot see the enclosing
module's items, so absence surfaces only as name resolution. The const's name is
the message. With a descriptive name (`evidence/b2-nodecl-descriptive.txt`):

```text
error[E0425]: cannot find value `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE` in this scope
```

(needs `#[allow(non_upper_case_globals)]` on the const). A trait or
`diagnostic::on_unimplemented` cannot improve this: name resolution fails
before any trait bound is evaluated.

Duplicate declaration (`evidence/b3-dup.txt`): a clear E0428 showing both
sites. An unknown group is rejected by `shared_guest!` itself; that path was
not compiled as a separate case.

### (b3) Two modules, different declarations — held

`alpha` = `cgroup-default`, `beta` = `probe`, `gamma` = `core`: each resolved
its own. A nested module does not inherit the parent's const (E0425,
`evidence/b3-nested-nodecl.txt`) unless it writes `use super::*;`.

### (b4) Record in the binary — held; `#[used]` alone survives

Each test emits a `#[used] #[unsafe(link_section = "overdrive_test_placement")]`
static of `<module path>\0<group>\0`, const-evaluated from the module const,
`module_path!()` and the fn name (`evidence/b4-readelf.txt`, over ssh):

```text
  [22] overdrive_test_placement PROGBITS 0000000000036be4 036be4 0001f6 00   A  0   0  1
  0x00036bf4 65677261 74696f6e 3a3a6265 74613a3a egration::beta::
  0x00036c04 7265736f 6c766573 5f697473 5f6f776e resolves_its_own
  0x00036c14 5f67726f 75700070 726f6265 00696e74 _group.probe.int
```

Eight records, one per test. With the body reference (`black_box(&RECORD)`)
removed, the section and all eight records are still present in dev and
release builds (`evidence/b4-noblackbox*.txt`), so the walking skeleton's
concern did not reproduce on rustc 1.95.0 / x86_64 Linux.

End to end, with the production wrapper unmodified: the tests ran with no table
entry; the control `inline_only::t`, a plain `#[test]` with no record, was
refused `OVERDRIVE_METAL_TEST_ERROR error=test_unclassified`, exit 78
(`evidence/c-control-unplaced.txt`).

### (b5) Stacking with other test attributes

| Form | Result |
|---|---|
| `#[overdrive_test] fn` | passes; macro adds `#[test]` |
| `#[overdrive_test] #[test] fn` | passes, runs once |
| `#[overdrive_test] #[tokio::test] async fn` | passes |
| `#[tokio::test] #[overdrive_test] async fn` | passes |
| `#[test] #[overdrive_test] fn` | runs twice |

`#[test]` above `#[overdrive_test]` is expanded first, so `#[overdrive_test]`
no longer sees a test attribute and adds a second `#[test]` (nextest reported
`running 2 tests` for `alpha::test_then_ours`). `crates/overdrive-testing-macros`
has the same "add `#[test]` when absent" logic, so it has this order
sensitivity today.

### (c) File module in the repo's entrypoint layout — held

`tests/integration.rs` with `mod integration { mod alpha; mod beta; mod gamma; }`
and `shared_guest!(…)` as the first item of each `tests/integration/*.rs`
works. The recorded module path is `integration::integration::<module>::<fn>`.

## Design implications

1. The per-module declaration is a function-like macro call, once per module,
   as an item. An attribute on a module is not available on stable.
2. The test attribute reads the module's const by plain path. A missing
   declaration is an E0425 compile error whose text is the const's name; DESIGN
   picks a name that reads as an instruction and decides whether it is pinned.
3. Declarations bind one module; nested modules need their own (or
   `use super::*`); a duplicate is a clear compile error.
4. The record needs no crate-level registry. `#[used]` alone survived here; a
   body reference is optional insurance.
5. "Add `#[test]` when absent" is order-sensitive with a `#[test]` written
   above it; DESIGN may require an explicit test attribute or reject the
   duplicate.

## Not run

- No guest hand-off in the probe macro.
- The unknown-group error path of `shared_guest!` as a separate compiled case.
