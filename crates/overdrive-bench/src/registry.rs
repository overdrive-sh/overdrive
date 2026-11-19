//! Link-time benchmark registration (DDD-3) and the closed substrate registry (DDD-4).
//!
//! The items here that the `#[overdrive_bench]` expansion names are public but
//! `#[doc(hidden)]`: they are the attribute's expansion surface, not an API for
//! authors. The substrate checks are `const fn`s because the attribute evaluates
//! them at build time, where a rejection is a build error naming the value.

use crate::{Error, Replicate, ReplicateFailure, Result};

pub use linkme;

/// The registered substrates (DDD-4): `native` is the exclusive Metal host with no
/// namespace and no wrapper; `guest` is a kit guest the benchmark launches directly on the
/// Metal host (ADR-0325). The registry is closed.
const SUBSTRATES: [&str; 2] = ["native", "guest"];

/// Every benchmark the linked crates declare; the macro adds one entry per declaration.
#[linkme::distributed_slice]
pub static BENCHMARKS: [BenchmarkDecl];

/// One declared benchmark: an echo of its `#[overdrive_bench(...)]` attribute and the
/// function the runner calls once per replicate.
pub struct BenchmarkDecl {
    #[doc(hidden)]
    pub family: &'static str,
    #[doc(hidden)]
    pub benchmark: &'static str,
    #[doc(hidden)]
    pub substrate: &'static str,
    #[doc(hidden)]
    pub arms: &'static [&'static str],
    #[doc(hidden)]
    pub replicates: u32,
    #[doc(hidden)]
    pub profiles: &'static [ProfileDecl],
    #[doc(hidden)]
    pub metrics: &'static [MetricDecl],
    #[doc(hidden)]
    pub bpf_stats: bool,
    #[doc(hidden)]
    pub references: &'static [ReferenceDecl],
    #[doc(hidden)]
    pub service_clock: Option<ServiceClock>,
    #[doc(hidden)]
    pub guest_clock: Option<GuestClock>,
    #[doc(hidden)]
    pub run: fn(&mut Replicate) -> Result<(), ReplicateFailure>,
}

/// The service clock source a benchmark declares (DDD-2 "Clock sources").
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceClock {
    /// Service time read on the injected `Clock::now()` (`injected_monotonic`).
    InjectedMonotonic,
}

impl ServiceClock {
    /// The string the record carries in `clock.service`.
    #[doc(hidden)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InjectedMonotonic => "Clock::now (monotonic)",
        }
    }
}

/// The guest clock source a benchmark declares (DDD-2 "Clock sources").
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestClock {
    /// A duration `overdrive-init` measured with `Instant::elapsed`, to the millisecond
    /// (`init_elapsed_ms`).
    InitElapsedMs,
    /// A duration the guest program measured with `Instant` on the guest's `CLOCK_MONOTONIC`,
    /// in whole nanoseconds (`guest_monotonic`).
    Monotonic,
}

impl GuestClock {
    /// The string the record carries in `clock.guest`.
    #[doc(hidden)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InitElapsedMs => "CLOCK_MONOTONIC (overdrive-init Instant::elapsed, 1 ms)",
            Self::Monotonic => "CLOCK_MONOTONIC (in-guest Instant, 1 ns)",
        }
    }
}

/// A declared profile.
#[doc(hidden)]
pub struct ProfileDecl {
    pub name: &'static str,
    pub preparation: &'static str,
}

/// A declared metric; `unit` is one of `ns bytes bytes_per_second qps ratio count`.
#[doc(hidden)]
pub struct MetricDecl {
    pub name: &'static str,
    pub unit: &'static str,
    pub table: &'static str,
    pub series: bool,
}

/// A declared reference value compared with a metric's A/B delta.
#[doc(hidden)]
pub struct ReferenceDecl {
    pub metric: &'static str,
    pub value: f64,
    pub label: &'static str,
    pub source: &'static str,
}

