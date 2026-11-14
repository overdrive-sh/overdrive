# Findings: const-evaluation diagnostic for an unknown guest group or profile (V-11)

Probe: `spike-scratch/metal-test-placement-const-diag/increment-a/` (own `Cargo.toml`, macro crate `macros/`, one test target per case, each selected by its own feature). Raw captures (full Metal run logs): `…/increment-a/evidence/<case>.txt`. Run with `cargo xtask metal run -- cargo check --manifest-path … --features <case> --test <case>` (ordinary grammar, host `x86_64` Metal box, rustc 1.95.0 from `rust-toolchain.toml`). The compiler output below is verbatim except the remote checkout prefix, shown as `<root>`. A control target (`ok_control`, valid groups through all four forms) compiles clean (exit 0).

## Verdicts

| Question | Verdict |
|---|---|
| Q1. `panic!("…{}", runtime_str)` in a `const fn` in const context | **Does not compile**: E0015 `cannot call non-const formatting macro in constant functions`. Confirmed. |
| Q2. Literal-only const-eval panic from a const item the macro emits by re-quoting the user's literal: error span at the literal, message shown? | **Message shown: yes. Span at the literal: NO by default.** The span is the whole `GuestPlacement::declare(...)` call expression (hand-written and `macro_rules!`) or the whole attribute / macro invocation (proc macros, `quote!` call-site spans). The offending value is NOT shown. The span lands on the literal only when the proc macro builds the tokens with `quote_spanned!(literal.span()=> …)` (confirmed below). |
| Q3. End to end through `#[overdrive_test(guest = …)]` and `shared_guest!(guest = …)`; can the proc macro embed the literal in the message? | **Yes.** The macro sees the literal and emits `panic!(concat!("unknown guest group `", <lit>, "`; known groups: ", known_guest_groups!()))`. The message names the value, and the known set via a registry-exported `macro_rules!` that expands to a string literal (nested `concat!` expands eagerly). The two forms are identical in outcome. |

## Recommended pinned diagnostic

Replace the "fixed message per failure kind, value shown by the span" floor with the embedded form. The contract the probe supports:

- The macro (attribute guest form and `shared_guest!`) emits, spanned on the user's `guest` literal (`quote_spanned!`), a const that calls a registry `const fn is_known_group(&str) -> bool` and panics with a literal built by `concat!`:
  `unknown guest group `<value>`; known groups: <space-separated list>`
- The known-set text comes from the single registry list through an exported callback `macro_rules!` (`known_guest_groups!()`, derived from the same invocation that defines the `GROUPS` const), so there is still one list and the proc-macro crate holds none.
- The error is `error[E0080]: evaluation panicked: …` with the primary span on the offending literal.
- `GuestPlacement::declare` keeps its fixed-text panics (`unknown guest group; …`, `unknown guest profile for this group; …`) as the floor for any caller that reaches it without the macro (they show an inner `note: inside GuestPlacement::declare` frame, pointing at the registry source line). The macro path never reaches them for a group; for a profile the same embed form applies (the macro quotes the profile literal and can name it; the per-group known-profile set needs the same callback-macro pattern per group; not probed here, only the group case was).
- Residual cost: the message ends with a trailing space from `concat!` (cosmetic; a separator form can avoid it), and the diagnostic is `E0080` with no `help:` line (rustc gives const-eval panics no custom help).

## Evidence

### Q1: formatting a runtime `&str` in a const-fn panic (`q1_fmt_panic`)

```rust
const fn declare(group: &str) -> usize {
    if group.len() > 100 { panic!("unknown guest group `{}`", group); }
    group.len()
}
const X: usize = declare("nonsense");
```
```
error[E0015]: cannot call non-const formatting macro in constant functions
 --> tests/q1_fmt_panic.rs:4:9
  |
4 |         panic!("unknown guest group `{}`", group);
  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  |
  = note: calls in constant functions are limited to constant functions, tuple structs and tuple variants

For more information about this error, try `rustc --explain E0015`.
error: could not compile `const-diag-probe` (test "q1_fmt_panic") due to 1 previous error
```

### Q2a: hand-written const item, literal-only panic (`q2_literal_const`)

`const _: GuestPlacement = GuestPlacement::declare("no-such-group", "base");`
```
error[E0080]: evaluation panicked: unknown guest group; the known groups are listed in overdrive_testing::placement::groups
  --> tests/q2_literal_const.rs:3:27
   |
 3 | const _: GuestPlacement = GuestPlacement::declare("no-such-group", "base");
   |                           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `_` failed inside this call
   |
note: inside `GuestPlacement::declare`
  --> /home/ubuntu/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/panic.rs:62:9
   |
62 |         $crate::panicking::panic_fmt($crate::const_format_args!($($t)+));
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the failure occurred here
   |
  ::: src/lib.rs:67:13
   |
67 |             panic!("unknown guest group; the known groups are listed in overdrive_testing::placement::groups");
   |             -------------------------------------------------------------------------------------------------- in this macro invocation

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q2_literal_const") due to 1 previous error
```

