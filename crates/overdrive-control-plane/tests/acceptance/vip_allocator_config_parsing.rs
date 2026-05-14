//! Step 02-02 — VIP allocator config parsing acceptance scenarios.
//!
//! Five scenarios exercise the boot-time TOML parser surface for
//! `[dataplane.vip_allocator]` (`S-VIP-15` through `S-VIP-18`). The
//! parser surface owns: (a) section presence (`Missing` → boot
//! refusal), (b) delegation to `VipRange::new` for the three
//! type-level invariants (`Overlapping` / `ReservedOutsideRange` /
//! `ZeroCapacity`), and (c) the structured `health.startup.refused`
//! event emitted on every refusal.
//!
//! Per ADR-0049 § 5b. Type-level invariant coverage lives at
//! `crates/overdrive-dataplane/tests/allocator_properties.rs`
//! (step 01-01); this file is the parser-surface counterpart.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use overdrive_control_plane::vip_allocator_config::{
    VipAllocatorBootError, parse_vip_allocator_section,
};
use overdrive_dataplane::allocators::VipAllocatorConfigError;
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

// ---------------------------------------------------------------------------
// Tracing capture — custom Layer, no tracing-test dep.
// Mirrors the pattern at
// `tests/integration/cgroup_isolation/alloc_scope_has_writable_cpu_weight_and_memory_max.rs`.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CapturedEvent {
    target: String,
    event_field: Option<String>,
    cause: Option<String>,
}

#[derive(Default, Clone)]
struct CaptureLayer {
    events: Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        struct V {
            event_field: Option<String>,
            cause: Option<String>,
        }
        impl tracing::field::Visit for V {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                match field.name() {
                    "event" => self.event_field = Some(value.to_string()),
                    "cause" => self.cause = Some(value.to_string()),
                    _ => {}
                }
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let s = format!("{value:?}");
                match field.name() {
                    "event" => self.event_field = Some(s),
                    "cause" => self.cause = Some(s),
                    _ => {}
                }
            }
        }
        let mut v = V { event_field: None, cause: None };
        event.record(&mut v);
        self.events.lock().expect("events mutex").push(CapturedEvent {
            target: event.metadata().target().to_string(),
            event_field: v.event_field,
            cause: v.cause,
        });
    }
}

fn install_capture()
-> (Arc<std::sync::Mutex<Vec<CapturedEvent>>>, tracing::subscriber::DefaultGuard) {
    let layer = CaptureLayer::default();
    let events = layer.events.clone();
    let guard = tracing_subscriber::registry().with(layer).set_default();
    (events, guard)
}

fn assert_startup_refused(events: &[CapturedEvent], cause_substring: &str) {
    let hit = events.iter().any(|e| {
        e.target == "overdrive::health"
            && e.event_field.as_deref() == Some("health.startup.refused")
            && e.cause.as_deref().is_some_and(|c| c.contains(cause_substring))
    });
    assert!(
        hit,
        "expected health.startup.refused event with cause containing {cause_substring:?}, got events: {events:#?}",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-15 — missing [dataplane.vip_allocator] subsection refuses boot.
// ---------------------------------------------------------------------------

#[test]
fn missing_vip_allocator_config_boot_refuses() {
    // [dataplane] present but [dataplane.vip_allocator] absent.
    let toml_str = r"
[dataplane]
";
    let (events, _guard) = install_capture();
    let result = parse_vip_allocator_section(toml_str);

    let err = result.expect_err("expected refusal when [dataplane.vip_allocator] is absent");
    match &err {
        VipAllocatorBootError::Config(VipAllocatorConfigError::Missing { section }) => {
            assert_eq!(
                *section, "dataplane.vip_allocator",
                "Missing variant should name the missing section verbatim",
            );
        }
        other => panic!("expected Config(Missing), got {other:?}"),
    }

    let events_snapshot = events.lock().expect("events mutex").clone();
    assert_startup_refused(&events_snapshot, "dataplane.vip_allocator");
}

// ---------------------------------------------------------------------------
// S-VIP-15 happy path — valid config parses and produces a VipRange with
// capacity 256 for a /24 (no automatic network/broadcast exclusion).
// ---------------------------------------------------------------------------

#[test]
fn valid_vip_allocator_config_parses() {
    let toml_str = r#"
[dataplane.vip_allocator]
ranges = ["10.96.0.0/24"]
"#;
    let range = parse_vip_allocator_section(toml_str).expect("valid config should parse");
    assert_eq!(
        range.capacity(),
        256,
        "/24 has 256 addresses (network/broadcast NOT auto-excluded — operator policy)",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-16 — overlapping ranges at parser surface emit typed error.
// ---------------------------------------------------------------------------

#[test]
fn vip_allocator_config_overlapping_ranges_surfaces_typed_error() {
    let toml_str = r#"
[dataplane.vip_allocator]
ranges = ["10.96.0.0/24", "10.96.0.128/25"]
"#;
    let (events, _guard) = install_capture();
    let err = parse_vip_allocator_section(toml_str).expect_err("overlapping ranges must refuse");

    match &err {
        VipAllocatorBootError::Config(VipAllocatorConfigError::OverlappingRanges { a, b }) => {
            let names = format!("{a} {b}");
            assert!(
                names.contains("10.96.0.0/24") && names.contains("10.96.0.128/25"),
                "operator message must name both overlapping ranges; got {a} / {b}",
            );
        }
        other => panic!("expected Config(OverlappingRanges), got {other:?}"),
    }

    let events_snapshot = events.lock().expect("events mutex").clone();
    assert_startup_refused(&events_snapshot, "10.96.0.0/24");
}

// ---------------------------------------------------------------------------
// S-VIP-17 — reserved address outside any configured range.
// ---------------------------------------------------------------------------

#[test]
fn vip_allocator_config_reserved_outside_range_surfaces_typed_error() {
    let toml_str = r#"
[dataplane.vip_allocator]
ranges = ["10.96.0.0/24"]
reserved = ["10.96.1.1"]
"#;
    let (events, _guard) = install_capture();
    let err =
        parse_vip_allocator_section(toml_str).expect_err("reserved-outside-range must refuse");

    match &err {
        VipAllocatorBootError::Config(VipAllocatorConfigError::ReservedOutsideRange { addr }) => {
            assert_eq!(
                addr.to_string(),
                "10.96.1.1",
                "operator message must name the offending address verbatim",
            );
        }
        other => panic!("expected Config(ReservedOutsideRange), got {other:?}"),
    }

    let events_snapshot = events.lock().expect("events mutex").clone();
    assert_startup_refused(&events_snapshot, "10.96.1.1");
}

// ---------------------------------------------------------------------------
// S-VIP-18 — zero effective capacity (a /30 with all 4 addresses reserved).
// ---------------------------------------------------------------------------

#[test]
fn vip_allocator_config_zero_capacity_surfaces_typed_error() {
    let toml_str = r#"
[dataplane.vip_allocator]
ranges = ["10.96.0.0/30"]
reserved = ["10.96.0.0", "10.96.0.1", "10.96.0.2", "10.96.0.3"]
"#;
    let (events, _guard) = install_capture();
    let err = parse_vip_allocator_section(toml_str).expect_err("zero-capacity must refuse");

    match &err {
        VipAllocatorBootError::Config(VipAllocatorConfigError::ZeroCapacity) => {}
        other => panic!("expected Config(ZeroCapacity), got {other:?}"),
    }

    let events_snapshot = events.lock().expect("events mutex").clone();
    assert_startup_refused(&events_snapshot, "zero effective capacity");
}
