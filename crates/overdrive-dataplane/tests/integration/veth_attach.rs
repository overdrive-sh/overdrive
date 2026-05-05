//! S-2.2-01..03 — Real-iface XDP attach.
//!
//! Tags: `@US-01` `@K1` `@slice-01` `@real-io @adapter-integration`.
//! Tier: Tier 3.
//!
//! See `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! for the Gherkin specification of each scenario.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-01 — Real veth pair attach with packet count assertion.
/// Starting scenario for DELIVER (NOT `@pending`).
#[test]
fn xdp_attaches_to_real_veth_and_packet_counter_increments() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-01 — \
         create veth0/veth1, attach xdp_pass, push 100 frames, \
         assert PACKET_COUNTER reads 100"
    );
}

/// S-2.2-02 — Native attach failure logs structured fallback warning.
///
/// Drives `EbpfDataplane::new(iface)` on a kernel `dummy` interface
/// (the in-tree `dummy` driver does not implement the
/// `ndo_bpf`/`ndo_xdp` op required for native XDP attach), so
/// `bpf_link_create` / `netlink_set_xdp_fd` returns `EOPNOTSUPP`
/// from the native (`DRV_MODE`) path. The loader retries in generic
/// mode (`SKB_MODE`) and emits a single structured `tracing::warn!`
/// event named `xdp.attach.fallback_generic` carrying the iface name.
///
/// Captured via a custom `tracing::Layer` rather than
/// `tracing-test`, because we need to assert on the *event name*
/// (the metadata `name` field used by `tracing::warn!(name: "...",
/// ...)`) and not just the rendered message text.
///
/// Requires root for `ip link add dummy0 type dummy`. The Lima
/// wrapper's default-root execution covers this; CI runs the test
/// in a privileged container with `CAP_NET_ADMIN`.
///
/// **Currently `#[ignore]`** — exercising this path requires a
/// real `overdrive_bpf.o` produced by `cargo xtask bpf-build` (step
/// 02-01). Phase 2.1 ships a 1.3 KB placeholder ELF; `aya::Ebpf::load`
/// rejects it before reaching the attach call. The fallback
/// classification logic itself is unit-tested in `lib.rs` (see
/// `should_fallback_to_generic`); the end-to-end happy-fallback
/// path is gated through the LVH Tier 3 smoke at step 03-02 once
/// the real artifact exists.
#[test]
#[ignore = "S-2.2-02 — needs real overdrive_bpf.o from step 02-01; classification covered by unit test in lib.rs; end-to-end path covered by LVH smoke (step 03-02)"]
#[serial_test::serial(net_admin)]
fn native_attach_failure_logs_fallback_warning() {
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use overdrive_dataplane::EbpfDataplane;
    use tracing::Subscriber;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;
    use tracing_subscriber::Layer;

    /// Records every event observed by the subscriber. Keeps the
    /// metadata `name` and any `iface` field's `Display`-rendered
    /// value — enough for the assertions below without taking a
    /// dependency on `tracing-test`.
    #[derive(Default)]
    struct CapturedEvent {
        name: &'static str,
        iface: Option<String>,
    }

    #[derive(Default, Clone)]
    struct CaptureLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            struct IfaceVisitor {
                iface: Option<String>,
            }
            impl tracing::field::Visit for IfaceVisitor {
                fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                    if field.name() == "iface" {
                        self.iface = Some(value.to_string());
                    }
                }
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "iface" {
                        self.iface = Some(format!("{value:?}").trim_matches('"').to_string());
                    }
                }
            }
            let mut v = IfaceVisitor { iface: None };
            event.record(&mut v);
            self.events.lock().expect("events mutex").push(CapturedEvent {
                name: event.metadata().name(),
                iface: v.iface,
            });
        }
    }

    /// RAII guard — tears down the dummy interface on drop, even when
    /// assertions panic mid-test.
    struct DummyIface {
        name: &'static str,
    }
    impl Drop for DummyIface {
        fn drop(&mut self) {
            let _ = Command::new("ip").args(["link", "del", self.name]).status();
        }
    }

    let iface = "ovd-dummy0";

    // Best-effort cleanup of any leftover from a prior aborted run.
    let _ = Command::new("ip").args(["link", "del", iface]).status();

    let add = Command::new("ip")
        .args(["link", "add", iface, "type", "dummy"])
        .status()
        .expect("ip link add: spawn");
    assert!(add.success(), "ip link add {iface} type dummy must succeed (need root + CAP_NET_ADMIN)");
    let _guard = DummyIface { name: iface };

    let up = Command::new("ip")
        .args(["link", "set", iface, "up"])
        .status()
        .expect("ip link set up: spawn");
    assert!(up.success(), "ip link set {iface} up must succeed");

    // Capture-layer subscriber. Scoped to this test only — `with_default`
    // installs and removes when the closure returns, so other tests in
    // the same process keep their global subscriber.
    let capture = CaptureLayer::default();
    let events = capture.events.clone();

    let subscriber = tracing_subscriber::registry().with(capture);
    let result = tracing::subscriber::with_default(subscriber, || EbpfDataplane::new(iface));

    assert!(
        result.is_ok(),
        "EbpfDataplane::new must succeed via generic fallback when native is rejected: {:?}",
        result.err()
    );

    let captured = events.lock().expect("events mutex");
    let fallback_events: Vec<&CapturedEvent> = captured
        .iter()
        .filter(|e| e.name == "xdp.attach.fallback_generic")
        .collect();
    assert_eq!(
        fallback_events.len(),
        1,
        "expected exactly one xdp.attach.fallback_generic event, got {} (all events: {:?})",
        fallback_events.len(),
        captured
            .iter()
            .map(|e| (e.name, e.iface.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fallback_events[0].iface.as_deref(),
        Some(iface),
        "fallback event must carry iface field equal to the requested name"
    );
}

/// S-2.2-03 — Missing iface produces typed `IfaceNotFound` error.
///
/// Driving port: `EbpfDataplane::new(iface)`. Asserts that a deliberately
/// non-existent interface name surfaces as the typed
/// [`DataplaneError::IfaceNotFound`] variant — never as a generic
/// `LoadFailed` string match. The loader resolves iface name to ifindex
/// via `nix::if_nametoindex`; `ENODEV` / `ENOENT` from the kernel map
/// to this variant before any aya BPF program is loaded.
///
/// Does not require root — name resolution fails before XDP attach.
#[test]
fn missing_iface_returns_typed_iface_not_found_error() {
    use overdrive_core::traits::dataplane::DataplaneError;
    use overdrive_dataplane::EbpfDataplane;

    // Deliberately non-existent interface — the kernel will return
    // ENODEV from if_nametoindex.
    let iface = "overdrive-veth-not-here-9999";
    let result = EbpfDataplane::new(iface);

    match result {
        Err(DataplaneError::IfaceNotFound { iface: returned }) => {
            assert_eq!(returned, iface, "IfaceNotFound must echo the requested iface name");
        }
        Err(other) => {
            panic!("expected DataplaneError::IfaceNotFound {{ iface: {iface:?} }}, got {other:?}")
        }
        Ok(_) => panic!("expected Err on missing iface, got Ok"),
    }
}
