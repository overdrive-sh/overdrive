//! S-ND295-65 — the serve composition has no optional switch for protection,
//! DNS, or the shared-network supervisor (D-295-R16, E16; FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `ServerConfig` ports, the removed `dns_probe_fault`, and the deleted `compose_mtls`);
//! FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (how the gate reaches the shim, and the `AppState` constructors); FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (row E16)).
//!
//! A token-level scan of the production source trees that compose the serve
//! boundary: every `.rs` file under `overdrive-control-plane/src` and
//! `overdrive-worker/src`. The scan finds each declaration by its name, not by
//! the file it lives in: no file placement is part of the contract. Before any
//! check, every comment (doc comments included), string, and char literal is
//! blanked, every item gated by exactly `#[cfg(test)]` is removed, and the file
//! of an out-of-line module declared under `#[cfg(test)]` is not scanned. An
//! item gated by `#[cfg(any(test, feature = "integration-tests"))]` is
//! production-compiled under that feature and is scanned.
//!
//! The scan reads declarations, not prose:
//!
//! - **No optional port in a composition declaration.** E16 scopes the rule to
//!   the fields and parameters that compose protection, DNS, the shared guest
//!   network, or the supervisor (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (row E16)): the fields
//!   of `AppState`, `ServerHandle`, and `ServerConfig`; the parameters of
//!   `AppState::new` and `AppState::new_with_workflow_engine`; the parameters of
//!   every `run_server*` function; and every parameter that carries the
//!   `MtlsInterceptLifecycle` port (the action-shim lifecycle parameters). None
//!   of them has a type in which `Option<…>` wraps a required serve port or
//!   owner (`MtlsInterceptWorker`, `ServiceBackendsResolve`,
//!   `SharedGuestNetworkOwner`, `MtlsInterceptLifecycle`, `MtlsIntercept`,
//!   `GuestDns`, `GuestDnsFactory`, `DnsServeTaskOwner`,
//!   `SharedNetworkSupervisorHandle`, or an alias of one), or names an alias
//!   that does. A local binding is not scanned.
//! - **A lifecycle slot is not a composition switch.** The DNS task owner's
//!   `responder` slot is `None` between a task's end and its replacement (FD
//!   § "D-295-DISTILL-8", `DnsServeTaskOwner`); E16 does not scope it. The scan
//!   records it as a positive row: the slot exists in the scanned sources and
//!   wraps `GuestDns` in `Option`, and it is not reported.
//! - **The required declarations exist, unwrapped.** `AppState.{mtls_worker,
//!   shared_guest_network, guest_network_exec, guest_pool}` and the same
//!   parameters of `AppState::new` and `AppState::new_with_workflow_engine`
//!   (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` fields and constructors)); `ServerHandle.{mtls_worker_owner, mtls_resolve_owner}` and
//!   its `SharedNetworkSupervisorHandle` field (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (`AppState` constructors: `ServerHandle` owner fields));
//!   `ServerConfig.{mtls_intercept, guest_dns}` (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `ServerConfig` port fields)); at least one
//!   parameter of the `MtlsInterceptLifecycle` port (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (no activation path without intercept-live)).
//! - **No switch and no after-boot replacement.** None of the identifiers
//!   `compose_mtls`, `dns_probe_fault`, `replace_mtls_worker_for_test`,
//!   `inject_owner_shutdown_failure_for_test`, or `owner_shutdown_failures`
//!   occurs in code (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the removed `dns_probe_fault` and the deleted `compose_mtls`); FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (`AppState` constructors: the deleted test-only replacement hooks)); no `ServerHandle` method assigns
//!   one of its owner fields; and no `run_server*` body assigns a required
//!   `AppState` or `ServerHandle` owner field after construction (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (`AppState` constructors: composition roots)).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The production source trees scanned, relative to this crate's manifest
/// directory. Declarations are found by name anywhere in them.
const SOURCE_ROOTS: [&str; 2] = ["src", "../overdrive-worker/src"];

/// The required serve ports and owners no composition declaration may wrap in
/// `Option`.
const REQUIRED_PORTS: [&str; 9] = [
    "MtlsInterceptWorker",
    "ServiceBackendsResolve",
    "SharedGuestNetworkOwner",
    "MtlsInterceptLifecycle",
    "MtlsIntercept",
    "GuestDns",
    "GuestDnsFactory",
    "DnsServeTaskOwner",
    "SharedNetworkSupervisorHandle",
];

/// The structs whose fields are composition declarations (E16).
const COMPOSITION_STRUCTS: [&str; 3] = ["AppState", "ServerHandle", "ServerConfig"];

/// The `AppState` constructors whose parameters are composition declarations.
const APP_STATE_CONSTRUCTORS: [&str; 2] = ["new", "new_with_workflow_engine"];

/// The port whose parameters are the action-shim lifecycle parameters.
const LIFECYCLE_PORT: &str = "MtlsInterceptLifecycle";

/// Lifecycle slots E16 does not scope: task-owner state that is `None` between
/// a task's end and its replacement, not a composition switch.
const EXEMPT_LIFECYCLE_SLOTS: [&str; 1] = ["DnsServeTaskOwner.responder"];

/// Identifiers whose presence in code is itself an optional switch or an
/// after-boot replacement hook.
const BANNED_IDENTIFIERS: [&str; 5] = [
    "compose_mtls",
    "dns_probe_fault",
    "replace_mtls_worker_for_test",
    "inject_owner_shutdown_failure_for_test",
    "owner_shutdown_failures",
];

