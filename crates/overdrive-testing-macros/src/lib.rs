//! The placement declarations of a Metal integration test:
//! `#[overdrive_test]` (the module form, reading the module's `shared_guest!`
//! declaration), `#[overdrive_test(host_pure)]`, `#[overdrive_test(native_launcher)]`,
//! `#[overdrive_test(guest = "<group>" [, profile = "<profile>"])]`,
//! `#[overdrive_test(lane_operator =
//! "<fixture>")]`, `#[overdrive_test(lane_owner = "<fixture>")]`,
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
//! `host_pure` and `native_launcher` leave the function untouched, so they apply
//! to any test function, including one that returns a `Result`, and reference
//! no runtime crate. `native_launcher` is the placement of a test of the Metal
//! runner's own machinery, which must observe the host from outside any guest.
//!
//! The module form, `guest = "<group>" [, profile = "<profile>"]`, `lane_operator =
//! "<fixture>"` and `lane_owner = "<fixture>"` wrap the body so that:
//!
//! - where `overdrive_testing::placement::inside_guest()` holds, or
//!   `overdrive_testing::placement::in_metal_run()` does not, the original body
//!   runs in place (off a Metal run the machine is private to the process);
//! - on a Metal run, outside a guest, the host-side half hands the running test
//!   binary to `overdrive_testing::placement::run_in_guest(&placement, exe,
//!   test_name)` (a typed `GuestPlacement`; for a lane form,
//!   `run_lane(&LanePlacement, exe, test_name)`) on a plain OS thread (the guest
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
//! (arguments: group, profile, `"1"` when the placement came from the module's
//! `shared_guest!` declaration and `"0"` for an explicit guest form),
//! `native_launcher`, `lane_operator` and `lane_owner`,
//! `module_host_pure` (`placement!`; `<test>` is the declaring module's path)
//! and `default_host_pure` (`overdrive_test_default!`; `<test>` is empty).
//! `infra/metal/test-wrapper.sh` reads the section to place a test that has no
//! table entry, so a test is placed only by an explicit declaration. The lane
//! kinds `lane_operator` and `lane_owner` carry the fixture as their one
//! argument; `native_launcher` carries none.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt as _;
use syn::parse::Parser as _;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned as _;
use syn::{
    Expr, ExprLit, Ident, ItemFn, Lit, LitStr, Meta, MetaNameValue, ReturnType, Token,
    parse_macro_input,
};

/// The ELF section the placement records are written to.
const SECTION: &str = "overdrive_test_placement";

/// The module-private item `shared_guest!` declares and the argument-free
/// `#[overdrive_test]` names by plain path (rustc's E0425 prints this name when
/// a module has no declaration).
const SHARED_GUEST_DECLARATION: &str =
    "OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE";

