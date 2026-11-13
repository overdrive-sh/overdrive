//! The `#[overdrive_test(guest = "<group>")]` attribute: declare on the test
//! itself that its body runs inside a fresh guest of the named group.
//!
//! The attribute is re-exported as `overdrive_testing::overdrive_test`;
//! consumers depend on `overdrive-testing` only.
//!
//! # Expansion
//!
//! The test function keeps its signature and its other attributes (for
//! example `#[tokio::test]`). Its body is wrapped so that:
//!
//! - where `overdrive_testing::guest_suite::inside_guest()` holds, the
//!   original body runs;
//! - everywhere else the host-side half hands the running test binary to
//!   `overdrive_testing::guest_suite::run_in_guest(group, exe, test_name)` on
//!   a plain OS thread (the guest kit drives its own runtime, which must not
//!   start inside an async test's runtime) and panics with the typed error
//!   when the in-guest run fails. The body never runs on the host.
//!
//! A test with no test attribute of its own gets `#[test]`.
//!
//! # Placement marker
//!
//! Each declared test also emits a record into the `overdrive_test_placement`
//! ELF section of the test binary: the test's full module path, a NUL, the
//! group, a NUL. `infra/metal/test-wrapper.sh` reads that section to place a
//! test that has no entry in the placement table, so a test is placed only by
//! an explicit declaration; a plain `#[test]` carries no record and is still
//! refused as unclassified. The test function references the record, so the
//! linker keeps its section.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned as _;
use syn::{Attribute, Expr, ExprLit, ItemFn, Lit, Meta, ReturnType, parse_macro_input};

/// Declares where a Metal integration test runs. The only supported form is
/// `#[overdrive_test(guest = "<group>")]`.
#[proc_macro_attribute]
pub fn overdrive_test(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as ItemFn);
    match expand(&attribute.into(), function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(attribute: &TokenStream2, mut function: ItemFn) -> syn::Result<TokenStream2> {
    let group = guest_group(attribute)?;
    if !matches!(function.sig.output, ReturnType::Default) {
        return Err(syn::Error::new(
            function.sig.output.span(),
            "#[overdrive_test] supports tests that return `()`",
        ));
    }
    let declares_test = function.attrs.iter().any(is_test_attribute);
    if !declares_test {
        if function.sig.asyncness.is_some() {
            return Err(syn::Error::new(
                function.sig.asyncness.span(),
                "an async #[overdrive_test] needs its runtime's test attribute, e.g. #[tokio::test]",
            ));
        }
        function.attrs.push(syn::parse_quote!(#[test]));
    }
    let name = &function.sig.ident;
    let gate = gate(&group, &name.to_string());
    let original = &function.block;
    function.block = syn::parse_quote!({
        #gate
        #original
    });
    Ok(quote!(#function))
}

fn guest_group(attribute: &TokenStream2) -> syn::Result<String> {
    let meta: Meta = syn::parse2(attribute.clone()).map_err(|_| expected_form(attribute))?;
    let Meta::NameValue(pair) = meta else { return Err(expected_form(attribute)) };
    let (true, Expr::Lit(ExprLit { lit: Lit::Str(group), .. })) =
        (pair.path.is_ident("guest"), &pair.value)
    else {
        return Err(expected_form(attribute));
    };
    Ok(group.value())
}

fn expected_form(attribute: &TokenStream2) -> syn::Error {
    syn::Error::new(
        attribute.span(),
        "expected #[overdrive_test(guest = \"<group>\")]; the guest case is the only supported placement",
    )
}

fn is_test_attribute(attribute: &Attribute) -> bool {
    attribute.path().segments.last().is_some_and(|segment| segment.ident == "test")
}

/// The statements prepended to the test body: the placement record, then the
/// guest hand-off for the host-side half.
fn gate(group: &str, name: &str) -> TokenStream2 {
    quote! {
        const __OVERDRIVE_TEST_FULL_NAME: &str = ::core::concat!(::core::module_path!(), "::", #name);
        const __OVERDRIVE_TEST_GROUP: &str = #group;
        const __OVERDRIVE_TEST_RECORD_LEN: usize =
            __OVERDRIVE_TEST_FULL_NAME.len() + 1 + __OVERDRIVE_TEST_GROUP.len() + 1;
        #[used]
        #[unsafe(link_section = "overdrive_test_placement")]
        static __OVERDRIVE_TEST_RECORD: [u8; __OVERDRIVE_TEST_RECORD_LEN] = {
            let mut record = [0_u8; __OVERDRIVE_TEST_RECORD_LEN];
            let name = __OVERDRIVE_TEST_FULL_NAME.as_bytes();
            let group = __OVERDRIVE_TEST_GROUP.as_bytes();
            let mut index = 0;
            while index < name.len() {
                record[index] = name[index];
                index += 1;
            }
            let mut index = 0;
            while index < group.len() {
                record[name.len() + 1 + index] = group[index];
                index += 1;
            }
            record
        };
        // The record lives only as long as the linker keeps this function's
        // section; reading it keeps the record's section referenced.
        ::core::hint::black_box(&__OVERDRIVE_TEST_RECORD);
        if !::overdrive_testing::guest_suite::inside_guest() {
            let test_name = match __OVERDRIVE_TEST_FULL_NAME.split_once("::") {
                ::core::option::Option::Some((_crate_name, rest)) => rest,
                ::core::option::Option::None => __OVERDRIVE_TEST_FULL_NAME,
            };
            let exe = ::std::env::current_exe()
                .unwrap_or_else(|error| ::core::panic!("reading the test binary path: {error}"));
            let outcome = ::std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        ::overdrive_testing::guest_suite::run_in_guest(
                            __OVERDRIVE_TEST_GROUP,
                            &exe,
                            test_name,
                        )
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