/// Required `AppState` fields and constructor parameters, with the type each
/// must name (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` fields and constructors)).
const APP_STATE_OWNERS: [(&str, &str); 4] = [
    ("mtls_worker", "MtlsInterceptWorker"),
    ("shared_guest_network", "SharedGuestNetworkOwner"),
    ("guest_network_exec", "GuestNetworkExecGate"),
    ("guest_pool", "GuestAddressPool"),
];

/// Required `ServerHandle` owner fields (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (`AppState` constructors: `ServerHandle` owner fields)).
const SERVER_HANDLE_OWNERS: [(&str, &str); 2] = [
    ("mtls_worker_owner", "MtlsInterceptWorker"),
    ("mtls_resolve_owner", "ServiceBackendsResolve"),
];

/// Required `ServerConfig` port fields (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `ServerConfig` port fields)).
const SERVER_CONFIG_PORTS: [(&str, &str); 2] =
    [("mtls_intercept", "MtlsIntercept"), ("guest_dns", "GuestDnsFactory")];

// ---------------------------------------------------------------------------
// Lexing.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Punct(String),
    Literal,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: usize,
}

impl Token {
    fn is_ident(&self, name: &str) -> bool {
        matches!(&self.tok, Tok::Ident(ident) if ident == name)
    }

    fn is_punct(&self, punct: &str) -> bool {
        matches!(&self.tok, Tok::Punct(p) if p == punct)
    }

    fn ident(&self) -> Option<&str> {
        match &self.tok {
            Tok::Ident(ident) => Some(ident),
            _ => None,
        }
    }
}

const fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Push `c` as a blank, keeping line structure.
fn push_blank(out: &mut String, c: char) {
    out.push(if c == '\n' { '\n' } else { ' ' });
}

/// Blank the literal `chars[start..end]` into one `0` followed by blanks.
fn push_literal(out: &mut String, chars: &[char], start: usize, end: usize) {
    out.push('0');
    for &c in &chars[start + 1..end] {
        push_blank(out, c);
    }
}