### Q2b: `macro_rules!` re-quoting the user's literal (`q2_requoted_macro_rules`)

```
error[E0080]: evaluation panicked: unknown guest group; the known groups are listed in overdrive_testing::placement::groups
  --> tests/q2_requoted_macro_rules.rs:4:57
   |
 4 |     (guest = $g:literal) => { const _: GuestPlacement = GuestPlacement::declare($g, "base"); };
   |                                                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `_` failed inside this call
 5 | }
 6 | shared_guest!(guest = "no-such-group");
   | -------------------------------------- in this macro invocation
   |
note: inside `GuestPlacement::declare`
  --> /home/ubuntu/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/panic.rs:62:9
   |
62 |         $crate::panicking::panic_fmt($crate::const_format_args!($($t)+));
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the failure occurred here
   |
  ::: src/lib.rs:67:13
   |
67 |             panic!("unknown guest group; the known groups are listed in overdrive_testing::placement::groups");
   |             -------------------------------------------------------------------------------------------------- in this macro invocation

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q2_requoted_macro_rules") due to 1 previous error
```

### Q3: attribute and function-like, fixed message, default (call-site) spans

`#[overdrive_test(guest = "no-such-group")]` (`q3_attr_fixed`):
```
error[E0080]: evaluation panicked: unknown guest group; the known groups are listed in overdrive_testing::placement::groups
  --> tests/q3_attr_fixed.rs:2:1
   |
 2 | #[overdrive_test(guest = "no-such-group")]
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `it_runs::_` failed inside this call
   |
note: inside `GuestPlacement::declare`
  --> /home/ubuntu/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/panic.rs:62:9
   |
62 |         $crate::panicking::panic_fmt($crate::const_format_args!($($t)+));
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the failure occurred here
   |
  ::: src/lib.rs:67:13
   |
67 |             panic!("unknown guest group; the known groups are listed in overdrive_testing::placement::groups");
   |             -------------------------------------------------------------------------------------------------- in this macro invocation

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_attr_fixed") due to 1 previous error
```
`shared_guest!(guest = "no-such-group")` (`q3_shared_fixed`):
```
error[E0080]: evaluation panicked: unknown guest group; the known groups are listed in overdrive_testing::placement::groups
  --> tests/q3_shared_fixed.rs:2:1
   |
 2 | shared_guest!(guest = "no-such-group");
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `_` failed inside this call
   |
note: inside `GuestPlacement::declare`
  --> /home/ubuntu/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/panic.rs:62:9
   |
62 |         $crate::panicking::panic_fmt($crate::const_format_args!($($t)+));
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the failure occurred here
   |
  ::: src/lib.rs:67:13
   |
67 |             panic!("unknown guest group; the known groups are listed in overdrive_testing::placement::groups");
   |             -------------------------------------------------------------------------------------------------- in this macro invocation

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_shared_fixed") due to 1 previous error
```
Profile, `#[overdrive_test(guest = "core", profile = "no-such-profile")]` (`q3_attr_profile_fixed`):
```
error[E0080]: evaluation panicked: unknown guest profile for this group; the known profiles are listed in overdrive_testing::placement::groups
  --> tests/q3_attr_profile_fixed.rs:2:1
   |
 2 | #[overdrive_test(guest = "core", profile = "no-such-profile")]
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `it_runs::_` failed inside this call
   |
note: inside `GuestPlacement::declare`
  --> /home/ubuntu/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/panic.rs:62:9
   |
62 |         $crate::panicking::panic_fmt($crate::const_format_args!($($t)+));
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the failure occurred here
   |
  ::: src/lib.rs:70:13
   |
70 |             panic!("unknown guest profile for this group; the known profiles are listed in overdrive_testing::placement::groups");
   |             --------------------------------------------------------------------------------------------------------------------- in this macro invocation

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_attr_profile_fixed") due to 1 previous error
```

### Q3: fixed message, tokens spanned on the literal (`quote_spanned!`)

