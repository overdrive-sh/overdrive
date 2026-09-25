//! S-ND295-48 Lima real-kernel lifecycle of the D-295-R21 egress guest-MAC
//! classifier: loaded from the embedded object beside the ingress classifier,
//! attached first at a TAP's TCX egress point, pinned at `links/<tap>-egress`
//! beside `links/<tap>-ingress`, queried, detached, and counted in the ninth
//! counter slot.
//!
//! Driving ports are the pinned dataplane contract (feature delta § *Driven
//! port — TAP egress guest-MAC delivery*): `GuestTcxProgram::attach_first_egress`,
//! `GuestTcxLink::{program_id, pin, detach}`, D6's `query_attachment`,
//! `detach_pinned_link`, and `read_counter`. The independent oracle for "the
//! loaded egress program" is the kernel's own program list, read through aya.
//!
//! Isolation: one scratch TAP outside `ovd-tp-` (kept down, so nothing
//! egresses it) and one pin root under `/sys/fs/bpf/overdrive/`, both removed
//! on drop (`guest_tcx_inventory` fixture). The `overdrive-dataplane`
//! integration binary is `host-kernel-shared`.

#![allow(clippy::doc_markdown, clippy::expect_used)]

use std::collections::BTreeSet;

use overdrive_dataplane::guest_tcx::{
    GuestTcxCounter, GuestTcxError, GuestTcxProgram, TcxAttachPoint, detach_pinned_link,
    query_attachment, read_counter,
};

use super::guest_tcx_inventory::{
    COUNTER_SCHEMA, ENDPOINT_SCHEMA, PinRoot, ScratchTap, capture, endpoint,
};

/// Every loaded BPF program id. A program that disappears between the id
/// walk and its descriptor open is skipped (it cannot be the one this test
/// loaded); any other enumeration failure fails the test.
fn loaded_program_ids() -> BTreeSet<u32> {
    aya::programs::loaded_programs()
        .filter_map(|program| match program {
            Ok(program) => Some(program.id()),
            Err(aya::programs::ProgramError::SyscallError(error))
                if error.io_error.kind() == std::io::ErrorKind::NotFound =>
            {
                None
            }
            Err(error) => panic!("enumerate loaded BPF programs: {error}"),
        })
        .collect()
}

/// The one SCHED_CLS program named `name` that is loaded now and was not
/// loaded in `before`.
fn new_classifier(before: &BTreeSet<u32>, name: &[u8]) -> u32 {
    let candidates: Vec<u32> = aya::programs::loaded_programs()
        .filter_map(|program| match program {
            Ok(program) => Some(program),
            Err(aya::programs::ProgramError::SyscallError(error))
                if error.io_error.kind() == std::io::ErrorKind::NotFound =>
            {
                None
            }
            Err(error) => panic!("enumerate loaded BPF programs: {error}"),
        })
        .filter(|program| {
            !before.contains(&program.id())
                && program.name() == name
                && matches!(program.program_type(), Ok(aya::programs::ProgramType::SchedClassifier))
        })
        .map(|program| program.id())
        .collect();
    match candidates.as_slice() {
        [id] => *id,
        other => panic!(
            "exactly one newly loaded SCHED_CLS program named {} is expected; found {other:?}",
            String::from_utf8_lossy(name)
        ),
    }
}

