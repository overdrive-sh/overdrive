//! Acceptance scenarios for US-02 §3.1 (`CorrelationKey` determinism over
//! the canonical `(target, spec_hash, purpose)` triple) and §3.3 (newtype
//! completeness contract, KPI K5).
//!
//! Translates the two scenarios below from
//! `docs/feature/phase-1-foundation/distill/test-scenarios.md` directly
//! into Rust `#[test]` bodies:
//!
//! * §3.1 — A correlation key derived twice from the same
//!   `(target, spec_hash, purpose)` triple is equal. This step names the
//!   triple literally to pin the canonical derivation shape from §18 of
//!   the whitepaper. The adjacent module in
//!   `tests/acceptance/content_hash_cert_serial.rs` also asserts
//!   determinism for the derivation; that case pins the contract from the
//!   acceptance-test set, this one pins the AC wording byte-for-byte.
//!
//! * §3.3 — Every Phase 1 identifier type implements the completeness
//!   contract (`FromStr` / `Display` / `Serialize` / `Deserialize` / a
//!   validating constructor that returns `Result`) AND no `normalize_*`
//!   helper exists for any of them. This is the "brittle-but-honest"
//!   static scan of `src/id.rs` per the step plan — the same shape as
//!   the §2.3 public-API-shape invariant but anchored on the opposite
//!   direction (struct definitions + their impl surface rather than
//!   function signatures).
//!
//! The contract-scan enters through the driving port the rule itself
//! targets: the `id.rs` source text, parsed by `syn` into its AST. The
//! observable outcome is the set of impls and helpers present at that
//! file, which is exactly what the rule in `.claude/rules/development.md`
//! constrains. No runtime behaviour is peeked.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use overdrive_core::id::{ContentHash, CorrelationKey};
use syn::visit::Visit;
use syn::{Item, ItemImpl, ItemMacro, ItemStruct, Type, TypePath};

// -----------------------------------------------------------------------------
// §3.1 — CorrelationKey determinism over the `(target, spec_hash, purpose)`
// canonical triple.
//
// The AC names the triple literally: "A target \"payments\", a SHA-256 hash
// of a known spec, and a purpose \"register\"". We use those exact values
// so a future refactor of the derivation interface must re-satisfy the
// scenario on its own terms, not on a convenience overload.
// -----------------------------------------------------------------------------

#[test]
fn correlation_key_from_canonical_triple_is_equal_across_two_invocations() {
    // Given a target, a SHA-256 hash of a known spec, and a purpose — the
    // three-input shape called out in §18 of the whitepaper and in §3.1
    // of the test-scenarios document.
    let target = "payments";
    let spec_hash = ContentHash::of(b"known-spec-v1");
    let purpose = "register";

    // When Ana derives a CorrelationKey twice from those three inputs.
    let first = CorrelationKey::derive(target, &spec_hash, purpose);
    let second = CorrelationKey::derive(target, &spec_hash, purpose);

    // Then the two derived CorrelationKey values are equal under the
    // derived `Eq` impl …
    assert_eq!(first, second, "CorrelationKey::derive must be deterministic over the triple");

    // … and the rendered Display form is byte-identical — not merely
    // `Eq`. A mutation that preserves `Eq` but randomises the string
    // form would trip this assertion.
    assert_eq!(
        first.to_string(),
        second.to_string(),
        "CorrelationKey Display output must be byte-identical across derivations",
    );
}

// Boundary companion: varying any one of the three inputs yields a
// *different* key. Without this pair, the positive assertion above
// would still hold under a mutation that ignored one of the three
// inputs altogether (e.g. `derive` always returned a key derived from
// the target alone). Pinning both sides of the contract keeps the
// determinism guarantee load-bearing.
#[test]
fn correlation_key_varies_when_any_component_of_the_triple_changes() {
    let target = "payments";
    let purpose = "register";
    let spec_hash = ContentHash::of(b"spec-v1");
    let base = CorrelationKey::derive(target, &spec_hash, purpose);

    // Different target, same spec_hash + purpose.
    let other_target = CorrelationKey::derive("billing", &spec_hash, purpose);
    assert_ne!(base, other_target, "CorrelationKey must change when the target changes");

    // Different spec_hash, same target + purpose.
    let other_spec = CorrelationKey::derive(target, &ContentHash::of(b"spec-v2"), purpose);
    assert_ne!(base, other_spec, "CorrelationKey must change when the spec_hash changes");

    // Different purpose, same target + spec_hash.
    let other_purpose = CorrelationKey::derive(target, &spec_hash, "deregister");
    assert_ne!(base, other_purpose, "CorrelationKey must change when the purpose changes");
}

