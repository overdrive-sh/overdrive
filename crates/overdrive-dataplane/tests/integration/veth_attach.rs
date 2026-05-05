//! S-2.2-01..03 — Real-iface XDP attach.
//!
//! Tags: `@US-01` `@K1` `@slice-01` `@real-io @adapter-integration`.
//! Tier: Tier 3.
//!
//! See `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! for the Gherkin specification of each scenario.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]
// `expect_used` is workspace-wide `warn` per `.claude/rules/development.md`
// § Errors. Tests that interact with kernel network state via `ip(8)` and
// std::sync::Mutex use `.expect(...)` to surface RAII-fail-fast at the
// assertion site — matching the convention in
// `crates/overdrive-worker/tests/integration/exec_driver/`.
//
// `significant_drop_tightening` (nursery) flags the `events.lock()` guard
// held across the assertion block — narrowing the lock scope here would
// require cloning the captured-events vec out before assertions, which
// trades clippy quiet for noisier test code.
#![allow(clippy::expect_used, clippy::significant_drop_tightening)]

/// S-2.2-01 — Real veth pair attach with packet count assertion.
///
/// Drives [`EbpfDataplane::new`] against a freshly-created veth pair,
/// pushes 100 minimal Ethernet+IPv4+UDP frames out the peer end, and
/// asserts the XDP `xdp_pass` program's `PKTS` LRU map records exactly
/// 100 increments via real `bpftool map dump -j` JSON parsing.
///
/// Capability-gated: requires `CAP_NET_ADMIN` for veth creation and
/// raw-socket sendto. Bails early with a skip message on EPERM/EACCES
/// rather than failing — matches the convention in
/// `crates/overdrive-worker/tests/integration/exec_driver/`. The Lima
/// wrapper (`cargo xtask lima run --`) runs as root by default; CI
/// runs the integration job as root.
///
/// Map name `PKTS` matches the source in
/// `crates/overdrive-bpf/src/main.rs` — the map is an `LruHashMap<u32,
/// u64>` keyed at index 0 with the running count. The roadmap step
/// description (`PACKET_COUNTER`) is a working name for the
/// underlying map; the test reads the actual exported symbol.
///
/// **Currently `#[ignore]`** — exercising this end-to-end path
/// requires a fully-formed BTF-equipped `overdrive_bpf.o` produced by
/// `cargo xtask bpf-build` (step 02-01). The Phase 2.1 baseline
/// pipeline emits a minimal ELF that lacks a `.BTF` section, which
/// `aya::Ebpf::load` rejects with `"error parsing ELF data"` before
/// the attach call is reached (verified at step 01-03 GREEN attempt
/// — see commit body). Symmetric blocker to S-2.2-02 below; the
/// veth/sendto/`bpftool` test scaffolding (helpers, raw-socket
/// injection, `bpftool map dump -j` parser, `parse_pkts_dump` unit
/// tests) is intentionally landed in this step so the GREEN flip at
/// step 02-01 is `s/ignore/run/` plus a re-run.
#[test]
#[ignore = "S-2.2-01 — needs real BTF-equipped overdrive_bpf.o from step 02-01; veth + sendto + bpftool scaffolding lands here at step 01-03"]
#[serial_test::serial(net_admin)]
fn xdp_attaches_to_real_veth_and_packet_counter_increments() {
    use overdrive_dataplane::EbpfDataplane;

    use super::helpers::veth::{VethError, VethPair};

    let host = "ovd-veth0";
    let peer = "ovd-veth1";

    let veth = match VethPair::create(host, peer) {
        Ok(v) => v,
        Err(VethError::CapNetAdminRequired) => {
            eprintln!(
                "skip: S-2.2-01 needs CAP_NET_ADMIN for veth setup — \
                 run via `cargo xtask lima run --` (default-root)"
            );
            return;
        }
        Err(e) => panic!("veth setup failed: {e}"),
    };

    // Best-effort: ensure no stale PKTS map is hanging around in the
    // kernel from a prior test run that did not detach cleanly. The
    // map is per-program; once the previous EbpfDataplane was dropped
    // the map is collected, but a stale program pinned to another
    // iface (or a leaked iface) could still hold the same name. The
    // map name `PKTS` is unique to this BPF object so name collision
    // is unlikely but the assertion is a tighter contract anyway.

    let _dataplane = EbpfDataplane::new(&veth.host)
        .unwrap_or_else(|e| panic!("EbpfDataplane::new({:?}) failed: {e:?}", veth.host));

    // Send exactly 100 frames out the peer end. Frames travel veth1
    // → kernel → veth0 (ingress) → XDP → PKTS++.
    inject_frames(&veth.peer, 100).expect("inject 100 frames");

    // Read PKTS[0] via real `bpftool map dump -j`. The map is small
    // (LruHashMap, max 1024) — a single dump call is cheap. Map name
    // is `PKTS` per `crates/overdrive-bpf/src/main.rs`.
    let count = read_pkts_counter().expect("read PKTS counter via bpftool");
    assert_eq!(count, 100, "PKTS[0] should equal 100 after 100 frames; observed {count}");
}