/// Declares where a Metal integration test runs.
///
/// Implemented for `#[overdrive_test]` (the module form, which reads the
/// module's `shared_guest!` declaration), `#[overdrive_test(host_pure)]`,
/// `#[overdrive_test(native_launcher)]`,
/// `#[overdrive_test(guest = "<group>" [, profile = "<profile>"])]`,
/// `#[overdrive_test(lane_operator = "<fixture>")]` and
/// `#[overdrive_test(lane_owner = "<fixture>")]`. Any other argument is a build
/// error.
#[proc_macro_attribute]
pub fn overdrive_test(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let attribute: TokenStream2 = attribute.into();
    let Some(declaration) = Declaration::parse(&attribute) else {
        let error = unrecognised_argument(&attribute, "#[overdrive_test]").to_compile_error();
        let item: TokenStream2 = item.into();
        return quote!(#error #item).into();
    };
    let function = parse_macro_input!(item as ItemFn);
    match declaration.expand(function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// The build error for an argument list no form of `macro_name` accepts.
/// `shared` names the declaration that owns sharing; anything else is reported
/// as unrecognised.
fn unrecognised_argument(attribute: &TokenStream2, macro_name: &str) -> syn::Error {
    let shared = attribute.clone().into_iter().find_map(|token| match token {
        proc_macro2::TokenTree::Ident(ident) if ident == "shared" => Some(ident.span()),
        _ => None,
    });
    shared.map_or_else(
        || syn::Error::new(attribute.span(), format!("unrecognised argument to {macro_name}")),
        |span| syn::Error::new(span, "sharing is declared by `shared_guest!`"),
    )
}

/// `shared_guest!(guest = "<group>" [, profile = "<profile>"] [,])`: the
/// module's shared-guest declaration.
///
/// Expands to one module-private item,
/// `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE`, that the
/// argument-free `#[overdrive_test]` of the module names by plain path.
///
/// An unknown group or profile is a build error at the offending literal; so is
/// an unknown, repeated or `shared` argument and a missing `guest`. A second
/// declaration in one module is rustc's E0428.
#[proc_macro]
pub fn shared_guest(declaration: TokenStream) -> TokenStream {
    let declaration: TokenStream2 = declaration.into();
    let Some((group, profile)) = parse_shared_guest(&declaration) else {
        return unrecognised_argument(&declaration, "shared_guest!").to_compile_error().into();
    };
    let profile = profile.unwrap_or_else(|| LitStr::new("", group.span()));
    let name = Ident::new(SHARED_GUEST_DECLARATION, Span::call_site());
    let known_group = known_group_statement(&group);
    let known_profile = known_profile_statement(&group, &profile);
    quote! {
        #[allow(dead_code, non_upper_case_globals)]
        const #name: ::overdrive_testing::placement::GuestPlacement = {
            #known_group
            #known_profile
            ::overdrive_testing::placement::GuestPlacement::declare(
                #group,
                #profile,
                ::overdrive_testing::placement::Sharing::Module,
                ::core::module_path!(),
            )
        };
    }
    .into()
}

/// `guest = "<group>"` with an optional `profile = "<profile>"` in either
/// order, each at most once and `guest` required.
fn parse_shared_guest(declaration: &TokenStream2) -> Option<(LitStr, Option<LitStr>)> {
    match Declaration::parse_guest(declaration)? {
        Declaration::Guest { group, profile } => Some((group, profile)),
        _ => None,
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
    /// The argument-free form: the module's `shared_guest!` declaration.
    Module,
    HostPure,
    NativeLauncher,
    Guest {
        group: LitStr,
        profile: Option<LitStr>,
    },
    Lane {
        owner: bool,
        fixture: LitStr,
    },
}

impl Declaration {
    fn parse(attribute: &TokenStream2) -> Option<Self> {
        if attribute.is_empty() {
            return Some(Self::Module);
        }
        if let Some(guest) = Self::parse_guest(attribute) {
            return Some(guest);
        }
        let meta: Meta = syn::parse2(attribute.clone()).ok()?;
        match meta {
            Meta::Path(path) if path.is_ident("host_pure") => Some(Self::HostPure),
            Meta::Path(path) if path.is_ident("native_launcher") => Some(Self::NativeLauncher),
            Meta::NameValue(pair) => {
                let Expr::Lit(ExprLit { lit: Lit::Str(value), .. }) = pair.value else {
                    return None;
                };
                if pair.path.is_ident("lane_operator") {
                    Some(Self::Lane { owner: false, fixture: value })
                } else if pair.path.is_ident("lane_owner") {
                    Some(Self::Lane { owner: true, fixture: value })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `guest = "<group>"` with an optional `profile = "<profile>"` in either
    /// order, each at most once.
    fn parse_guest(attribute: &TokenStream2) -> Option<Self> {
        let pairs = Punctuated::<MetaNameValue, Token![,]>::parse_terminated
            .parse2(attribute.clone())
            .ok()?;
        let mut group = None;
        let mut profile = None;
        for pair in pairs {
            let Expr::Lit(ExprLit { lit: Lit::Str(value), .. }) = pair.value else {
                return None;
            };
            let slot = if pair.path.is_ident("guest") {
                &mut group
            } else if pair.path.is_ident("profile") {
                &mut profile
            } else {
                return None;
            };
            if slot.replace(value).is_some() {
                return None;
            }
        }
        group.map(|group| Self::Guest { group, profile })
    }

    fn expand(&self, mut function: ItemFn) -> syn::Result<TokenStream2> {
        let name = function.sig.ident.unraw();
        let item = record_name(&name);
        let test = test_name(&name);
        match self {
            Self::HostPure | Self::NativeLauncher => {
                let kind =
                    if matches!(self, Self::HostPure) { "host_pure" } else { "native_launcher" };
                let record = record_item(&item, &quote!(#kind), &test, &[]);
                Ok(quote!(#function #record))
            }
            Self::Guest { group, profile } => {
                require_unit_return(&function)?;
                let profile = profile.clone().unwrap_or_else(|| LitStr::new("", group.span()));
                let record = record_item(
                    &item,
                    &quote!("guest"),
                    &test,
                    &[quote!(#group), quote!(#profile), quote!("0")],
                );
                let known_group = known_group_check(group);
                let known_profile = known_profile_check(group, &profile);
                let hand_off = quote! {
                    ::overdrive_testing::placement::run_in_guest(
                        &::overdrive_testing::placement::GuestPlacement::declare(
                            #group,
                            #profile,
                            ::overdrive_testing::placement::Sharing::Isolated,
                            "",
                        ),
                        &exe,
                        test_name,
                    )
                };
                guard_body(&mut function, &name.to_string(), &hand_off);
                Ok(quote!(#function #record #known_group #known_profile))
            }
            Self::Module => {
                require_unit_return(&function)?;
                let declaration = Ident::new(SHARED_GUEST_DECLARATION, Span::call_site());
                let record = record_item(
                    &item,
                    &quote!("guest"),
                    &test,
                    &[
                        quote!(#declaration.declared_group()),
                        quote!(#declaration.declared_profile()),
                        quote!("1"),
                    ],
                );
                let hand_off = quote! {
                    ::overdrive_testing::placement::run_in_guest(&#declaration, &exe, test_name)
                };
                guard_body(&mut function, &name.to_string(), &hand_off);
                Ok(quote!(#function #record))
            }
            Self::Lane { owner, fixture } => {
                require_unit_return(&function)?;
                require_lane_fixture(fixture)?;
                let kind = if *owner { "lane_owner" } else { "lane_operator" };
                let record = record_item(&item, &quote!(#kind), &test, &[quote!(#fixture)]);
                let constructor = format_ident!("{}", if *owner { "owner" } else { "operator" });
                let hand_off = quote! {
                    ::overdrive_testing::placement::run_lane(
                        &::overdrive_testing::placement::LanePlacement::#constructor(#fixture),
                        &exe,
                        test_name,
                    )
                };
                guard_body(&mut function, &name.to_string(), &hand_off);
                Ok(quote!(#function #record))
            }
        }
    }
}

/// The guest forms apply to a test that returns `()`.
fn require_unit_return(function: &ItemFn) -> syn::Result<()> {
    if matches!(function.sig.output, ReturnType::Default) {
        return Ok(());
    }
    Err(syn::Error::new(
        function.sig.output.span(),
        "#[overdrive_test] supports tests that return `()`",
    ))
}

/// The grammar of a lane fixture: `integration::in_guest_lane::<ident>`.
fn require_lane_fixture(fixture: &LitStr) -> syn::Result<()> {
    let value = fixture.value();
    let well_formed = value.strip_prefix("integration::in_guest_lane::").is_some_and(|name| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    if well_formed {
        return Ok(());
    }
    Err(syn::Error::new(
        fixture.span(),
        "a lane fixture is named `integration::in_guest_lane::<name>`",
    ))
}

/// Prepends the guest gate to the body of `function`.
fn guard_body(function: &mut ItemFn, name: &str, hand_off: &TokenStream2) {
    let gate = gate(name, hand_off);
    let original = &function.block;
    function.block = syn::parse_quote!({
        #gate
        #original
    });
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

/// The build-time check that the group is one of the registry's, as a statement
/// spanned on the user's literal so rustc's E0080 points at the offending
/// argument, with the value named in the message.
fn known_group_statement(group: &LitStr) -> TokenStream2 {
    let message = LitStr::new(
        &format!("unknown guest group `{}`", group.value().replace('{', "{{").replace('}', "}}")),
        group.span(),
    );
    quote_spanned! {group.span()=>
        if !::overdrive_testing::placement::groups::is_known_group(#group) {
            ::core::panic!(#message);
        }
    }
}

/// The build-time check that the profile is one of the group's, as a statement
/// spanned on the user's profile literal. Skipped for an unknown group, which
/// `known_group_statement` already reports.
fn known_profile_statement(group: &LitStr, profile: &LitStr) -> TokenStream2 {
    let escape = |value: String| value.replace('{', "{{").replace('}', "}}");
    let message = LitStr::new(
        &format!(
            "unknown guest profile `{}` for group `{}`",
            escape(profile.value()),
            escape(group.value())
        ),
        profile.span(),
    );
    quote_spanned! {profile.span()=>
        if ::overdrive_testing::placement::groups::is_known_group(#group)
            && !::overdrive_testing::placement::groups::is_known_profile(#group, #profile)
        {
            ::core::panic!(#message);
        }
    }
}

/// The group check as a sibling of the test function, so it is evaluated
/// whatever test attribute decides the function's fate.
fn known_group_check(group: &LitStr) -> TokenStream2 {
    let statement = known_group_statement(group);
    quote!(const _: () = { #statement };)
}

/// The profile check as a sibling of the test function.
fn known_profile_check(group: &LitStr, profile: &LitStr) -> TokenStream2 {
    let statement = known_profile_statement(group, profile);
    quote!(const _: () = { #statement };)
}

/// The statements prepended to a guest- or lane-placed test body: the hand-off
/// for the host-side half. `hand_off` evaluates to `Result<(), RunInGuestError>`
/// over the test binary `exe` and the libtest name `test_name` in scope.
fn gate(name: &str, hand_off: &TokenStream2) -> TokenStream2 {
    quote! {
        const __OVERDRIVE_TEST_FULL_NAME: &str = ::core::concat!(::core::module_path!(), "::", #name);
        if !::overdrive_testing::placement::inside_guest()
            && ::overdrive_testing::placement::in_metal_run()
        {
            let test_name = match __OVERDRIVE_TEST_FULL_NAME.split_once("::") {
                ::core::option::Option::Some((_crate_name, rest)) => rest,
                ::core::option::Option::None => __OVERDRIVE_TEST_FULL_NAME,
            };
            let exe = ::std::env::current_exe()
                .unwrap_or_else(|error| ::core::panic!("reading the test binary path: {error}"));
            let outcome = ::std::thread::scope(|scope| {
                scope.spawn(|| #hand_off).join()
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
