//! The placement declarations of a Metal integration test:
//! `#[overdrive_test(host_pure)]`, `#[overdrive_test(guest = "<group>")]`,
//! `placement!(host_pure)` and `overdrive_test_default!(host_pure)`.
//!
//! The macros are re-exported as `overdrive_testing::{overdrive_test,
//! placement, overdrive_test_default}`; consumers depend on `overdrive-testing`
//! (or, where nothing of the guest kit is needed, on this crate alone).
//!
//! # Expansion
//!
//! `#[overdrive_test]` never inserts `#[test]`: the test function keeps its
//! signature and its own test attribute (`#[test]`, `#[tokio::test]`, ...), and
//! a function without one is not registered as a test.
//!
//! `host_pure` leaves the function untouched, so it applies to any test
//! function, including one that returns a `Result`, and references no runtime
//! crate.
//!
//! `guest = "<group>"` wraps the body so that:
//!
//! - where `overdrive_testing::placement::inside_guest()` holds, or
//!   `overdrive_testing::placement::in_metal_run()` does not, the original body
//!   runs in place (off a Metal run the machine is private to the process);
//! - on a Metal run, outside a guest, the host-side half hands the running test
//!   binary to `overdrive_testing::placement::run_in_guest(&placement, exe,
//!   test_name)` with a typed `GuestPlacement` on a plain OS thread (the guest
//!   kit drives its own runtime, which must not start inside an async test's
//!   runtime) and panics with the typed error when the in-guest run fails. The
//!   body never runs on the host.
//!
//! Where `#[quint_test]` or `#[quint_run]` is present, `#[overdrive_test]` goes
//! below it.
//!
//! # Placement record
//!
//! Every declaration emits one record into the `overdrive_test_placement` ELF
//! section of the test binary, as one named item (so a duplicate is rustc's
//! E0428): a sequence of NUL-terminated fields `<kind>`, `<test>`, `<argc>`,
//! then `argc` arguments. `<test>` is the function's `module_path!()` plus
//! `::<fn name>`. The kinds this crate writes are `host_pure`, `guest`
//! (arguments: group, profile, `"0"` for an explicit guest form),
//! `module_host_pure` (`placement!`; `<test>` is the declaring module's path)
//! and `default_host_pure` (`overdrive_test_default!`; `<test>` is empty).
//! `infra/metal/test-wrapper.sh` reads the section to place a test that has no
//! table entry, so a test is placed only by an explicit declaration.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{Expr, ExprLit, Ident, ItemFn, Lit, LitStr, Meta, ReturnType, parse_macro_input};

/// The ELF section the placement records are written to.
const SECTION: &str = "overdrive_test_placement";