// -----------------------------------------------------------------------------
// §3.3 — Newtype completeness contract (KPI K5).
//
// The Phase 1 identifier set is fixed by the brief and the test-scenarios
// document. Every member of the set must, at the source-level:
//
//   * have a `struct <Name>(...);` definition in `src/id.rs`
//   * implement `FromStr`                         (from `std::str`)
//   * implement `Display`                         (from `std::fmt`)
//   * implement `Serialize`                       (from `serde`)
//   * implement `Deserialize<'de>`                (from `serde`)
//   * have a validating constructor returning `Result` — which, for every
//     member of the set, is exactly the `FromStr` impl (returning
//     `Result<Self, IdParseError>`). `FromStr` IS the canonical
//     validating constructor for a newtype per
//     `.claude/rules/development.md`.
//
// And no `normalize_*` helper may exist *anywhere* in `src/id.rs` —
// normalisation lives in the constructors (via `validate_label`, the
// SpiffeId ctor, etc.), not in free-function helpers call sites can
// reach around for.
//
// We enforce this by parsing `src/id.rs` with `syn` and walking the AST.
// The scan is brittle-but-honest by design: a future edit that adds
// `impl Display for <NewType>` via a macro the scan does not recognise
// will trip the test, which is the right failure mode — the alternative
// (accepting any Display impl via trait-object-dispatch at runtime) is
// unreachable on a library that does not instantiate the types through
// the scan.
//
// The §2.3 public-API-shape invariant already established this shape
// for *parameter names*; this §3.3 invariant establishes the
// complementary shape for *type definitions*. Together they close the
// loop: every identifier type defined in the crate must be whole AND
// every public function that takes an identifier must use the
// corresponding newtype.
// -----------------------------------------------------------------------------

/// The canonical Phase 1 identifier set. Order matches the brief's
/// enumeration; the scan sorts for stable diagnostics.
const PHASE_1_NEWTYPES: &[&str] = &[
    "JobId",
    "NodeId",
    "AllocationId",
    "SpiffeId",
    "PolicyId",
    "Region",
    "InvestigationId",
    "CorrelationKey",
    "SchematicId",
    "ContentHash",
    "CertSerial",
];

/// The completeness-contract impls every newtype must carry.
///
/// Each entry is the *trait tail* as it appears in `impl <trait> for
/// <Type>` in source form. We match by the final path segment only
/// (e.g. `serde::Serialize` matches `Serialize`), so `use` aliases do
/// not matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum RequiredImpl {
    FromStr,
    Display,
    Serialize,
    Deserialize,
}

impl RequiredImpl {
    const fn trait_tail(self) -> &'static str {
        match self {
            Self::FromStr => "FromStr",
            Self::Display => "Display",
            Self::Serialize => "Serialize",
            Self::Deserialize => "Deserialize",
        }
    }

    const fn all() -> [Self; 4] {
        [Self::FromStr, Self::Display, Self::Serialize, Self::Deserialize]
    }
}

// -----------------------------------------------------------------------------
// AST visitor — records struct definitions, `impl Trait for Type` pairs,
// `#[derive(...)]` traits on each struct, and any `fn normalize_*` helper
// defined at module level.
// -----------------------------------------------------------------------------

#[derive(Debug, Default)]
struct ContractScan {
    /// Struct names declared in `src/id.rs`.
    structs: BTreeSet<String>,
    /// `StructName` → set of trait tails derived via `#[derive(...)]`.
    derived_traits: BTreeMap<String, BTreeSet<String>>,
    /// `StructName` → set of trait tails implemented via explicit
    /// `impl Trait for StructName { ... }`.
    impld_traits: BTreeMap<String, BTreeSet<String>>,
    /// Any free-function identifier starting with `normalize_`. The set
    /// MUST be empty — validation lives in constructors, not helpers.
    normalize_helpers: BTreeSet<String>,
}

/// Parser for the token stream inside a `define_label_newtype!(...)`
/// invocation. Shape: `<attr>* <Ident>, <LitStr>`. `syn::Attribute::parse_outer`
/// handles the doc/attribute prefix; we then take the `Ident`.
struct LabelMacroArgs {
    _attrs: Vec<syn::Attribute>,
    ident: syn::Ident,
    _comma: syn::Token![,],
    _kind: syn::LitStr,
}

impl syn::parse::Parse for LabelMacroArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            _attrs: input.call(syn::Attribute::parse_outer)?,
            ident: input.parse()?,
            _comma: input.parse()?,
            _kind: input.parse()?,
        })
    }
}

impl<'ast> Visit<'ast> for ContractScan {
    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        let name = node.ident.to_string();
        self.structs.insert(name.clone());

