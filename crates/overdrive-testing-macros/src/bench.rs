//! The `#[overdrive_bench(...)]` benchmark declaration attribute (ADR-0297, DDD-2).
//!
//! The attribute validates the declaration at build time, registers it in
//! `overdrive_bench`'s link-time slice (DDD-3) and, unless `smoke(none)`, emits the
//! `smoke_<name>` test that runs one replicate per arm. A rejection that needs a
//! registry the proc-macro cannot see (the substrate, `smoke(none)`'s location, the
//! guest group) is a constant evaluation, printed by rustc as
//! `error[E0080]: evaluation panicked: <message>`; every other rejection is a
//! plain build error naming the offending key or value.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt as _;
use syn::parse::{Parse, ParseStream, Parser as _};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned as _;
use syn::{Expr, ExprArray, ExprLit, ExprUnary, Ident, ItemFn, Lit, LitStr, Token, UnOp};

use crate::{known_group_check, known_profile_check};

/// The fewest replicates per arm the statistical protocol (DDD-5) can use.
const MINIMUM_REPLICATES: u32 = 9;

/// The metric units a declaration may name.
const UNITS: [&str; 6] = ["ns", "bytes", "bytes_per_second", "qps", "ratio", "count"];

/// The service clock sources a declaration may name, with the `ServiceClock` variant each emits.
const SERVICE_CLOCKS: [(&str, &str); 1] = [("injected_monotonic", "InjectedMonotonic")];

/// The guest clock sources a declaration may name, with the `GuestClock` variant each emits.
const GUEST_CLOCKS: [(&str, &str); 2] =
    [("init_elapsed_ms", "InitElapsedMs"), ("guest_monotonic", "Monotonic")];

/// The top-level keys, in the order the unknown-key message lists them.
const KEYS: &str =
    "family, substrate, arms, replicates, profile, metric, bpf_stats, reference, clock, smoke";

const SMOKE_HINT: &str =
    "give smoke(host_pure), smoke(guest = \"<group>\") or, in a spike, smoke(none)";

pub fn expand(attribute: TokenStream, item: &TokenStream) -> TokenStream {
    let function: ItemFn = match syn::parse2(item.clone()) {
        Ok(function) => function,
        Err(error) => {
            let error = error.to_compile_error();
            return quote!(#error #item);
        }
    };
    match Declaration::parse(attribute) {
        Ok(declaration) => declaration.expand(&function),
        Err(error) => {
            let error = error.to_compile_error();
            quote!(#[allow(dead_code)] #function #error)
        }
    }
}

/// One `key`, `key = <expr>` or `key(<tokens>)` entry of an argument list.
struct Entry {
    key: Ident,
    value: Value,
}

enum Value {
    Flag,
    Eq(Expr),
    Group(TokenStream),
}

impl Parse for Entry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key = Ident::parse_any(input)?;
        let value = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Value::Eq(input.parse()?)
        } else if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            Value::Group(content.parse()?)
        } else {
            Value::Flag
        };
        Ok(Self { key, value })
    }
}

fn entries(tokens: TokenStream) -> syn::Result<Vec<Entry>> {
    Punctuated::<Entry, Token![,]>::parse_terminated
        .parse2(tokens)
        .map(|entries| entries.into_iter().collect())
}

fn expected(span: Span, key: &str, what: &str) -> syn::Error {
    syn::Error::new(span, format!("key `{key}`: expected {what}"))
}

const fn string_literal(expression: &Expr) -> Option<&LitStr> {
    match expression {
        Expr::Lit(ExprLit { lit: Lit::Str(literal), .. }) => Some(literal),
        _ => None,
    }
}

impl Entry {
    fn name(&self) -> String {
        self.key.unraw().to_string()
    }

    fn expression(&self, what: &str) -> syn::Result<&Expr> {
        match &self.value {
            Value::Eq(expression) => Ok(expression),
            Value::Flag | Value::Group(_) => Err(expected(self.key.span(), &self.name(), what)),
        }
    }