/// End (exclusive) of the quoted literal whose opening `"` is at `open`.
fn quoted_end(chars: &[char], open: usize) -> usize {
    let mut i = open + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// End (exclusive) of a raw string whose `r` is at `r_at`, or `None` when
/// `r_at` does not start one (a raw identifier such as `r#type`).
fn raw_string_end(chars: &[char], r_at: usize) -> Option<usize> {
    let mut i = r_at + 1;
    let mut hashes = 0;
    while chars.get(i) == Some(&'#') {
        hashes += 1;
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    while i < chars.len() {
        if chars[i] == '"' && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#')) {
            return Some(i + 1 + hashes);
        }
        i += 1;
    }
    Some(chars.len())
}

/// End (exclusive) of a char literal whose `'` is at `open`, or `None` when
/// the quote starts a lifetime or label.
fn char_literal_end(chars: &[char], open: usize) -> Option<usize> {
    if chars.get(open + 1) == Some(&'\\') {
        let mut i = open + 3;
        while i < chars.len() && chars[i] != '\'' {
            i += 1;
        }
        return Some(i + 1);
    }
    (chars.get(open + 2) == Some(&'\'')).then_some(open + 3)
}

/// The source with every comment and literal blanked; line numbers are kept.
fn blank_comments_and_literals(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let after_ident = i > 0 && is_ident_char(chars[i - 1]);
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0_usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    push_blank(&mut out, chars[i]);
                    i += 1;
                }
            }
        } else if c == '"' {
            let end = quoted_end(&chars, i);
            push_literal(&mut out, &chars, i, end);
            i = end;
        } else if !after_ident && c == 'b' && next == Some('"') {
            let end = quoted_end(&chars, i + 1);
            push_literal(&mut out, &chars, i, end);
            i = end;
        } else if !after_ident && c == 'b' && next == Some('\'') {
            let end = char_literal_end(&chars, i + 1).unwrap_or(i + 2);
            push_literal(&mut out, &chars, i, end);
            i = end;
        } else if let Some(end) =
            (!after_ident && c == 'r').then(|| raw_string_end(&chars, i)).flatten().or_else(|| {
                (!after_ident && c == 'b' && next == Some('r'))
                    .then(|| raw_string_end(&chars, i + 1))
                    .flatten()
            })
        {
            push_literal(&mut out, &chars, i, end);
            i = end;
        } else if c == '\'' {
            if let Some(end) = char_literal_end(&chars, i) {
                push_literal(&mut out, &chars, i, end);
                i = end;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

const MULTI_CHAR_PUNCT: [&str; 20] = [
    "..=", "...", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=",
    "%=", "^=", "&=", "|=", "..",
];

fn lex(blanked: &str) -> Vec<Token> {
    let chars: Vec<char> = blanked.chars().collect();
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let mut ident: String = chars[start..i].iter().collect();
            // A raw identifier `r#name` is the identifier `name`.
            if ident == "r"
                && chars.get(i) == Some(&'#')
                && chars.get(i + 1).is_some_and(|&c| is_ident_char(c))
            {
                let raw_start = i + 1;
                i = raw_start;
                while i < chars.len() && is_ident_char(chars[i]) {
                    i += 1;
                }
                ident = chars[raw_start..i].iter().collect();
            }
            tokens.push(Token { tok: Tok::Ident(ident), line });
        } else if c.is_ascii_digit() {
            while i < chars.len()
                && (is_ident_char(chars[i])
                    || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            tokens.push(Token { tok: Tok::Literal, line });
        } else {
            let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
            let punct = MULTI_CHAR_PUNCT
                .iter()
                .find(|p| rest.starts_with(**p))
                .map_or_else(|| c.to_string(), |p| (*p).to_owned());
            i += punct.chars().count();
            tokens.push(Token { tok: Tok::Punct(punct), line });
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// Structure.
// ---------------------------------------------------------------------------

fn opens(token: &Token) -> bool {
    token.is_punct("(") || token.is_punct("[") || token.is_punct("{")
}

fn closes(token: &Token) -> bool {
    token.is_punct(")") || token.is_punct("]") || token.is_punct("}")
}

/// Index of the token closing the bracket opened at `open` (`(`, `[`, `{`).
fn matching_close(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0_usize;
    for (k, token) in tokens.iter().enumerate().skip(open) {
        if opens(token) {
            depth += 1;
        } else if closes(token) {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    tokens.len()
}

/// Index just past the generic list opened by `<` at `open`.
fn generics_end(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0_usize;
    for (k, token) in tokens.iter().enumerate().skip(open) {
        if token.is_punct("<") {
            depth += 1;
        } else if token.is_punct(">") {
            depth -= 1;
            if depth == 0 {
                return k + 1;
            }
        }
    }
    tokens.len()
}

/// Whether an `#[cfg(test)]` attribute starts at `i`.
fn is_cfg_test_attr(tokens: &[Token], i: usize) -> bool {
    const SHAPE: [&str; 7] = ["#", "[", "cfg", "(", "test", ")", "]"];
    SHAPE.iter().enumerate().all(|(k, part)| {
        tokens.get(i + k).is_some_and(|token| token.is_punct(part) || token.is_ident(part))
    })
}

/// Index just past the outer attributes starting at `i`.
fn skip_attributes(tokens: &[Token], mut i: usize) -> usize {
    while tokens.get(i).is_some_and(|t| t.is_punct("#"))
        && tokens.get(i + 1).is_some_and(|t| t.is_punct("["))
    {
        i = matching_close(tokens, i + 1) + 1;
    }
    i
}

/// Index just past a visibility qualifier starting at `i`.
fn skip_visibility(tokens: &[Token], i: usize) -> usize {
    if !tokens.get(i).is_some_and(|t| t.is_ident("pub")) {
        return i;
    }
    if tokens.get(i + 1).is_some_and(|t| t.is_punct("(")) {
        return matching_close(tokens, i + 1) + 1;
    }
    i + 1
}

const ITEM_KEYWORDS: [&str; 11] = [
    "fn",
    "mod",
    "impl",
    "struct",
    "enum",
    "trait",
    "union",
    "async",
    "unsafe",
    "extern",
    "macro_rules",
];

/// Index just past the item, field, or statement a `#[cfg(test)]` at `start`
/// gates.
fn gated_item_end(tokens: &[Token], start: usize) -> usize {
    let body = skip_visibility(tokens, skip_attributes(tokens, start + 7));
    let is_item = tokens.get(body).is_some_and(|t| ITEM_KEYWORDS.iter().any(|kw| t.is_ident(kw)));
    let mut depth = 0_usize;
    let mut angles = 0_usize;
    let mut k = body;
    while k < tokens.len() {
        let token = &tokens[k];
        if token.is_punct("{") && depth == 0 {
            return matching_close(tokens, k) + 1;
        } else if opens(token) {
            depth += 1;
        } else if closes(token) {
            if depth == 0 {
                // The enclosing list closes: the gated element ended.
                return k;
            }
            depth -= 1;
        } else if depth == 0 && token.is_punct("<") {
            angles += 1;
        } else if depth == 0 && token.is_punct(">") && angles > 0 {
            angles -= 1;
        } else if depth == 0
            && angles == 0
            && (token.is_punct(";") || (!is_item && token.is_punct(",")))
        {
            // A `;` ends any gated element; a `,` ends a gated field, element,
            // or arm, never an item (whose header commas are generic or
            // `where`-clause separators).
            return k + 1;
        }
        k += 1;
    }
    tokens.len()
}

/// The token stream with every `#[cfg(test)]`-gated item removed.
fn without_cfg_test_items(tokens: &[Token]) -> Vec<Token> {
    let mut kept = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if is_cfg_test_attr(tokens, i) {
            i = gated_item_end(tokens, i);
        } else {
            kept.push(tokens[i].clone());
            i += 1;
        }
    }
    kept
}

/// `tokens` split at commas that are not nested in brackets or generics.
fn split_top_level(tokens: &[Token]) -> Vec<&[Token]> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut angles = 0_usize;
    let mut start = 0;
    for (k, token) in tokens.iter().enumerate() {
        if opens(token) {
            depth += 1;
        } else if closes(token) {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && token.is_punct("<") {
            angles += 1;
        } else if depth == 0 && token.is_punct(">") {
            angles = angles.saturating_sub(1);
        } else if token.is_punct(",") && depth == 0 && angles == 0 {
            parts.push(&tokens[start..k]);
            start = k + 1;
        }
    }
    if start < tokens.len() {
        parts.push(&tokens[start..]);
    }
    parts
}

/// One declared type: a field, a parameter, or a type alias.
struct Declaration {
    /// `Owner.field`, `fn_name(param)`, or `type Alias`.
    site: String,
    name: String,
    ty: Vec<Token>,
    line: usize,
}

/// `name: Type` from a field chunk, after attributes and visibility.
fn named_field(chunk: &[Token]) -> Option<(String, Vec<Token>, usize)> {
    let at = skip_visibility(chunk, skip_attributes(chunk, 0));
    let name = chunk.get(at)?.ident()?.to_owned();
    chunk.get(at + 1).filter(|t| t.is_punct(":"))?;
    Some((name, chunk[at + 2..].to_vec(), chunk[at].line))
}

/// The fields of `struct`/`enum` bodies, with the owner's name.
fn field_declarations(tokens: &[Token]) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let is_struct = token.is_ident("struct") || token.is_ident("union");
        let is_enum = token.is_ident("enum");
        if !(is_struct || is_enum) {
            continue;
        }
        let Some(owner) = tokens.get(i + 1).and_then(Token::ident) else {
            continue;
        };
        let mut j = i + 2;
        if tokens.get(j).is_some_and(|t| t.is_punct("<")) {
            j = generics_end(tokens, j);
        }
        if is_struct && tokens.get(j).is_some_and(|t| t.is_punct("(")) {
            let close = matching_close(tokens, j);
            for (index, chunk) in split_top_level(&tokens[j + 1..close]).into_iter().enumerate() {
                let at = skip_visibility(chunk, skip_attributes(chunk, 0));
                if let Some(first) = chunk.get(at) {
                    declarations.push(Declaration {
                        site: format!("{owner}.{index}"),
                        name: index.to_string(),
                        ty: chunk[at..].to_vec(),
                        line: first.line,
                    });
                }
            }
            continue;
        }
        let Some(open) = (j..tokens.len())
            .find(|&k| {
                tokens[k].is_punct("{") || tokens[k].is_punct(";") || tokens[k].is_punct("(")
            })
            .filter(|&k| tokens[k].is_punct("{"))
        else {
            continue;
        };
        let close = matching_close(tokens, open);
        for chunk in split_top_level(&tokens[open + 1..close]) {
            if is_struct {
                if let Some((name, ty, line)) = named_field(chunk) {
                    declarations.push(Declaration {
                        site: format!("{owner}.{name}"),
                        name,
                        ty,
                        line,
                    });
                }
                continue;
            }
            // An enum variant: `Variant { fields }` or `Variant(types)`.
            let at = skip_attributes(chunk, 0);
            let Some(variant) = chunk.get(at).and_then(Token::ident) else {
                continue;
            };
            let Some(body) = chunk.get(at + 1).filter(|t| t.is_punct("{") || t.is_punct("("))
            else {
                continue;
            };
            let inner_close = matching_close(chunk, at + 1);
            for (index, field) in
                split_top_level(&chunk[at + 2..inner_close]).into_iter().enumerate()
            {
                if body.is_punct("{") {
                    if let Some((name, ty, line)) = named_field(field) {
                        declarations.push(Declaration {
                            site: format!("{owner}::{variant}.{name}"),
                            name,
                            ty,
                            line,
                        });
                    }
                } else if let Some(first) = field.first() {
                    declarations.push(Declaration {
                        site: format!("{owner}::{variant}.{index}"),
                        name: index.to_string(),
                        ty: field.to_vec(),
                        line: first.line,
                    });
                }
            }
        }
    }
    declarations
}

/// One function signature, with its body's token range when it has one.
struct FnDecl {
    name: String,
    params: Vec<Declaration>,
    body: Option<(usize, usize)>,
}

fn function_declarations(tokens: &[Token]) -> Vec<FnDecl> {
    let mut functions = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if !token.is_ident("fn") {
            continue;
        }
        let Some(name) = tokens.get(i + 1).and_then(Token::ident) else {
            continue;
        };
        let mut j = i + 2;
        if tokens.get(j).is_some_and(|t| t.is_punct("<")) {
            j = generics_end(tokens, j);
        }
        if !tokens.get(j).is_some_and(|t| t.is_punct("(")) {
            continue;
        }
        let close = matching_close(tokens, j);
        let mut params = Vec::new();
        for chunk in split_top_level(&tokens[j + 1..close]) {
            let chunk = &chunk[skip_attributes(chunk, 0)..];
            let Some(colon) = chunk.iter().position(|t| t.is_punct(":")) else {
                continue; // `self`, `&self`, `&mut self`, `mut self`
            };
            let Some(binding) = chunk[..colon]
                .iter()
                .filter_map(Token::ident)
                .find(|ident| *ident != "mut" && *ident != "ref")
            else {
                continue;
            };
            if binding == "self" {
                continue;
            }
            params.push(Declaration {
                site: format!("{name}({binding})"),
                name: binding.to_owned(),
                ty: chunk[colon + 1..].to_vec(),
                line: chunk[colon].line,
            });
        }
        let mut depth = 0_usize;
        let mut body = None;
        for k in close + 1..tokens.len() {
            let t = &tokens[k];
            if depth == 0 && t.is_punct("{") {
                body = Some((k, matching_close(tokens, k)));
                break;
            } else if depth == 0 && t.is_punct(";") {
                break;
            } else if t.is_punct("(") || t.is_punct("[") {
                depth += 1;
            } else if t.is_punct(")") || t.is_punct("]") {
                depth = depth.saturating_sub(1);
            }
        }
        functions.push(FnDecl { name: name.to_owned(), params, body });
    }
    functions
}

/// `type Alias = Type;` declarations (associated type declarations without a
/// right-hand side are not aliases).
fn alias_declarations(tokens: &[Token]) -> Vec<Declaration> {
    let mut aliases = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if !token.is_ident("type") {
            continue;
        }
        let Some(name) = tokens.get(i + 1).and_then(Token::ident) else {
            continue;
        };
        let mut j = i + 2;
        if tokens.get(j).is_some_and(|t| t.is_punct("<")) {
            j = generics_end(tokens, j);
        }
        if !tokens.get(j).is_some_and(|t| t.is_punct("=")) {
            continue;
        }
        let end = (j..tokens.len()).find(|&k| tokens[k].is_punct(";")).unwrap_or(tokens.len());
        aliases.push(Declaration {
            site: format!("type {name}"),
            name: name.to_owned(),
            ty: tokens[j + 1..end].to_vec(),
            line: token.line,
        });
    }
    aliases
}

/// The `impl` blocks whose self type is `self_type`, as token ranges.
fn impl_blocks(tokens: &[Token], self_type: &str) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if !token.is_ident("impl") {
            continue;
        }
        let mut j = i + 1;
        if tokens.get(j).is_some_and(|t| t.is_punct("<")) {
            j = generics_end(tokens, j);
        }
        let Some(open) = (j..tokens.len()).find(|&k| tokens[k].is_punct("{")) else {
            continue;
        };
        let header = &tokens[j..open];
        let header = header
            .iter()
            .position(|t| t.is_ident("where"))
            .map_or(header, |where_at| &header[..where_at]);
        let target = header
            .iter()
            .position(|t| t.is_ident("for"))
            .map_or(header, |for_at| &header[for_at + 1..]);
        let target = target
            .iter()
            .position(|t| t.is_punct("<"))
            .map_or(target, |generic_at| &target[..generic_at]);
        if target.iter().filter_map(Token::ident).next_back() == Some(self_type) {
            blocks.push((open, matching_close(tokens, open)));
        }
    }
    blocks
}