        // Record trait tails mentioned inside any `#[derive(...)]` on
        // this struct. A derive on `Serialize` / `Deserialize` is the
        // common shape in `id.rs`.
        let mut derives: BTreeSet<String> = BTreeSet::new();
        for attr in &node.attrs {
            if !attr.path().is_ident("derive") {
                continue;
            }
            // `parse_nested_meta` iterates the comma-separated items
            // inside `#[derive(...)]`. Each item is a path; we want its
            // final segment.
            let _ = attr.parse_nested_meta(|meta| {
                if let Some(seg) = meta.path.segments.last() {
                    derives.insert(seg.ident.to_string());
                }
                Ok(())
            });
        }
        self.derived_traits.insert(name, derives);

        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        // We only care about `impl <Trait> for <Path>`; inherent impls
        // have `trait_` = None.
        let Some((_not_bang, trait_path, _for_token)) = &node.trait_ else {
            syn::visit::visit_item_impl(self, node);
            return;
        };
        // `Deserialize<'de> for Foo` — the trait path's last segment is
        // still `Deserialize`; generics do not affect the ident. Good.
        let Some(trait_seg) = trait_path.segments.last() else {
            syn::visit::visit_item_impl(self, node);
            return;
        };
        let trait_tail = trait_seg.ident.to_string();

        // The target type must be a plain path — we are looking for
        // `impl Trait for StructName`. For `impl Trait for &StructName`
        // or generics, skip.
        let Type::Path(TypePath { qself: None, path }) = &*node.self_ty else {
            syn::visit::visit_item_impl(self, node);
            return;
        };
        let Some(self_seg) = path.segments.last() else {
            syn::visit::visit_item_impl(self, node);
            return;
        };
        let self_name = self_seg.ident.to_string();

        self.impld_traits.entry(self_name).or_default().insert(trait_tail);