    fn string(&self) -> syn::Result<LitStr> {
        let what = "a string literal";
        string_literal(self.expression(what)?)
            .cloned()
            .ok_or_else(|| expected(self.key.span(), &self.name(), what))
    }

    fn group(&self, what: &str) -> syn::Result<TokenStream> {
        match &self.value {
            Value::Group(tokens) => Ok(tokens.clone()),
            Value::Flag | Value::Eq(_) => Err(expected(self.key.span(), &self.name(), what)),
        }
    }
}

fn once<T>(slot: &mut Option<T>, entry: &Entry, value: T) -> syn::Result<()> {
    if slot.replace(value).is_some() {
        return Err(syn::Error::new(
            entry.key.span(),
            format!("key `{}` is given more than once", entry.name()),
        ));
    }
    Ok(())
}

struct Profile {
    name: LitStr,
    preparation: LitStr,
}

struct Metric {
    name: LitStr,
    unit: Ident,
    table: LitStr,
    series: bool,
}

struct Reference {
    metric: LitStr,
    value: f64,
    label: LitStr,
    source: LitStr,
}

/// The declared clock sources: the `ServiceClock` and `GuestClock` variant names, each `None`
/// when its sub-key is omitted.
struct Clock {
    service: Option<&'static str>,
    guest: Option<&'static str>,
}

enum Smoke {
    HostPure,
    Guest { group: LitStr, profile: Option<LitStr> },
    None(Span),
}

struct Declaration {
    family: LitStr,
    substrate: Ident,
    arms: Vec<LitStr>,
    replicates: u32,
    profiles: Vec<Profile>,
    metrics: Vec<Metric>,
    bpf_stats: bool,
    references: Vec<Reference>,
    clock: Clock,
    smoke: Smoke,
}

fn missing(key: &str) -> syn::Error {
    syn::Error::new(Span::call_site(), format!("missing required key `{key}`"))
}

impl Declaration {
    fn parse(attribute: TokenStream) -> syn::Result<Self> {
        let mut family = None;
        let mut substrate = None;
        let mut arms = None;
        let mut replicates = None;
        let mut profiles = Vec::new();
        let mut metrics = Vec::new();
        let mut bpf_stats = None;
        let mut references = Vec::new();
        let mut clock = None;
        let mut smoke = None;
        for entry in entries(attribute)? {
            match entry.name().as_str() {
                "family" => once(&mut family, &entry, entry.string()?)?,
                "substrate" => once(&mut substrate, &entry, parse_substrate(&entry)?)?,
                "arms" => once(&mut arms, &entry, parse_arms(&entry)?)?,
                "replicates" => once(&mut replicates, &entry, parse_replicates(&entry)?)?,
                "profile" => profiles.push(parse_profile(&entry)?),
                "metric" => metrics.push(parse_metric(&entry)?),
                "bpf_stats" => {
                    if !matches!(entry.value, Value::Flag) {
                        return Err(expected(entry.key.span(), "bpf_stats", "no value"));
                    }
                    once(&mut bpf_stats, &entry, true)?;
                }
                "reference" => references.push(parse_reference(&entry)?),
                "clock" => once(&mut clock, &entry, parse_clock(&entry)?)?,
                "smoke" => once(&mut smoke, &entry, parse_smoke(&entry)?)?,
                _ => {
                    return Err(syn::Error::new(
                        entry.key.span(),
                        format!(
                            "unknown key `{}` in `#[overdrive_bench(...)]`; the keys are {KEYS}",
                            entry.name()
                        ),
                    ));
                }
            }
        }
        if metrics.is_empty() {
            return Err(missing("metric"));
        }
        Ok(Self {
            family: family.ok_or_else(|| missing("family"))?,
            substrate: substrate.ok_or_else(|| missing("substrate"))?,
            arms: arms.ok_or_else(|| missing("arms"))?,
            replicates: replicates.ok_or_else(|| missing("replicates"))?,
            profiles,
            metrics,
            bpf_stats: bpf_stats.unwrap_or(false),
            references,
            clock: clock.unwrap_or(Clock { service: None, guest: None }),
            smoke: smoke.ok_or_else(|| {
                syn::Error::new(
                    Span::call_site(),
                    format!("missing required key `smoke`; {SMOKE_HINT}"),
                )
            })?,
        })
    }
}