/// Build and emit `n` minimal Ethernet+IPv4+UDP frames via an
/// `AF_PACKET` raw socket bound to `iface`. The frame body is the
/// shortest the kernel will accept on a veth (minimum 64 bytes on
/// Ethernet — pad with zeros). Destination MAC is broadcast so no ARP
/// is needed.
#[cfg(target_os = "linux")]
fn inject_frames(iface: &str, n: u32) -> Result<(), std::io::Error> {
    use std::ffi::CString;
    use std::io;
    use std::mem;
    use std::os::raw::c_int;

    // SAFETY: `if_nametoindex` is a thin wrapper over a libc call that
    // does not retain the input pointer past the call.
    let iface_c = CString::new(iface).expect("iface name has no NUL");
    // SAFETY: libc::if_nametoindex returns 0 on error and sets errno.
    let ifindex = unsafe { libc::if_nametoindex(iface_c.as_ptr()) };
    if ifindex == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: socket(2) is a thin syscall; PF_PACKET + SOCK_RAW +
    // ETH_P_ALL is the standard L2 send path.
    const ETH_P_ALL: c_int = 0x0003;
    let fd = unsafe {
        libc::socket(libc::PF_PACKET, libc::SOCK_RAW, (ETH_P_ALL as u16).to_be() as c_int)
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // Bind to iface — sockaddr_ll{ sll_family, sll_protocol, sll_ifindex }
    let mut addr: libc::sockaddr_ll = unsafe { mem::zeroed() };
    addr.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits in u16");
    addr.sll_protocol = (ETH_P_ALL as u16).to_be();
    addr.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits in i32");

    // Frame: 14B ethernet hdr + 20B IPv4 + 8B UDP + 22B pad = 64B min.
    let mut frame = [0u8; 64];
    // Dst MAC: broadcast (ff*6); Src MAC: 02:00:00:00:00:01 (locally-
    // administered unicast — irrelevant for ingress count).
    frame[0..6].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    frame[6..12].copy_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    // Ethertype: IPv4 (0x0800) — must match what the BPF program
    // sees, even though `xdp_pass` does not branch on it.
    frame[12..14].copy_from_slice(&[0x08, 0x00]);
    // Minimal IPv4 header: version=4, IHL=5, no options. Total length
    // covers IPv4 + UDP + 22B pad = 50.
    frame[14] = 0x45;
    frame[16..18].copy_from_slice(&50u16.to_be_bytes());
    frame[22] = 64; // TTL
    frame[23] = 17; // proto = UDP
    frame[26..30].copy_from_slice(&[10, 0, 0, 1]); // src
    frame[30..34].copy_from_slice(&[10, 0, 0, 2]); // dst
    // Minimal UDP header: src port, dst port, length, checksum=0.
    frame[34..36].copy_from_slice(&1234u16.to_be_bytes());
    frame[36..38].copy_from_slice(&5678u16.to_be_bytes());
    frame[38..40].copy_from_slice(&30u16.to_be_bytes()); // UDP len = 8 + 22

    let mut sent_err: Option<io::Error> = None;
    for _ in 0..n {
        // SAFETY: sendto writes from `frame` (length-bound) to the
        // bound socket; addr is fully initialised.
        let rc = unsafe {
            libc::sendto(
                fd,
                frame.as_ptr().cast(),
                frame.len(),
                0,
                std::ptr::addr_of!(addr).cast(),
                u32::try_from(mem::size_of::<libc::sockaddr_ll>())
                    .expect("sockaddr_ll size fits in socklen_t"),
            )
        };
        if rc < 0 {
            sent_err = Some(io::Error::last_os_error());
            break;
        }
    }
    // SAFETY: fd was returned by socket(); close exactly once.
    unsafe { libc::close(fd) };
    if let Some(e) = sent_err {
        return Err(e);
    }
    Ok(())
}

/// Read the `PKTS` map (key=0) via `bpftool map dump name PKTS -j`,
/// parse the JSON, and return the u64 count. Returns the kernel's
/// representation faithfully — for an `LruHashMap` with no eviction
/// pressure, the single entry at key=0 is the running counter.
#[cfg(target_os = "linux")]
fn read_pkts_counter() -> Result<u64, String> {
    use std::process::Command;

    let out = Command::new("bpftool")
        .args(["map", "dump", "name", "PKTS", "-j"])
        .output()
        .map_err(|e| format!("bpftool spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "bpftool map dump failed (status={:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout =
        String::from_utf8(out.stdout).map_err(|e| format!("bpftool stdout not UTF-8: {e}"))?;
    parse_pkts_dump(&stdout)
}

/// Parse `bpftool map dump -j` output for a `LruHashMap<u32, u64>`
/// holding key=0. Output shape (per bpftool 7.x):
/// `[{"key":[0,0,0,0],"value":[c0,c1,c2,c3,c4,c5,c6,c7]}]` — bytes in
/// little-endian on x86-64/aarch64. We sum across all rows defensively
/// (LRU may have been touched concurrently with no bound on key).
#[cfg(target_os = "linux")]
fn parse_pkts_dump(json: &str) -> Result<u64, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("bpftool json parse: {e} — output: {json}"))?;
    let array =
        value.as_array().ok_or_else(|| format!("expected top-level JSON array, got: {json}"))?;
    let mut total: u64 = 0;
    for entry in array {
        let bytes = entry
            .get("value")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| format!("entry missing `value` array: {entry}"))?;
        if bytes.len() != 8 {
            return Err(format!("expected 8-byte value (u64), got {} bytes: {entry}", bytes.len()));
        }
        let mut buf = [0u8; 8];
        for (i, b) in bytes.iter().enumerate() {
            // bpftool emits each byte as either a JSON number ("0xff") or string ("0xff").
            let byte = match b {
                serde_json::Value::Number(n) => {
                    u8::try_from(n.as_u64().ok_or_else(|| format!("byte not u64: {n}"))?)
                        .map_err(|e| format!("byte u8 conv: {e}"))?
                }
                serde_json::Value::String(s) => {
                    let s = s.trim_start_matches("0x");
                    u8::from_str_radix(s, 16).map_err(|e| format!("byte hex parse: {s} — {e}"))?
                }
                other => return Err(format!("unexpected byte type: {other}")),
            };
            buf[i] = byte;
        }
        total = total.saturating_add(u64::from_le_bytes(buf));
    }
    Ok(total)
}

#[cfg(test)]
mod parse_tests {
    //! Pure unit tests for the bpftool JSON parser — exercise the
    //! `parse_pkts_dump` helper without spawning a subprocess.

    use super::parse_pkts_dump;

    #[test]
    fn parses_single_entry_with_numeric_bytes() {
        // value = 100 LE = [0x64, 0, 0, 0, 0, 0, 0, 0]
        let json = r#"[{"key":[0,0,0,0],"value":[100,0,0,0,0,0,0,0]}]"#;
        assert_eq!(parse_pkts_dump(json).unwrap(), 100);
    }

    #[test]
    fn parses_single_entry_with_hex_string_bytes() {
        let json = r#"[{"key":[0,0,0,0],"value":["0x64","0x00","0x00","0x00","0x00","0x00","0x00","0x00"]}]"#;
        assert_eq!(parse_pkts_dump(json).unwrap(), 100);
    }

    #[test]
    fn empty_array_is_zero() {
        assert_eq!(parse_pkts_dump("[]").unwrap(), 0);
    }

    #[test]
    fn rejects_non_array_top_level() {
        assert!(parse_pkts_dump(r#"{"x":1}"#).is_err());
    }

    #[test]
    fn rejects_wrong_value_size() {
        let json = r#"[{"key":[0,0,0,0],"value":[1,2,3,4]}]"#;
        assert!(parse_pkts_dump(json).is_err());
    }
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
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;

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
            self.events
                .lock()
                .expect("events mutex")
                .push(CapturedEvent { name: event.metadata().name(), iface: v.iface });
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
    assert!(
        add.success(),
        "ip link add {iface} type dummy must succeed (need root + CAP_NET_ADMIN)"
    );
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
    let fallback_events: Vec<&CapturedEvent> =
        captured.iter().filter(|e| e.name == "xdp.attach.fallback_generic").collect();
    assert_eq!(
        fallback_events.len(),
        1,
        "expected exactly one xdp.attach.fallback_generic event, got {} (all events: {:?})",
        fallback_events.len(),
        captured.iter().map(|e| (e.name, e.iface.clone())).collect::<Vec<_>>()
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