`q3_attr_fixed_spanned`:
```
error[E0080]: evaluation panicked: unknown guest group; the known groups are listed in overdrive_testing::placement::groups
  --> tests/q3_attr_fixed_spanned.rs:2:40
   |
 2 | #[overdrive_test_fixed_spanned(guest = "no-such-group")]
   |                                        ^^^^^^^^^^^^^^^ evaluation of `it_runs::_` failed inside this call
   |
note: inside `GuestPlacement::declare`
  --> /home/ubuntu/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/panic.rs:62:9
   |
62 |         $crate::panicking::panic_fmt($crate::const_format_args!($($t)+));
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the failure occurred here
   |
  ::: src/lib.rs:67:13
   |
67 |             panic!("unknown guest group; the known groups are listed in overdrive_testing::placement::groups");
   |             -------------------------------------------------------------------------------------------------- in this macro invocation

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_attr_fixed_spanned") due to 1 previous error
```
`q3_shared_fixed_spanned`:
```
error[E0080]: evaluation panicked: unknown guest group; the known groups are listed in overdrive_testing::placement::groups
  --> tests/q3_shared_fixed_spanned.rs:2:37
   |
 2 | shared_guest_fixed_spanned!(guest = "no-such-group");
   |                                     ^^^^^^^^^^^^^^^ evaluation of `_` failed inside this call
   |
note: inside `GuestPlacement::declare`
  --> /home/ubuntu/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/panic.rs:62:9
   |
62 |         $crate::panicking::panic_fmt($crate::const_format_args!($($t)+));
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the failure occurred here
   |
  ::: src/lib.rs:67:13
   |
67 |             panic!("unknown guest group; the known groups are listed in overdrive_testing::placement::groups");
   |             -------------------------------------------------------------------------------------------------- in this macro invocation

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_shared_fixed_spanned") due to 1 previous error
```

### Q3: literal embedded in the message by the proc macro (call-site span)

`q3_attr_embed`:
```
error[E0080]: evaluation panicked: unknown guest group `no-such-group`
 --> tests/q3_attr_embed.rs:2:1
  |
2 | #[overdrive_test_embed(guest = "no-such-group")]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `it_runs::_` failed here

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_attr_embed") due to 1 previous error
```
`q3_shared_embed`:
```
error[E0080]: evaluation panicked: unknown guest group `no-such-group`
 --> tests/q3_shared_embed.rs:2:1
  |
2 | shared_guest_embed!(guest = "no-such-group");
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `_` failed here

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_shared_embed") due to 1 previous error
```

### Q3: embedded literal and known set, call-site span

`q3_attr_embed_known`:
```
error[E0080]: evaluation panicked: unknown guest group `no-such-group`; known groups: probe core unload ulp mtls-relay cgroup-default stress nested-boundary 
 --> tests/q3_attr_embed_known.rs:2:1
  |
2 | #[overdrive_test_embed_known(guest = "no-such-group")]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `it_runs::_` failed here

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_attr_embed_known") due to 1 previous error
```
`q3_shared_embed_known`:
```
error[E0080]: evaluation panicked: unknown guest group `no-such-group`; known groups: probe core unload ulp mtls-relay cgroup-default stress nested-boundary 
 --> tests/q3_shared_embed_known.rs:2:1
  |
2 | shared_guest_embed_known!(guest = "no-such-group");
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ evaluation of `_` failed here

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_shared_embed_known") due to 1 previous error
```

### Q3: embedded literal and known set, spanned on the literal (recommended form)

`q3_attr_spanned`:
```
error[E0080]: evaluation panicked: unknown guest group `no-such-group`; known groups: probe core unload ulp mtls-relay cgroup-default stress nested-boundary 
 --> tests/q3_attr_spanned.rs:2:34
  |
2 | #[overdrive_test_spanned(guest = "no-such-group")]
  |                                  ^^^^^^^^^^^^^^^ evaluation of `it_runs::_` failed here

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_attr_spanned") due to 1 previous error
```
`q3_shared_spanned`:
```
error[E0080]: evaluation panicked: unknown guest group `no-such-group`; known groups: probe core unload ulp mtls-relay cgroup-default stress nested-boundary 
 --> tests/q3_shared_spanned.rs:2:31
  |
2 | shared_guest_spanned!(guest = "no-such-group");
  |                               ^^^^^^^^^^^^^^^ evaluation of `_` failed here

For more information about this error, try `rustc --explain E0080`.
error: could not compile `const-diag-probe` (test "q3_shared_spanned") due to 1 previous error
```

## Implications for the design

- The feature-delta's assumption that the error's span shows the offending value (because the macro quotes the user's literal) is **false as written**: the default span covers the whole call or invocation. Two things make it true, and both are needed for the best result: spanning the emitted tokens on the literal, and embedding the literal in the message.
- Formatting a runtime value in the const panic (Q1) is not possible, so the registry's `const fn declare` cannot name the value itself; naming it is the macro's job, which works because the macro sees the literal.
- The single-list property holds: the group list lives only in the registry, exposed to the macro as emitted paths (`::overdrive_testing::…`), so the proc-macro crate needs no dependency on it.
- Not probed: per-group known-profile text, and a `group`/`profile` pair given through a non-literal expression (the macro sees only literals by construction; a non-literal falls to the parse error).