fn parse_substrate(entry: &Entry) -> syn::Result<Ident> {
    let what = "an identifier such as `native`";
    let expression = entry.expression(what)?;
    if string_literal(expression).is_some() {
        return Err(syn::Error::new(
            expression.span(),
            "key `substrate`: expected an identifier such as `native`, found a string literal",
        ));
    }
    match expression {
        Expr::Path(path) if path.qself.is_none() => path
            .path
            .get_ident()
            .cloned()
            .ok_or_else(|| expected(expression.span(), "substrate", what)),
        _ => Err(expected(expression.span(), "substrate", what)),
    }
}

fn is_arm(arm: &str) -> bool {
    let mut characters = arm.chars();
    let starts = characters.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    starts
        && characters.all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '.' | '_' | '-')
        })
}

fn parse_arms(entry: &Entry) -> syn::Result<Vec<LitStr>> {
    let what = "an array of string literals";
    let Expr::Array(ExprArray { elems, .. }) = entry.expression(what)? else {
        return Err(expected(entry.key.span(), "arms", what));
    };
    let mut arms: Vec<LitStr> = Vec::with_capacity(elems.len());
    for element in elems {
        let arm =
            string_literal(element).ok_or_else(|| expected(element.span(), "arms", what))?.clone();
        if !is_arm(&arm.value()) {
            return Err(syn::Error::new(
                arm.span(),
                format!("arm `{}` does not match [a-z0-9][a-z0-9+._-]*", arm.value()),
            ));
        }
        if arms.iter().any(|seen| seen.value() == arm.value()) {
            return Err(syn::Error::new(
                arm.span(),
                format!("arm `{}` is declared more than once", arm.value()),
            ));
        }
        arms.push(arm);
    }
    if arms.is_empty() {
        return Err(expected(entry.key.span(), "arms", "at least one arm"));
    }
    Ok(arms)
}

fn parse_replicates(entry: &Entry) -> syn::Result<u32> {
    let what = "an integer literal";
    let expression = entry.expression(what)?;
    let Expr::Lit(ExprLit { lit: Lit::Int(literal), .. }) = expression else {
        return Err(expected(expression.span(), "replicates", what));
    };
    let count: u32 = literal.base10_parse().map_err(|_| {
        expected(expression.span(), "replicates", "an integer literal that fits a u32")
    })?;
    if count < MINIMUM_REPLICATES {
        return Err(syn::Error::new(
            expression.span(),
            format!("replicates = {count} is below the minimum of {MINIMUM_REPLICATES} per arm"),
        ));
    }
    Ok(count)
}

/// The sub-entries of `key(...)`, each of whose keys must be one of `allowed`.
fn group_entries(entry: &Entry, shape: &str, allowed: &[&str]) -> syn::Result<Vec<Entry>> {
    let key = entry.name();
    let entries = entries(entry.group(shape)?)?;
    for sub in &entries {
        if !allowed.contains(&sub.name().as_str()) {
            return Err(syn::Error::new(
                sub.key.span(),
                format!(
                    "unknown key `{}` in `{key}(...)`; the keys are {}",
                    sub.name(),
                    allowed.join(", ")
                ),
            ));
        }
    }
    Ok(entries)
}

fn find<'a>(entries: &'a [Entry], parent: &Entry, name: &str) -> syn::Result<&'a Entry> {
    let mut found = entries.iter().filter(|entry| entry.name() == name);
    let first = found.next().ok_or_else(|| {
        syn::Error::new(
            parent.key.span(),
            format!("missing required key `{name}` in `{}(...)`", parent.name()),
        )
    })?;
    found.next().map_or(Ok(first), |repeat| {
        Err(syn::Error::new(repeat.key.span(), format!("key `{name}` is given more than once")))
    })
}