/// The one benchmark of `family` among `declared`.
///
/// A family that nothing declares is `Error::FamilyNotDeclared` (the usual cause is a bench
/// `main` that does not reference the declaring crate, so linkme dropped its entries); a
/// family with more than one benchmark is `Error::Usage` naming them (DDD-3).
pub(crate) fn select<'a>(family: &str, declared: &'a [BenchmarkDecl]) -> Result<&'a BenchmarkDecl> {
    let mut matching = declared.iter().filter(|decl| decl.family == family);
    let Some(first) = matching.next() else {
        let mut found: Vec<String> = declared.iter().map(|decl| decl.family.to_owned()).collect();
        found.sort();
        found.dedup();
        return Err(Error::FamilyNotDeclared { family: family.to_owned(), found });
    };
    let others: Vec<&str> = matching.map(|decl| decl.benchmark).collect();
    if others.is_empty() {
        return Ok(first);
    }
    let names: Vec<&str> = std::iter::once(first.benchmark).chain(others).collect();
    Err(Error::Usage {
        reason: format!("family `{family}` declares more than one benchmark: {}", names.join(", ")),
    })
}

const fn same(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut at = 0;
    while at < left.len() {
        if left[at] != right[at] {
            return false;
        }
        at += 1;
    }
    true
}

/// Whether `name` is a registered substrate.
#[doc(hidden)]
pub const fn substrate_registered(name: &str) -> bool {
    let mut at = 0;
    while at < SUBSTRATES.len() {
        if same(SUBSTRATES[at], name) {
            return true;
        }
        at += 1;
    }
    false
}

/// The registered substrates joined by `, `, as a constant byte sequence of `LENGTH` bytes.
const fn registered_list<const LENGTH: usize>() -> [u8; LENGTH] {
    let mut buffer = Buffer { bytes: [0; LENGTH], used: 0 };
    let mut at = 0;
    while at < SUBSTRATES.len() {
        if at > 0 {
            buffer.push(", ");
        }
        buffer.push(SUBSTRATES[at]);
        at += 1;
    }
    buffer.bytes
}

const fn registered_list_length() -> usize {
    let mut length = 0;
    let mut at = 0;
    while at < SUBSTRATES.len() {
        length += SUBSTRATES[at].len();
        if at > 0 {
            length += ", ".len();
        }
        at += 1;
    }
    length
}

const LIST_LENGTH: usize = registered_list_length();
const LIST: [u8; LIST_LENGTH] = registered_list::<LIST_LENGTH>();

struct Buffer<const LENGTH: usize> {
    bytes: [u8; LENGTH],
    used: usize,
}

impl<const LENGTH: usize> Buffer<LENGTH> {
    const fn push(&mut self, text: &str) {
        let text = text.as_bytes();
        let mut at = 0;
        while at < text.len() {
            self.bytes[self.used] = text[at];
            self.used += 1;
            at += 1;
        }
    }
}

/// The three pieces of the refusal of `name`: the registry's message, with the substrate named.
const fn refusal_pieces(name: &str) -> [&str; 3] {
    ["substrate `", name, "` is not registered; the registered substrates are: "]
}

/// The length of the refusal message of `name`.
#[doc(hidden)]
pub const fn substrate_refusal_length(name: &str) -> usize {
    let pieces = refusal_pieces(name);
    pieces[0].len() + pieces[1].len() + pieces[2].len() + LIST_LENGTH
}

/// The refusal message of `name` (`LENGTH` is `substrate_refusal_length(name)`).
#[doc(hidden)]
pub const fn substrate_refusal<const LENGTH: usize>(name: &str) -> [u8; LENGTH] {
    let pieces = refusal_pieces(name);
    let mut buffer = Buffer { bytes: [0; LENGTH], used: 0 };
    buffer.push(pieces[0]);
    buffer.push(pieces[1]);
    buffer.push(pieces[2]);
    let Ok(list) = core::str::from_utf8(&LIST) else {
        panic!("the registered substrates are not UTF-8")
    };
    buffer.push(list);
    buffer.bytes
}

/// A message built by `substrate_refusal`, as text.
#[doc(hidden)]
pub const fn message(bytes: &[u8]) -> &str {
    match core::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => panic!("a refusal message is not UTF-8"),
    }
}