/// Declares where a Metal integration test runs.
///
/// Implemented for `#[overdrive_test(host_pure)]` and
/// `#[overdrive_test(guest = "<group>")]`; the other forms of the approved
/// grammar are accepted and leave the test untouched until they are built.
#[proc_macro_attribute]
pub fn overdrive_test(attribute: TokenStream, item: TokenStream) -> TokenStream {
    // SCAFFOLD: true. The approved grammar's other forms (`native_launcher`,
    // `lane_operator`, `lane_owner`, the argument-free module form and
    // `guest = .., profile = ..`) return the item unchanged until their
    // expansion is built, so tests that name them compile and fail for the
    // missing record or hand-off, never for a grammar error.
    let attribute: TokenStream2 = attribute.into();
    let Some(declaration) = Declaration::parse(&attribute) else { return item };
    let function = parse_macro_input!(item as ItemFn);
    match declaration.expand(function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// `placement!(host_pure)`: the module-scoped host-pure declaration. It places
/// the tests whose module path equals the declaring module's, and no test of a
/// child module.
#[proc_macro]
pub fn placement(declaration: TokenStream) -> TokenStream {
    declare_host_pure(
        &declaration.into(),
        "__OVERDRIVE_PLACEMENT_MODULE_HOST_PURE",
        "module_host_pure",
        &quote!(::core::module_path!()),
    )
}

/// `overdrive_test_default!(host_pure)`: the per-binary host-pure default. It
/// places every test of the binary that no attribute or module declaration
/// places.
#[proc_macro]
pub fn overdrive_test_default(declaration: TokenStream) -> TokenStream {
    declare_host_pure(
        &declaration.into(),
        "__OVERDRIVE_PLACEMENT_DEFAULT_HOST_PURE",
        "default_host_pure",
        &quote!(""),
    )
}

/// The one record of a function-like host-pure declaration, as a named item.
fn declare_host_pure(
    declaration: &TokenStream2,
    item: &str,
    kind: &str,
    test: &TokenStream2,
) -> TokenStream {
    let argument: Result<Ident, _> = syn::parse2(declaration.clone());
    match argument {
        Ok(ident) if ident == "host_pure" => {
            let item = Ident::new(item, ident.span());
            record_item(&item, &quote!(#kind), test, &[]).into()
        }
        _ => syn::Error::new(
            declaration.span(),
            "expected `host_pure`, the only placement a declaration takes",
        )
        .to_compile_error()
        .into(),
    }
}

/// The forms of `#[overdrive_test(...)]` that are built.
enum Declaration {
    HostPure,
    Guest(LitStr),
}

impl Declaration {
    fn parse(attribute: &TokenStream2) -> Option<Self> {
        let meta: Meta = syn::parse2(attribute.clone()).ok()?;
        match meta {
            Meta::Path(path) if path.is_ident("host_pure") => Some(Self::HostPure),
            Meta::NameValue(pair) if pair.path.is_ident("guest") => match pair.value {
                Expr::Lit(ExprLit { lit: Lit::Str(group), .. }) => Some(Self::Guest(group)),
                _ => None,
            },
            _ => None,
        }
    }

    fn expand(&self, mut function: ItemFn) -> syn::Result<TokenStream2> {
        let name = function.sig.ident.unraw();
        let item = record_name(&name);
        let test = test_name(&name);
        match self {
            Self::HostPure => {
                let record = record_item(&item, &quote!("host_pure"), &test, &[]);
                Ok(quote!(#function #record))
            }
            Self::Guest(group) => {
                if !matches!(function.sig.output, ReturnType::Default) {
                    return Err(syn::Error::new(
                        function.sig.output.span(),
                        "#[overdrive_test] supports tests that return `()`",
                    ));
                }
                let record = record_item(
                    &item,
                    &quote!("guest"),
                    &test,
                    &[quote!(#group), quote!(""), quote!("0")],
                );
                let known_group = known_group_check(group);
                let gate = gate(group, &name.to_string());
                let original = &function.block;
                function.block = syn::parse_quote!({
                    #gate
                    #original
                });
                Ok(quote!(#function #record #known_group))
            }
        }
    }
}

fn record_name(function: &Ident) -> Ident {
    format_ident!("__OVERDRIVE_PLACEMENT_RECORD_{}", function)
}

/// The `<test>` field of a per-test record: the function's module path and name.
fn test_name(function: &Ident) -> TokenStream2 {
    let name = function.to_string();
    quote!(::core::concat!(::core::module_path!(), "::", #name))
}

/// One record as one named const item: the fields, then a `#[used]` static in
/// the placement section holding them NUL-terminated. `arguments` are
/// `&'static str` expressions; `<argc>` is their count.
fn record_item(
    item: &Ident,
    kind: &TokenStream2,
    test: &TokenStream2,
    arguments: &[TokenStream2],
) -> TokenStream2 {
    let argc = arguments.len().to_string();
    let count = 3 + arguments.len();
    quote! {
        #[doc(hidden)]
        #[allow(non_upper_case_globals, dead_code)]
        const #item: () = {
            const FIELDS: [&str; #count] = [#kind, #test, #argc #(, #arguments)*];
            const LENGTH: usize = {
                let mut length = 0;
                let mut field = 0;
                while field < FIELDS.len() {
                    length += FIELDS[field].len() + 1;
                    field += 1;
                }
                length
            };
            const fn encode() -> [u8; LENGTH] {
                let mut record = [0_u8; LENGTH];
                let mut offset = 0;
                let mut field = 0;
                while field < FIELDS.len() {
                    let bytes = FIELDS[field].as_bytes();
                    let mut index = 0;
                    while index < bytes.len() {
                        record[offset] = bytes[index];
                        offset += 1;
                        index += 1;
                    }
                    // The field's terminating NUL is the zero already there.
                    offset += 1;
                    field += 1;
                }
                record
            }
            #[used]
            #[unsafe(link_section = #SECTION)]
            static RECORD: [u8; LENGTH] = encode();
        };
    }
}

/// The build-time check that the group is one of the registry's. A sibling of
/// the test function, so it is evaluated whatever test attribute decides the
/// function's fate.
fn known_group_check(group: &LitStr) -> TokenStream2 {
    let message = LitStr::new(
        &format!("unknown guest group `{}`", group.value().replace('{', "{{").replace('}', "}}")),
        group.span(),
    );
    // Spanned on the user's literal so rustc's E0080 points at the offending
    // argument, with the value named in the message.
    quote_spanned! {group.span()=>
        const _: () = {
            if !::overdrive_testing::placement::groups::is_known_group(#group) {
                ::core::panic!(#message);
            }
        };
    }
}

/// The statements prepended to a guest-placed test body: the guest hand-off for
/// the host-side half.
fn gate(group: &LitStr, name: &str) -> TokenStream2 {
    quote! {
        const __OVERDRIVE_TEST_FULL_NAME: &str = ::core::concat!(::core::module_path!(), "::", #name);
        if !::overdrive_testing::placement::inside_guest()
            && ::overdrive_testing::placement::in_metal_run()
        {
            let placement = ::overdrive_testing::placement::GuestPlacement::declare(
                #group,
                "",
                ::overdrive_testing::placement::Sharing::Isolated,
                "",
            );
            let test_name = match __OVERDRIVE_TEST_FULL_NAME.split_once("::") {
                ::core::option::Option::Some((_crate_name, rest)) => rest,
                ::core::option::Option::None => __OVERDRIVE_TEST_FULL_NAME,
            };
            let exe = ::std::env::current_exe()
                .unwrap_or_else(|error| ::core::panic!("reading the test binary path: {error}"));
            let outcome = ::std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        ::overdrive_testing::placement::run_in_guest(&placement, &exe, test_name)
                    })
                    .join()
            });
            match outcome {
                ::core::result::Result::Ok(::core::result::Result::Ok(())) => {}
                ::core::result::Result::Ok(::core::result::Result::Err(error)) => {
                    ::core::panic!("{error}")
                }
                ::core::result::Result::Err(panic) => ::std::panic::resume_unwind(panic),
            }
            return;
        }
    }
}