fn parse_profile(entry: &Entry) -> syn::Result<Profile> {
    let entries = group_entries(
        entry,
        "(name = \"<kebab>\", preparation = \"<text>\")",
        &["name", "preparation"],
    )?;
    Ok(Profile {
        name: find(&entries, entry, "name")?.string()?,
        preparation: find(&entries, entry, "preparation")?.string()?,
    })
}

fn parse_unit(entry: &Entry) -> syn::Result<Ident> {
    let expression = entry.expression("an identifier naming a unit")?;
    let (name, span) = match expression {
        Expr::Path(path) if path.qself.is_none() => match path.path.get_ident() {
            Some(ident) => (ident.unraw().to_string(), ident.span()),
            None => return Err(expected(expression.span(), "unit", "an identifier naming a unit")),
        },
        other => match string_literal(other) {
            Some(literal) => (literal.value(), literal.span()),
            None => return Err(expected(other.span(), "unit", "an identifier naming a unit")),
        },
    };
    if !UNITS.contains(&name.as_str()) {
        return Err(syn::Error::new(
            span,
            format!("unit `{name}` is not one of {}", UNITS.join(", ")),
        ));
    }
    Ok(Ident::new(&name, span))
}

fn parse_metric(entry: &Entry) -> syn::Result<Metric> {
    let entries = group_entries(
        entry,
        "(name = \"<snake>\", unit = <unit>, table = \"<kebab>\" [, series])",
        &["name", "unit", "table", "series"],
    )?;
    let series = match entries.iter().find(|sub| sub.name() == "series") {
        None => false,
        Some(sub) if matches!(sub.value, Value::Flag) => true,
        Some(sub) => return Err(expected(sub.key.span(), "series", "no value")),
    };
    Ok(Metric {
        name: find(&entries, entry, "name")?.string()?,
        unit: parse_unit(find(&entries, entry, "unit")?)?,
        table: find(&entries, entry, "table")?.string()?,
        series,
    })
}

fn number(expression: &Expr) -> Option<f64> {
    match expression {
        Expr::Lit(ExprLit { lit: Lit::Float(literal), .. }) => literal.base10_parse().ok(),
        Expr::Lit(ExprLit { lit: Lit::Int(literal), .. }) => literal.base10_parse().ok(),
        Expr::Unary(ExprUnary { op: UnOp::Neg(_), expr, .. }) => number(expr).map(|value| -value),
        _ => None,
    }
}

fn parse_reference(entry: &Entry) -> syn::Result<Reference> {
    let entries = group_entries(
        entry,
        "(metric = \"<snake>\", value = <float>, label = \"<text>\", source = \"<path>\")",
        &["metric", "value", "label", "source"],
    )?;
    let value = find(&entries, entry, "value")?;
    let expression = value.expression("a number literal")?;
    Ok(Reference {
        metric: find(&entries, entry, "metric")?.string()?,
        value: number(expression)
            .ok_or_else(|| expected(expression.span(), "value", "a number literal"))?,
        label: find(&entries, entry, "label")?.string()?,
        source: find(&entries, entry, "source")?.string()?,
    })
}