/// Whether the manifest directory lies under `spike-scratch/`, where `smoke(none)` is allowed.
#[doc(hidden)]
pub const fn in_spike_scratch(manifest_dir: &str) -> bool {
    let (haystack, needle) = (manifest_dir.as_bytes(), b"/spike-scratch/");
    if haystack.len() < needle.len() {
        return false;
    }
    let mut start = 0;
    while start + needle.len() <= haystack.len() {
        let mut at = 0;
        while at < needle.len() && haystack[start + at] == needle[at] {
            at += 1;
        }
        if at == needle.len() {
            return true;
        }
        start += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[allow(clippy::unnecessary_wraps, reason = "the signature is the benchmark body's")]
    fn ignore(_: &mut Replicate) -> Result<(), ReplicateFailure> {
        Ok(())
    }

    fn decl(family: &'static str, benchmark: &'static str) -> BenchmarkDecl {
        BenchmarkDecl {
            family,
            benchmark,
            substrate: "native",
            arms: &["a"],
            replicates: 9,
            profiles: &[],
            metrics: &[],
            bpf_stats: false,
            references: &[],
            service_clock: None,
            guest_clock: None,
            run: ignore,
        }
    }

    /// The registry is closed to exactly `native` and `guest` (ADR-0325); every other spelling,
    /// including a different case or a trailing space, is unregistered.
    /// `CONTRACT_SHAPE`: pure-function.
    #[test]
    fn native_and_guest_are_the_registered_substrates() {
        assert!(substrate_registered("native"));
        assert!(substrate_registered("guest"), "guest must be a registered substrate");
        for other in ["guest_xyz", "", "Native", "native ", "Guest", "guest "] {
            assert!(!substrate_registered(other), "{other:?} must not be registered");
        }
    }

    /// An unregistered substrate is refused by name with the registered ones listed in
    /// registration order; there is no refusal of its own for `guest` any more.
    /// `CONTRACT_SHAPE`: pure-function.
    #[test]
    fn the_refusal_names_the_substrate_and_the_registered_ones() {
        const LENGTH: usize = substrate_refusal_length("guest_xyz");
        const BUILT: [u8; LENGTH] = substrate_refusal::<LENGTH>("guest_xyz");
        assert_eq!(
            message(&BUILT),
            "substrate `guest_xyz` is not registered; the registered substrates are: native, guest"
        );
    }

    #[test]
    fn smoke_none_is_allowed_only_under_spike_scratch() {
        assert!(in_spike_scratch("/work/repo/spike-scratch/nbp-probe"));
        assert!(!in_spike_scratch("/work/repo/crates/overdrive-bench"));
        assert!(!in_spike_scratch("/work/repo/spike-scratch"));
        assert!(!in_spike_scratch(""));
    }

    #[test]
    fn selection_returns_the_one_benchmark_of_the_family() {
        let declared = [decl("alpha", "one"), decl("beta", "two")];
        let picked = select("beta", &declared).map(|decl| decl.benchmark);
        assert!(matches!(picked, Ok("two")));
    }

    #[test]
    fn an_undeclared_family_names_the_families_found() {
        let declared = [decl("beta", "two"), decl("alpha", "one")];
        match select("gamma", &declared) {
            Err(Error::FamilyNotDeclared { family, found }) => {
                assert_eq!(family, "gamma");
                assert_eq!(found, ["alpha", "beta"]);
            }
            other => panic!("expected FamilyNotDeclared, got {:?}", other.map(|d| d.benchmark)),
        }
    }

    #[test]
    fn a_family_with_two_benchmarks_is_a_usage_refusal_naming_both() {
        let declared = [decl("alpha", "one"), decl("alpha", "two")];
        match select("alpha", &declared) {
            Err(Error::Usage { reason }) => {
                assert!(reason.contains("`alpha`") && reason.contains("one, two"), "{reason}");
            }
            other => panic!("expected Usage, got {:?}", other.map(|d| d.benchmark)),
        }
    }

    proptest! {
        #[test]
        fn selection_finds_exactly_the_requested_family(
            families in proptest::collection::btree_set("[a-z]{1,6}", 1..6),
            pick in any::<proptest::sample::Index>(),
        ) {
            let names: Vec<&'static str> =
                families.into_iter().map(|name| &*Box::leak(name.into_boxed_str())).collect();
            let declared: Vec<BenchmarkDecl> =
                names.iter().map(|name| decl(name, name)).collect();
            let wanted = names[pick.index(names.len())];
            let picked = select(wanted, &declared).map(|decl| decl.family);
            prop_assert!(matches!(picked, Ok(family) if family == wanted));
        }
    }
}