/// The first watched identifier a type wraps in `Option<…>`, if any.
fn option_wrapped<'a>(ty: &'a [Token], watched: &BTreeSet<String>) -> Option<&'a str> {
    ty.iter().enumerate().find_map(|(k, token)| {
        if !token.is_ident("Option") {
            return None;
        }
        let open = if ty.get(k + 1).is_some_and(|t| t.is_punct("<")) {
            k + 1
        } else if ty.get(k + 1).is_some_and(|t| t.is_punct("::"))
            && ty.get(k + 2).is_some_and(|t| t.is_punct("<"))
        {
            k + 2
        } else {
            return None;
        };
        ty[open..generics_end(ty, open).min(ty.len())]
            .iter()
            .filter_map(Token::ident)
            .find(|ident| watched.contains(*ident))
    })
}

fn names(ty: &[Token], ident: &str) -> bool {
    ty.iter().any(|t| t.is_ident(ident))
}

fn render(ty: &[Token]) -> String {
    ty.iter()
        .map(|t| match &t.tok {
            Tok::Ident(s) | Tok::Punct(s) => s.as_str(),
            Tok::Literal => "_",
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// The scan.
// ---------------------------------------------------------------------------

struct ScannedFile {
    path: String,
    tokens: Vec<Token>,
    fields: Vec<Declaration>,
    functions: Vec<FnDecl>,
    aliases: Vec<Declaration>,
}

/// Scan one source text, labelled `path` in every report.
fn scan_source(path: String, source: &str) -> ScannedFile {
    let tokens = without_cfg_test_items(&lex(&blank_comments_and_literals(source)));
    ScannedFile {
        path,
        fields: field_declarations(&tokens),
        functions: function_declarations(&tokens),
        aliases: alias_declarations(&tokens),
        tokens,
    }
}

/// Every `.rs` file under `root`, recursively.
fn rust_files(root: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read directory {}: {error}", directory.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!("read an entry of {}: {error}", directory.display())
            });
            let path = entry.path();
            let file_type = entry
                .file_type()
                .unwrap_or_else(|error| panic!("read the type of {}: {error}", path.display()));
            if file_type.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.insert(path);
            }
        }
    }
    files
}