/// The variant name of the source `entry` names, from the closed `sources` of `kind`.
fn parse_source(
    entry: &Entry,
    kind: &str,
    sources: &[(&str, &'static str)],
) -> syn::Result<&'static str> {
    let first = sources.first().map_or("", |(identifier, _)| identifier);
    let what = format!("an identifier such as `{first}`");
    let expression = entry.expression(&what)?;
    if string_literal(expression).is_some() {
        return Err(syn::Error::new(
            expression.span(),
            format!("key `clock`: expected {what}, found a string literal"),
        ));
    }
    let (name, span) = match expression {
        Expr::Path(path) if path.qself.is_none() => match path.path.get_ident() {
            Some(ident) => (ident.unraw().to_string(), ident.span()),
            None => return Err(expected(expression.span(), "clock", &what)),
        },
        _ => return Err(expected(expression.span(), "clock", &what)),
    };
    if let Some((_, variant)) = sources.iter().find(|(identifier, _)| *identifier == name) {
        return Ok(variant);
    }
    let set: Vec<&str> = sources.iter().map(|(identifier, _)| *identifier).collect();
    Err(syn::Error::new(
        span,
        format!("{kind} clock source `{name}` is not one of {}", set.join(", ")),
    ))
}

fn parse_clock(entry: &Entry) -> syn::Result<Clock> {
    let what = "service = <source> and/or guest = <source>";
    let entries = group_entries(entry, what, &["service", "guest"])?;
    if entries.is_empty() {
        return Err(expected(entry.key.span(), "clock", what));
    }
    let mut service = None;
    let mut guest = None;
    for sub in &entries {
        match sub.name().as_str() {
            "service" => once(&mut service, sub, parse_source(sub, "service", &SERVICE_CLOCKS)?)?,
            _ => once(&mut guest, sub, parse_source(sub, "guest", &GUEST_CLOCKS)?)?,
        }
    }
    Ok(Clock { service, guest })
}

fn parse_smoke(entry: &Entry) -> syn::Result<Smoke> {
    let what = "host_pure, guest = \"<group>\" [, profile = \"<profile>\"] or none";
    let entries = entries(entry.group(what)?)?;
    let [first, rest @ ..] = entries.as_slice() else {
        return Err(expected(entry.key.span(), "smoke", what));
    };
    match (first.name().as_str(), &first.value, rest) {
        ("host_pure", Value::Flag, []) => Ok(Smoke::HostPure),
        ("none", Value::Flag, []) => Ok(Smoke::None(first.key.span())),
        ("guest", Value::Eq(_), rest) => {
            let group = first.string()?;
            let profile = match rest {
                [] => None,
                [profile] if profile.name() == "profile" => Some(profile.string()?),
                [other, ..] => return Err(expected(other.key.span(), "smoke", what)),
            };
            Ok(Smoke::Guest { group, profile })
        }
        _ => Err(expected(first.key.span(), "smoke", what)),
    }
}

impl Declaration {
    fn expand(&self, function: &ItemFn) -> TokenStream {
        let name = function.sig.ident.unraw();
        let entry = format_ident!("__OVERDRIVE_BENCH_{}", name.to_string().to_uppercase());
        let substrate = self.substrate_check();
        let smoke = self.smoke_test(&name, &entry);
        let registration = self.registration(&name, &entry, &function.sig.ident);
        quote! {
            #function
            #substrate
            #registration
            #smoke
        }
    }

    /// The constant evaluation that rejects a substrate the registry does not hold, its
    /// message built by the registry so the list of registered substrates has one home.
    fn substrate_check(&self) -> TokenStream {
        let span = self.substrate.span();
        let value = LitStr::new(&self.substrate.unraw().to_string(), span);
        quote_spanned! {span=>
            const _: () = {
                const NAME: &str = #value;
                const LENGTH: usize = ::overdrive_bench::registry::substrate_refusal_length(NAME);
                const REFUSAL: [u8; LENGTH] =
                    ::overdrive_bench::registry::substrate_refusal::<LENGTH>(NAME);
                if !::overdrive_bench::registry::substrate_registered(NAME) {
                    ::core::panic!("{}", ::overdrive_bench::registry::message(&REFUSAL));
                }
            };
        }
    }

    fn registration(&self, name: &Ident, entry: &Ident, function: &Ident) -> TokenStream {
        let family = &self.family;
        let benchmark = name.to_string();
        let substrate = self.substrate.unraw().to_string();
        let arms = &self.arms;
        let replicates = self.replicates;
        let bpf_stats = self.bpf_stats;
        let profiles = self.profiles.iter().map(|profile| {
            let (name, preparation) = (&profile.name, &profile.preparation);
            quote!(::overdrive_bench::registry::ProfileDecl { name: #name, preparation: #preparation })
        });
        let metrics = self.metrics.iter().map(|metric| {
            let unit = metric.unit.to_string();
            let (name, table, series) = (&metric.name, &metric.table, metric.series);
            quote! {
                ::overdrive_bench::registry::MetricDecl {
                    name: #name, unit: #unit, table: #table, series: #series,
                }
            }
        });
        let references = self.references.iter().map(|reference| {
            let (metric, label, source) = (&reference.metric, &reference.label, &reference.source);
            let value = proc_macro2::Literal::f64_suffixed(reference.value);
            quote! {
                ::overdrive_bench::registry::ReferenceDecl {
                    metric: #metric, value: #value, label: #label, source: #source,
                }
            }
        });
        let service_clock = self.clock.service.map_or_else(
            || quote!(::core::option::Option::None),
            |variant| {
                let variant = format_ident!("{variant}");
                quote!(::core::option::Option::Some(::overdrive_bench::registry::ServiceClock::#variant))
            },
        );
        let guest_clock = self.clock.guest.map_or_else(
            || quote!(::core::option::Option::None),
            |variant| {
                let variant = format_ident!("{variant}");
                quote!(::core::option::Option::Some(::overdrive_bench::registry::GuestClock::#variant))
            },
        );
        quote! {
            #[::overdrive_bench::registry::linkme::distributed_slice(::overdrive_bench::registry::BENCHMARKS)]
            #[linkme(crate = ::overdrive_bench::registry::linkme)]
            static #entry: ::overdrive_bench::BenchmarkDecl = ::overdrive_bench::BenchmarkDecl {
                family: #family,
                benchmark: #benchmark,
                substrate: #substrate,
                arms: &[#(#arms),*],
                replicates: #replicates,
                profiles: &[#(#profiles),*],
                metrics: &[#(#metrics),*],
                bpf_stats: #bpf_stats,
                references: &[#(#references),*],
                service_clock: #service_clock,
                guest_clock: #guest_clock,
                run: #function,
            };
        }
    }

    /// The generated `smoke_<name>` test, with its own placement, or the build error of a
    /// `smoke(none)` outside a spike crate. A guest group is checked here as well when the
    /// test itself is not emitted (the placement attribute checks it where it is).
    fn smoke_test(&self, name: &Ident, entry: &Ident) -> TokenStream {
        let test = format_ident!("smoke_{name}");
        let placement = match &self.smoke {
            Smoke::None(span) => {
                return quote_spanned! {*span=>
                    const _: () = {
                        if !::overdrive_bench::registry::in_spike_scratch(
                            ::core::env!("CARGO_MANIFEST_DIR"),
                        ) {
                            ::core::panic!("smoke(none) is allowed only in a crate under spike-scratch/");
                        }
                    };
                };
            }
            Smoke::HostPure => quote!(host_pure),
            Smoke::Guest { group, profile } => {
                let checked = profile.clone().unwrap_or_else(|| LitStr::new("", group.span()));
                let group_check = known_group_check(group);
                let profile_check = known_profile_check(group, &checked);
                let profile_argument = profile.as_ref().map(|profile| quote!(, profile = #profile));
                return quote! {
                    #[cfg(not(feature = "integration-tests"))]
                    #group_check
                    #[cfg(not(feature = "integration-tests"))]
                    #profile_check
                    #[cfg(feature = "integration-tests")]
                    #[::overdrive_testing::overdrive_test(guest = #group #profile_argument)]
                    #[test]
                    fn #test() {
                        ::overdrive_bench::smoke(&#entry);
                    }
                };
            }
        };
        quote! {
            #[cfg(feature = "integration-tests")]
            #[::overdrive_testing::overdrive_test(#placement)]
            #[test]
            fn #test() {
                ::overdrive_bench::smoke(&#entry);
            }
        }
    }
}
