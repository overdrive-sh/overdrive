//! PROBE V-11 macros. `*_fixed`: emit a const calling `GuestPlacement::declare` with the
//! user's own literal token. `*_embed`: the macro itself embeds the user's literal in the
//! panic message via concat!. `*_embed_known`: also names the known set via a registry macro.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, ItemFn, LitStr, Token, parse_macro_input};

struct Args {
    guest: LitStr,
    profile: Option<LitStr>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        if key != "guest" {
            return Err(syn::Error::new(key.span(), "expected `guest = \"<group>\"`"));
        }
        input.parse::<Token![=]>()?;
        let guest: LitStr = input.parse()?;
        let mut profile = None;
        if input.parse::<Option<Token![,]>>()?.is_some() && !input.is_empty() {
            let key: Ident = input.parse()?;
            if key != "profile" {
                return Err(syn::Error::new(key.span(), "expected `profile = \"<name>\"`"));
            }
            input.parse::<Token![=]>()?;
            profile = Some(input.parse()?);
        }
        Ok(Self { guest, profile })
    }
}

fn declare_fixed(args: &Args) -> TokenStream2 {
    let guest = &args.guest;
    let profile = args.profile.clone().unwrap_or_else(|| LitStr::new("base", args.guest.span()));
    quote! {
        const _: ::const_diag_probe::GuestPlacement =
            ::const_diag_probe::GuestPlacement::declare(#guest, #profile);
    }
}

fn declare_embed(args: &Args, known: bool) -> TokenStream2 {
    let guest = &args.guest;
    let message = if known {
        quote!(::core::concat!(
            "unknown guest group `", #guest, "`; known groups: ",
            ::const_diag_probe::known_guest_groups!()
        ))
    } else {
        quote!(::core::concat!("unknown guest group `", #guest, "`"))
    };
    quote! {
        const _: () = {
            if !::const_diag_probe::is_known_group(#guest) {
                ::core::panic!(#message);
            }
        };
    }
}

fn declare_embed_spanned(args: &Args) -> TokenStream2 {
    let guest = &args.guest;
    let span = guest.span();
    quote_spanned! {span=>
        const _: () = {
            if !::const_diag_probe::is_known_group(#guest) {
                ::core::panic!(::core::concat!(
                    "unknown guest group `", #guest, "`; known groups: ",
                    ::const_diag_probe::known_guest_groups!()
                ));
            }
        };
    }
}

fn declare_fixed_spanned(args: &Args) -> TokenStream2 {
    let guest = &args.guest;
    let span = guest.span();
    let profile = args.profile.clone().unwrap_or_else(|| LitStr::new("base", span));
    quote_spanned! {span=>
        const _: ::const_diag_probe::GuestPlacement =
            ::const_diag_probe::GuestPlacement::declare(#guest, #profile);
    }
}

fn shared(input: TokenStream, mode: u8) -> TokenStream {
    let args = parse_macro_input!(input as Args);
    match mode {
        0 => declare_fixed(&args),
        1 => declare_embed(&args, false),
        2 => declare_embed(&args, true),
        3 => declare_embed_spanned(&args),
        _ => declare_fixed_spanned(&args),
    }
    .into()
}

fn attr(attribute: TokenStream, item: TokenStream, mode: u8) -> TokenStream {
    let args = parse_macro_input!(attribute as Args);
    let mut function = parse_macro_input!(item as ItemFn);
    let decl = match mode {
        0 => declare_fixed(&args),
        1 => declare_embed(&args, false),
        2 => declare_embed(&args, true),
        3 => declare_embed_spanned(&args),
        _ => declare_fixed_spanned(&args),
    };
    function.attrs.push(syn::parse_quote!(#[test]));
    let block = &function.block;
    function.block = syn::parse_quote!({ #decl #block });
    quote!(#function).into()
}

#[proc_macro]
pub fn shared_guest(input: TokenStream) -> TokenStream { shared(input, 0) }
#[proc_macro]
pub fn shared_guest_embed(input: TokenStream) -> TokenStream { shared(input, 1) }
#[proc_macro]
pub fn shared_guest_embed_known(input: TokenStream) -> TokenStream { shared(input, 2) }
#[proc_macro_attribute]
pub fn overdrive_test(a: TokenStream, i: TokenStream) -> TokenStream { attr(a, i, 0) }
#[proc_macro_attribute]
pub fn overdrive_test_embed(a: TokenStream, i: TokenStream) -> TokenStream { attr(a, i, 1) }
#[proc_macro_attribute]
pub fn overdrive_test_embed_known(a: TokenStream, i: TokenStream) -> TokenStream { attr(a, i, 2) }
#[proc_macro]
pub fn shared_guest_spanned(input: TokenStream) -> TokenStream { shared(input, 3) }
#[proc_macro]
pub fn shared_guest_fixed_spanned(input: TokenStream) -> TokenStream { shared(input, 4) }
#[proc_macro_attribute]
pub fn overdrive_test_spanned(a: TokenStream, i: TokenStream) -> TokenStream { attr(a, i, 3) }
#[proc_macro_attribute]
pub fn overdrive_test_fixed_spanned(a: TokenStream, i: TokenStream) -> TokenStream { attr(a, i, 4) }