/// The directory in which `file`'s out-of-line child modules live.
fn module_directory(file: &Path) -> PathBuf {
    let parent =
        file.parent().unwrap_or_else(|| panic!("scanned file {} has a parent", file.display()));
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_else(|| panic!("scanned file {} has a UTF-8 stem", file.display()));
    match stem {
        "lib" | "main" | "mod" => parent.to_path_buf(),
        stem => parent.join(stem),
    }
}

/// The candidate files of every out-of-line module `file` declares under
/// `#[cfg(test)]` (`#[cfg(test)] mod name;`).
fn cfg_test_module_files(file: &Path, tokens: &[Token]) -> Vec<PathBuf> {
    let directory = module_directory(file);
    let mut files = Vec::new();
    for i in 0..tokens.len() {
        if !is_cfg_test_attr(tokens, i) {
            continue;
        }
        let at = skip_visibility(tokens, skip_attributes(tokens, i + 7));
        if tokens.get(at).is_some_and(|t| t.is_ident("mod"))
            && let Some(name) = tokens.get(at + 1).and_then(Token::ident)
            && tokens.get(at + 2).is_some_and(|t| t.is_punct(";"))
        {
            files.push(directory.join(format!("{name}.rs")));
            files.push(directory.join(name).join("mod.rs"));
        }
    }
    files
}

