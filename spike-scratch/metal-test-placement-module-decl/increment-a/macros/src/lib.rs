//! PROBE V-1: stable-Rust forms of a per-module placement declaration.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::spanned::Spanned as _;
use syn::{Attribute, Expr, ExprLit, ItemFn, Lit, Meta, ReturnType, parse_macro_input};

#[cfg(not(feature = "descriptive-name"))]
const MODULE_CONST: &str = "__OVERDRIVE_MODULE_PLACEMENT";
#[cfg(feature = "descriptive-name")]
const MODULE_CONST: &str = "OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE";

const GROUPS: &[&str] = &[
    "probe", "core", "unload", "ulp", "mtls-relay", "cgroup-default", "stress", "nested-boundary",
];

/// Function-like: `shared_guest!(guest = "<group>")`, once per module.
#[proc_macro]
pub fn shared_guest(input: TokenStream) -> TokenStream {
    let input: TokenStream2 = input.into();
    let group = match parse_group(&input) {
        Ok(group) => group,
        Err(error) => return error.to_compile_error().into(),
    };
    let name = format_ident!("{}", MODULE_CONST);
    quote! {
        #[doc(hidden)]
        #[allow(dead_code, non_upper_case_globals)]
        const #name: &str = #group;
    }
    .into()
}

fn parse_group(input: &TokenStream2) -> syn::Result<String> {
    let form = || syn::Error::new(input.span(), "expected shared_guest!(guest = \"<group>\")");
    let meta: Meta = syn::parse2(input.clone()).map_err(|_| form())?;
    let Meta::NameValue(pair) = meta else { return Err(form()) };
    let (true, Expr::Lit(ExprLit { lit: Lit::Str(group), .. })) =
        (pair.path.is_ident("guest"), &pair.value)
    else {
        return Err(form());
    };
    let group = group.value();
    if !GROUPS.contains(&group.as_str()) {
        return Err(syn::Error::new(pair.value.span(), format!("unknown guest group `{group}`")));
    }
    Ok(group)
}

/// No-argument test attribute; the group comes from the module's const.
#[proc_macro_attribute]
pub fn overdrive_test(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as ItemFn);
    match expand(&attribute.into(), function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(attribute: &TokenStream2, mut function: ItemFn) -> syn::Result<TokenStream2> {
    if !attribute.is_empty() {
        return Err(syn::Error::new(attribute.span(), "#[overdrive_test] takes no arguments"));
    }
    if !matches!(function.sig.output, ReturnType::Default) {
        return Err(syn::Error::new(function.sig.output.span(), "tests must return `()`"));
    }
    if !function.attrs.iter().any(is_test_attribute) {
        if function.sig.asyncness.is_some() {
            return Err(syn::Error::new(
                function.sig.asyncness.span(),
                "an async #[overdrive_test] needs its runtime's test attribute",
            ));
        }
        function.attrs.push(syn::parse_quote!(#[test]));
    }
    let name = function.sig.ident.to_string();
    let gate = gate(&name);
    let original = &function.block;
    function.block = syn::parse_quote!({
        #gate
        #original
    });
    Ok(quote!(#function))
}

fn is_test_attribute(attribute: &Attribute) -> bool {
    attribute.path().segments.last().is_some_and(|segment| segment.ident == "test")
}

fn gate(name: &str) -> TokenStream2 {
    let module_const = format_ident!("{}", MODULE_CONST);
    let keep = if cfg!(feature = "no-blackbox") {
        quote!()
    } else {
        quote!(::core::hint::black_box(&__OVERDRIVE_TEST_RECORD);)
    };
    quote! {
        const __OVERDRIVE_TEST_FULL_NAME: &str = ::core::concat!(::core::module_path!(), "::", #name);
        // The plain-path reference to the module's declaration.
        const __OVERDRIVE_TEST_GROUP: &str = #module_const;
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
        #keep
        ::std::println!(
            "PLACEMENT_PROBE test={} group={}",
            __OVERDRIVE_TEST_FULL_NAME,
            __OVERDRIVE_TEST_GROUP
        );
    }
}

/// Identity attribute: used only to see which positions stable accepts.
#[proc_macro_attribute]
pub fn module_placement(_attribute: TokenStream, item: TokenStream) -> TokenStream {
    item
}