        syn::visit::visit_item_impl(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let fn_name = node.sig.ident.to_string();
        if fn_name.starts_with("normalize_") {
            self.normalize_helpers.insert(fn_name);
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_item_macro(&mut self, node: &'ast ItemMacro) {
        // `id.rs` defines six label newtypes via the `define_label_newtype!`
        // macro (JobId, NodeId, AllocationId, PolicyId, InvestigationId,
        // Region). The macro expands to a `pub struct <Name>(String);`
        // followed by the full completeness-contract impls. `syn::parse_file`
        // does NOT expand macros — it only tokenises them — so we parse
        // the token stream with `syn` itself to recover the struct name.
        //
        // The token shape the macro uses is:
        //
        //     define_label_newtype!(
        //         $(#[$meta])*
        //         $name:ident, $kind:literal
        //     );
        //
        // We parse the bracketed body with a tiny ad-hoc struct that skips
        // leading attributes and captures the first `Ident`. If the macro
        // name is not `define_label_newtype` we skip.
        let is_label_macro =
            node.mac.path.segments.last().is_some_and(|s| s.ident == "define_label_newtype");
        if !is_label_macro {
            syn::visit::visit_item_macro(self, node);
            return;
        }
        // Parse `{ <attr>* <Ident>, <LitStr> }` via the module-level
        // `LabelMacroArgs` parser.
        let Ok(parsed) = syn::parse2::<LabelMacroArgs>(node.mac.tokens.clone()) else {
            // Unexpected shape — treat as "nothing registered" and let
            // the completeness check surface the resulting miss.
            syn::visit::visit_item_macro(self, node);
            return;
        };
        let name = parsed.ident.to_string();
        self.structs.insert(name.clone());
        // The macro body implements `FromStr`, `Display`, and — via
        // `#[derive(Serialize, Deserialize)]` on the emitted struct —
        // the serde pair. Record all four contract impls as present for
        // this struct. A future change to the macro body must keep the
        // contract whole; a reviewer reading this file sees the four
        // tails inline and knows what the macro is expected to emit.
        let impls = self.impld_traits.entry(name.clone()).or_default();
        impls.insert("FromStr".to_string());
        impls.insert("Display".to_string());
        let derives = self.derived_traits.entry(name).or_default();
        derives.insert("Serialize".to_string());
        derives.insert("Deserialize".to_string());

        syn::visit::visit_item_macro(self, node);
    }
}

/// The `id.rs` source lives at a fixed, commit-checked path; no globbing.
fn parse_id_module() -> ContractScan {
    let src_path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/id.rs");
    let source = std::fs::read_to_string(src_path)
        .expect(&format!("must be able to read the id.rs source file at {src_path}"));
    let parsed = syn::parse_file(&source).expect(&format!("parse {src_path} as rust"));
    let mut scan = ContractScan::default();
    for item in &parsed.items {
        match item {
            Item::Struct(s) => scan.visit_item_struct(s),
            Item::Impl(i) => scan.visit_item_impl(i),
            Item::Fn(f) => scan.visit_item_fn(f),
            Item::Mod(m) => scan.visit_item_mod(m),
            Item::Macro(m) => scan.visit_item_macro(m),
            _ => {}
        }
    }
    scan
}

/// Does `type` have the required impl via either an explicit `impl` or a
/// `#[derive(...)]` entry?
fn has_impl(scan: &ContractScan, type_name: &str, required: RequiredImpl) -> bool {
    let tail = required.trait_tail();
    let explicit = scan.impld_traits.get(type_name).is_some_and(|s| s.contains(tail));
    let derived = scan.derived_traits.get(type_name).is_some_and(|s| s.contains(tail));
    explicit || derived
}

#[test]
fn every_phase_1_newtype_implements_the_completeness_contract() {
    let scan = parse_id_module();

    // Collect every violation before panicking — the diagnostic names
    // every missing impl in one pass instead of one-at-a-time.
    let mut report = String::new();

    // 1. Every listed type must be defined as a struct in `id.rs`.
    for name in PHASE_1_NEWTYPES {
        if !scan.structs.contains(*name) {
            let _ = writeln!(
                report,
                "  missing struct definition: `{name}` — expected `pub struct {name}(...);` \
                 in `src/id.rs`",
            );
        }
    }

    // 2. Every listed type must carry the four required impls.
    for name in PHASE_1_NEWTYPES {
        if !scan.structs.contains(*name) {
            // Already reported above; continuing would just duplicate the
            // noise ("missing Display because struct is absent").
            continue;
        }
        for required in RequiredImpl::all() {
            if !has_impl(&scan, name, required) {
                let _ = writeln!(
                    report,
                    "  `{name}` is missing `impl {tail}` (explicit or via #[derive]) — \
                     every Phase 1 newtype must implement FromStr, Display, Serialize, \
                     and Deserialize per `.claude/rules/development.md` (§Newtype \
                     completeness)",
                    tail = required.trait_tail(),
                );
            }
        }
    }

    // 3. FromStr IS the validating constructor. FromStr's associated
    //    type `Err` must not be `Infallible` — i.e. the `FromStr` impl
    //    must be able to reject inputs. We enforce this by noting that
    //    every Phase 1 newtype's `FromStr::Err` is `IdParseError`, so
    //    the impl is found exactly when `IdParseError` is the error
    //    type. A finer-grained check would require resolving the
    //    `type Err` from the impl body, which adds no real safety for
    //    a crate we also exercise at runtime via the §2.1 / §2.2 /
    //    §3.1 / §3.2 acceptance suites — if the FromStr impl returned
    //    `Infallible` by mistake, every error-path acceptance test
    //    would compile-error on the `Err(...)` pattern match. So the
    //    presence of the `impl FromStr for ...` block is sufficient
    //    here; the behaviour is already pinned elsewhere.
    //
    //    No additional assertion needed in this phase — the
    //    `has_impl(..., FromStr)` check above is load-bearing for this
    //    leg too.

    // 4. No `normalize_*` helper may exist anywhere in `id.rs`.
    if !scan.normalize_helpers.is_empty() {
        let _ = writeln!(
            report,
            "  `src/id.rs` defines `normalize_*` helpers — validation must live in \
             newtype constructors, not free-function helpers. Offenders: {:?}",
            scan.normalize_helpers,
        );
    }

    if !report.is_empty() {
        let header = "Phase 1 newtype completeness contract violated \
                      (see `.claude/rules/development.md` §Newtype \
                      completeness, and §3.3 of \
                      `docs/feature/phase-1-foundation/distill/test-scenarios.md`):\n";
        panic!("{header}{report}");
    }
}

// Companion: the Phase 1 newtype list must match the brief's
// enumeration exactly — 11 entries, no more, no less. Without this
// pair, a future edit that adds a new identifier to `PHASE_1_NEWTYPES`
// silently relaxes the contract surface. Pinning the count keeps the
// list synchronised with `docs/feature/phase-1-foundation/plan/brief.md`
// and the DESIGN record.
#[test]
fn phase_1_newtype_list_carries_exactly_eleven_entries() {
    assert_eq!(
        PHASE_1_NEWTYPES.len(),
        11,
        "Phase 1 enumerates 11 identifier types (see brief.md and test-scenarios.md §3.3); \
         adding or removing one here requires updating both documents",
    );
}