/// Every production source file of the scanned trees: each `.rs` file except
/// the file of an out-of-line module declared under `#[cfg(test)]`.
fn production_files() -> Vec<ScannedFile> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = BTreeMap::new();
    for root in SOURCE_ROOTS {
        for path in rust_files(&manifest.join(root)) {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read scanned file {}: {error}", path.display()));
            sources.insert(path, source);
        }
    }
    let test_only: BTreeSet<PathBuf> = sources
        .iter()
        .flat_map(|(path, source)| {
            cfg_test_module_files(path, &lex(&blank_comments_and_literals(source)))
        })
        .collect();
    sources
        .into_iter()
        .filter(|(path, _)| !test_only.contains(path))
        .map(|(path, source)| {
            let label = path.strip_prefix(manifest).unwrap_or(&path).display().to_string();
            scan_source(label, &source)
        })
        .collect()
}

/// One composition declaration (E16) and the file it is in.
struct CompositionDeclaration<'a> {
    file: &'a str,
    site: String,
    declaration: &'a Declaration,
}

/// The composition declarations of `file`: the fields of the composition
/// structs, the `AppState` constructor parameters, the `run_server*`
/// parameters, and every parameter of the lifecycle port.
fn composition_declarations(file: &ScannedFile) -> Vec<CompositionDeclaration<'_>> {
    let mut declarations = Vec::new();
    for field in &file.fields {
        let owner = field.site.split_once('.').map_or("", |(owner, _)| owner);
        if COMPOSITION_STRUCTS.contains(&owner) {
            declarations.push(CompositionDeclaration {
                file: &file.path,
                site: field.site.clone(),
                declaration: field,
            });
        }
    }
    let app_state_impls = impl_blocks(&file.tokens, "AppState");
    for function in &file.functions {
        let is_constructor = APP_STATE_CONSTRUCTORS.contains(&function.name.as_str())
            && function.body.is_some_and(|(open, _)| {
                app_state_impls.iter().any(|(start, end)| *start < open && open < *end)
            });
        for parameter in &function.params {
            let site = if is_constructor {
                Some(format!("AppState::{}", parameter.site))
            } else if function.name.starts_with("run_server")
                || names(&parameter.ty, LIFECYCLE_PORT)
            {
                Some(parameter.site.clone())
            } else {
                None
            };
            if let Some(site) = site {
                declarations.push(CompositionDeclaration {
                    file: &file.path,
                    site,
                    declaration: parameter,
                });
            }
        }
    }
    declarations
}

/// What the optional-port rule found.
struct PortScan {
    /// One entry per composition declaration that wraps a required port.
    violations: Vec<String>,
    /// The exempt lifecycle slots found wrapping a required port, unreported.
    exempt_seen: Vec<String>,
    /// How many composition declarations were checked.
    scanned: usize,
}

/// The optional-port rule over `files`: every composition declaration is
/// checked; an exempt lifecycle slot is recorded, never reported.
fn optional_composition_ports(files: &[ScannedFile]) -> PortScan {
    // Aliases of a required port are themselves required ports.
    let mut watched: BTreeSet<String> = REQUIRED_PORTS.iter().map(|p| (*p).to_owned()).collect();
    loop {
        let before = watched.len();
        for alias in files.iter().flat_map(|f| &f.aliases) {
            if alias.ty.iter().filter_map(Token::ident).any(|i| watched.contains(i)) {
                watched.insert(alias.name.clone());
            }
        }
        if watched.len() == before {
            break;
        }
    }
    // An alias that wraps a required port in `Option`, directly or through
    // another such alias, is an optional port.
    let mut optional_aliases: BTreeSet<String> = BTreeSet::new();
    loop {
        let before = optional_aliases.len();
        for alias in files.iter().flat_map(|f| &f.aliases) {
            if option_wrapped(&alias.ty, &watched).is_some()
                || alias.ty.iter().filter_map(Token::ident).any(|i| optional_aliases.contains(i))
            {
                optional_aliases.insert(alias.name.clone());
            }
        }
        if optional_aliases.len() == before {
            break;
        }
    }
    let optional_port = |ty: &[Token]| -> Option<String> {
        option_wrapped(ty, &watched).map(str::to_owned).or_else(|| {
            ty.iter()
                .filter_map(Token::ident)
                .find(|ident| optional_aliases.contains(*ident))
                .map(|alias| format!("{alias}, an optional alias"))
        })
    };

    let mut scan = PortScan { violations: Vec::new(), exempt_seen: Vec::new(), scanned: 0 };
    for file in files {
        for composition in composition_declarations(file) {
            scan.scanned += 1;
            if let Some(port) = optional_port(&composition.declaration.ty) {
                scan.violations.push(format!(
                    "{}:{}: {} is `{}`, an optional `{port}`",
                    composition.file,
                    composition.declaration.line,
                    composition.site,
                    render(&composition.declaration.ty)
                ));
            }
        }
        for field in
            file.fields.iter().filter(|f| EXEMPT_LIFECYCLE_SLOTS.contains(&f.site.as_str()))
        {
            if optional_port(&field.ty).is_some() {
                scan.exempt_seen.push(format!("{}:{}: {}", file.path, field.line, field.site));
            }
        }
    }
    scan
}