fn attached(tap: &ScratchTap, attach_point: TcxAttachPoint) -> Vec<u32> {
    query_attachment(tap.name(), attach_point)
        .unwrap_or_else(|error| panic!("query TCX {attach_point:?} on {}: {error:?}", tap.name()))
        .program_ids
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-48 — The egress classifier is loaded, attached, pinned, counted,
/// and detached at the TAP's egress point like its ingress sibling.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 06-01 (S-ND295-48)"]
fn the_egress_classifier_attaches_pins_queries_and_detaches_at_the_egress_point() {
    let tap = ScratchTap::create('e');
    let pins = PinRoot::create("egress-lifecycle");
    let before = loaded_program_ids();

    // Load: the egress classifier comes from the same embedded object as the
    // ingress classifier and is a distinct loaded program.
    let identity = capture(&pins);
    let mut program = GuestTcxProgram::load(&identity).expect("load the embedded TCX object");
    let egress_program = new_classifier(&before, b"gh295c_egress");
    let ingress_program = new_classifier(&before, b"gh295c_endpoint");
    assert_ne!(egress_program, ingress_program, "the two classifiers are distinct programs");
    assert_eq!(
        program.pin_endpoint_map(&pins.endpoint_map()).expect("pin endpoint map"),
        ENDPOINT_SCHEMA
    );
    assert_eq!(
        program.pin_counter_map(&pins.counter_map()).expect("pin counter map"),
        COUNTER_SCHEMA
    );
    program.insert_endpoint(tap.ifindex(), endpoint()).expect("register the TAP's guest");

    // An unpinned egress link reports the egress program at TCX egress only,
    // and detaching it empties TCX egress.
    let unpinned = program.attach_first_egress(tap.name()).expect("attach first-egress");
    assert_eq!(unpinned.program_id(), egress_program, "the egress link reports the egress program");
    assert_eq!(attached(&tap, TcxAttachPoint::Egress), vec![egress_program]);
    assert!(
        attached(&tap, TcxAttachPoint::Ingress).is_empty(),
        "an egress attach leaves TCX ingress untouched"
    );
    unpinned.detach().expect("detach the unpinned egress link");
    assert!(
        attached(&tap, TcxAttachPoint::Egress).is_empty(),
        "detaching the unpinned egress link empties TCX egress"
    );

    // Both classifiers attached and pinned side by side under `links/`.
    let ingress = program.attach_first_ingress(tap.name()).expect("attach first-ingress");
    assert_eq!(ingress.program_id(), ingress_program);
    let ingress_pin = pins.link(&tap, "ingress");
    ingress.pin(&ingress_pin).expect("pin the ingress link");
    let egress = program.attach_first_egress(tap.name()).expect("attach first-egress again");
    assert_eq!(egress.program_id(), egress_program);
    let egress_pin = pins.link(&tap, "egress");
    egress.pin(&egress_pin).expect("pin the egress link");
    assert_eq!(
        egress_pin.file_name().and_then(|name| name.to_str()),
        Some(format!("{}-egress", tap.name()).as_str()),
        "the egress link pins at links/<tap>-egress"
    );
    assert_eq!(
        egress_pin.parent(),
        ingress_pin.parent(),
        "the egress pin sits beside the ingress pin"
    );
    aya::programs::links::PinnedLink::from_pin(&egress_pin)
        .unwrap_or_else(|error| panic!("{} holds a BPF link: {error}", egress_pin.display()));
    assert_eq!(attached(&tap, TcxAttachPoint::Egress), vec![egress_program]);
    assert_eq!(attached(&tap, TcxAttachPoint::Ingress), vec![ingress_program]);

    // The ninth counter slot is readable as the egress drop counter; the TAP
    // is down, so nothing has egressed it.
    assert_eq!(
        read_counter(pins.counter_map(), GuestTcxCounter::EgressDestinationDrop)
            .expect("read the egress drop counter"),
        0
    );

    // Fault: a real unpin/detach of the pinned egress link, then an absence
    // read-back. Only the egress attach point empties; the ingress sibling
    // and its pin are untouched; a second detach finds nothing to detach.
    detach_pinned_link(&egress_pin).expect("unpin and detach the egress link");
    assert!(!egress_pin.exists(), "the egress pin is gone");
    assert!(
        attached(&tap, TcxAttachPoint::Egress).is_empty(),
        "detaching the pinned egress link empties TCX egress"
    );
    assert_eq!(
        attached(&tap, TcxAttachPoint::Ingress),
        vec![ingress_program],
        "detaching egress leaves the ingress classifier attached"
    );
    assert!(ingress_pin.exists(), "the ingress pin is untouched");
    let absent = detach_pinned_link(&egress_pin);
    assert!(
        matches!(&absent, Err(GuestTcxError::Link { .. })),
        "an absent egress pin keeps the pinned-link source: {absent:?}"
    );

    detach_pinned_link(&ingress_pin).expect("detach the ingress sibling");
    drop(program);
}