/// Required declarations: present, naming `ty`, and not optional.
fn check_required(
    violations: &mut Vec<String>,
    site: &str,
    declared: &[(&str, &Declaration)],
    required: &[(&str, &str)],
) {
    for (name, ty) in required {
        match declared.iter().find(|(_, d)| d.name == *name) {
            None => violations.push(format!("{site} declares no `{name}`")),
            Some((file, d)) if !names(&d.ty, ty) || names(&d.ty, "Option") => {
                violations.push(format!(
                    "{file}:{}: {} is `{}`, not a required `{ty}`",
                    d.line,
                    d.site,
                    render(&d.ty)
                ));
            }
            Some(_) => {}
        }
    }
}

/// Every place where a scanned token window assigns `.field` for one of
/// `fields` (`.field =`).
fn field_assignments(tokens: &[Token], fields: &BTreeSet<&str>) -> Vec<(usize, String)> {
    tokens
        .windows(3)
        .filter_map(|w| {
            let field = w[1].ident()?;
            (w[0].is_punct(".") && fields.contains(field) && w[2].is_punct("="))
                .then(|| (w[1].line, field.to_owned()))
        })
        .collect()
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-65 — Protection, DNS, and the supervisor are always composed
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 05-01 (S-ND295-65)"]
#[allow(clippy::too_many_lines, reason = "one fail-closed universe report for the whole scan")]
fn no_optional_switch_gates_protection_dns_or_supervisor_composition() {
    let files = production_files();
    let mut violations = Vec::new();

    // No optional port in a composition declaration; the exempt lifecycle
    // slot is a positive row, seen and not reported.
    let ports = optional_composition_ports(&files);
    violations.extend(ports.violations.iter().cloned());
    if ports.exempt_seen.is_empty() {
        violations.push(format!(
            "no exempt lifecycle slot ({EXEMPT_LIFECYCLE_SLOTS:?}) wrapping a required port was \
             found, so the exemption row observed nothing"
        ));
    }

    for file in &files {
        for token in &file.tokens {
            if let Some(ident) = token.ident().filter(|i| BANNED_IDENTIFIERS.contains(i)) {
                violations.push(format!("{}:{}: `{ident}` is present", file.path, token.line));
            }
        }
    }

    // The required declarations exist, unwrapped, wherever they are declared.
    let fields_of = |owner: &str| -> Vec<(&str, &Declaration)> {
        files
            .iter()
            .flat_map(|file| {
                file.fields
                    .iter()
                    .filter(move |d| d.site.starts_with(&format!("{owner}.")))
                    .map(move |d| (file.path.as_str(), d))
            })
            .collect()
    };
    check_required(&mut violations, "AppState", &fields_of("AppState"), &APP_STATE_OWNERS);
    check_required(
        &mut violations,
        "ServerHandle",
        &fields_of("ServerHandle"),
        &SERVER_HANDLE_OWNERS,
    );
    check_required(
        &mut violations,
        "ServerConfig",
        &fields_of("ServerConfig"),
        &SERVER_CONFIG_PORTS,
    );
    let supervisor_field = fields_of("ServerHandle")
        .into_iter()
        .find(|(_, d)| names(&d.ty, "SharedNetworkSupervisorHandle"))
        .map(|(_, d)| d.name.clone());
    if supervisor_field.is_none() {
        violations.push("ServerHandle retains no `SharedNetworkSupervisorHandle` field".to_owned());
    }
    for constructor in APP_STATE_CONSTRUCTORS {
        let declared: Vec<(&str, &FnDecl)> = files
            .iter()
            .flat_map(|file| {
                let app_state_impls = impl_blocks(&file.tokens, "AppState");
                file.functions
                    .iter()
                    .filter(move |f| {
                        f.name == constructor
                            && f.body.is_some_and(|(open, _)| {
                                app_state_impls
                                    .iter()
                                    .any(|(start, end)| *start < open && open < *end)
                            })
                    })
                    .map(move |f| (file.path.as_str(), f))
            })
            .collect();
        match declared.as_slice() {
            [(file, function)] => check_required(
                &mut violations,
                &format!("AppState::{constructor}"),
                &function.params.iter().map(|p| (*file, p)).collect::<Vec<_>>(),
                &APP_STATE_OWNERS,
            ),
            other => violations
                .push(format!("expected one `AppState::{constructor}`, found {}", other.len())),
        }
    }
    let lifecycle_parameters = files
        .iter()
        .flat_map(|file| file.functions.iter().flat_map(|f| &f.params))
        .filter(|p| names(&p.ty, LIFECYCLE_PORT))
        .count();
    if lifecycle_parameters == 0 {
        violations.push(format!("no parameter carries the `{LIFECYCLE_PORT}` port"));
    }

    // No after-boot replacement through a `ServerHandle` method.
    let mut owner_fields: BTreeSet<&str> =
        SERVER_HANDLE_OWNERS.iter().map(|(name, _)| *name).collect();
    if let Some(field) = supervisor_field.as_deref() {
        owner_fields.insert(field);
    }
    let mut server_handle_impls = 0;
    for file in &files {
        for (open, close) in impl_blocks(&file.tokens, "ServerHandle") {
            server_handle_impls += 1;
            let body = &file.tokens[open..close];
            for (k, token) in body.iter().enumerate() {
                let assigns = token.is_ident("self")
                    && body.get(k + 1).is_some_and(|t| t.is_punct("."))
                    && body
                        .get(k + 2)
                        .and_then(Token::ident)
                        .is_some_and(|f| owner_fields.contains(f))
                    && body.get(k + 3).is_some_and(|t| t.is_punct("="));
                let swaps = ["replace", "swap", "take"].iter().any(|op| token.is_ident(op))
                    && body.get(k + 1).is_some_and(|t| t.is_punct("("))
                    && body.get(k + 2).is_some_and(|t| t.is_punct("&"))
                    && body.get(k + 3).is_some_and(|t| t.is_ident("mut"))
                    && body.get(k + 4).is_some_and(|t| t.is_ident("self"))
                    && body.get(k + 5).is_some_and(|t| t.is_punct("."))
                    && body
                        .get(k + 6)
                        .and_then(Token::ident)
                        .is_some_and(|f| owner_fields.contains(f))
                    && body.get(k + 7).is_some_and(|t| t.is_punct(",") || t.is_punct(")"));
                if assigns || swaps {
                    violations.push(format!(
                        "{}:{}: a `ServerHandle` method replaces an owner field",
                        file.path, token.line
                    ));
                }
            }
        }
    }
    if server_handle_impls == 0 {
        violations.push("no `impl ServerHandle` block found".to_owned());
    }

    // No post-construction assignment of a required owner in `run_server*`.
    let reassignable: BTreeSet<&str> = APP_STATE_OWNERS
        .iter()
        .map(|(name, _)| *name)
        .chain(owner_fields.iter().copied())
        .collect();
    let mut run_servers = 0;
    for file in &files {
        for function in
            file.functions.iter().filter(|f| f.name.starts_with("run_server") && f.body.is_some())
        {
            run_servers += 1;
            let Some((open, close)) = function.body else { continue };
            for (line, field) in field_assignments(&file.tokens[open..close], &reassignable) {
                violations.push(format!(
                    "{}:{line}: `{}` assigns `.{field}` after construction",
                    file.path, function.name
                ));
            }
        }
    }
    if run_servers == 0 {
        violations.push("no `run_server*` function found".to_owned());
    }

    let mut report = String::new();
    for violation in &violations {
        writeln!(report, "  - {violation}").expect("writing to a String cannot fail");
    }
    assert!(
        violations.is_empty(),
        "an optional switch or after-boot replacement gates serve composition \
         ({} files, {} composition declarations, {lifecycle_parameters} lifecycle parameters, \
         {run_servers} run_server functions scanned; exempt slots seen {:?}):\n{report}",
        files.len(),
        ports.scanned,
        ports.exempt_seen
    );
}

/// A planted source that exercises every row of the optional-port rule: one
/// optional port per kind of composition declaration, and the lifecycle slot
/// and a non-composition parameter that carry the same `Option`.
const PLANTED_SOURCE: &str = r"
pub type MaybeDnsFactory = Option<Arc<dyn GuestDnsFactory>>;

pub struct AppState {
    pub mtls_worker: Option<Arc<MtlsInterceptWorker>>,
    pub shared_guest_network: Arc<dyn SharedGuestNetworkOwner>,
}

impl AppState {
    pub fn new(shared_guest_network: Option<Arc<dyn SharedGuestNetworkOwner>>) -> Self {
        build()
    }
}

pub struct ServerConfig {
    pub guest_dns: MaybeDnsFactory,
    pub mtls_intercept: Arc<dyn MtlsIntercept>,
}

pub struct ServerHandle {
    mtls_resolve_owner: Arc<ServiceBackendsResolve>,
}

pub async fn run_server_planted(worker: Option<Arc<MtlsInterceptWorker>>) {}

fn dispatch_planted(mtls_lifecycle: Option<&dyn MtlsInterceptLifecycle>) {}

struct DnsServeTaskOwner {
    responder: Option<Arc<dyn GuestDns>>,
}

fn replace_responder(replacement: Option<Arc<dyn GuestDns>>) {}
";

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-65 — Protection, DNS, and the supervisor are always composed
/// CONTRACT_SHAPE: pure-function.
///
/// The positive and negative rows of the scan's E16 scope, over a planted
/// source: an optional port is reported in every kind of composition
/// declaration (a composition-struct field, directly and through an optional
/// alias; an `AppState` constructor parameter; a `run_server*` parameter; a
/// lifecycle-port parameter), and the DNS task owner's lifecycle slot and a
/// parameter outside the composition are not, the slot being recorded as the
/// exemption's positive row.
#[test]
fn the_scan_reports_optional_ports_only_in_composition_declarations() {
    let planted = scan_source("planted.rs".to_owned(), PLANTED_SOURCE);
    let scan = optional_composition_ports(std::slice::from_ref(&planted));
    let reported: BTreeSet<String> = scan
        .violations
        .iter()
        .map(|violation| {
            violation
                .split(": ")
                .nth(1)
                .and_then(|rest| rest.split(" is `").next())
                .unwrap_or_else(|| panic!("violation {violation:?} names its site"))
                .to_owned()
        })
        .collect();
    assert_eq!(
        reported,
        BTreeSet::from(
            [
                "AppState.mtls_worker",
                "AppState::new(shared_guest_network)",
                "ServerConfig.guest_dns",
                "run_server_planted(worker)",
                "dispatch_planted(mtls_lifecycle)",
            ]
            .map(str::to_owned)
        ),
        "the optional ports reported: {:#?}",
        scan.violations
    );
    assert_eq!(
        scan.exempt_seen,
        ["planted.rs:29: DnsServeTaskOwner.responder".to_owned()],
        "the lifecycle slot is seen as the exemption's positive row and not reported"
    );
    assert_eq!(scan.scanned, 8, "every composition declaration of the planted source is checked");
}
