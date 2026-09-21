//! Tier-2 `BPF_PROG_TEST_RUN` partitions for the shared guest TCX classifier.

#![allow(
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::ptr_as_ptr,
    clippy::ref_as_ptr
)]

use std::os::fd::{AsFd as _, AsRawFd as _};

use aya::maps::{Array, HashMap};
use aya::programs::{ProgramFd, SchedClassifier};
use aya::{EbpfLoader, Pod};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

const TC_ACT_OK: u32 = 0;
const TC_ACT_SHOT: u32 = 2;
const INTERCEPT_MARK: u32 = 0x295a;
const ACCEPTED_MARK: u32 = 0x295b;
const SOURCE_IP: [u8; 4] = [100, 95, 0, 2];
const PEER_IP: [u8; 4] = [100, 95, 0, 3];
const GATEWAY_IP: [u8; 4] = [100, 95, 0, 1];
const SOURCE_MAC: [u8; 6] = [0x02, 0x00, 100, 95, 0, 2];
const PEER_MAC: [u8; 6] = [0x02, 0x00, 100, 95, 0, 3];
const BRIDGE_MAC: [u8; 6] = [0x02, 0x01, 0, 0, 0, 1];
const IFINDEX: u32 = 7;

#[repr(C)]
#[derive(Clone, Copy)]
struct Endpoint {
    source_ip: u32,
    source_mac: [u8; 6],
    source_pad: [u8; 2],
    bridge_mac: [u8; 6],
    bridge_pad: [u8; 2],
}

// SAFETY: `Endpoint` is `repr(C)`, plain bytes/integers, and has no padding
// with an invalid bit pattern; its layout is the kernel map ABI.
unsafe impl Pod for Endpoint {}

/// Host copy of Linux UAPI `struct __sk_buff` used only as
/// `BPF_PROG_TEST_RUN` context. Pointer-valued union arms are represented by
/// their ABI-sized `u64`; this test writes only `ingress_ifindex` and reads
/// `mark`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SkBuffContext {
    len: u32,
    pkt_type: u32,
    mark: u32,
    queue_mapping: u32,
    protocol: u32,
    vlan_present: u32,
    vlan_tci: u32,
    vlan_proto: u32,
    priority: u32,
    ingress_ifindex: u32,
    ifindex: u32,
    tc_index: u32,
    cb: [u32; 5],
    hash: u32,
    tc_classid: u32,
    data: u32,
    data_end: u32,
    napi_id: u32,
    family: u32,
    remote_ip4: u32,
    local_ip4: u32,
    remote_ip6: [u32; 4],
    local_ip6: [u32; 4],
    remote_port: u32,
    local_port: u32,
    data_meta: u32,
    flow_keys: u64,
    tstamp: u64,
    wire_len: u32,
    gso_segs: u32,
    sk: u64,
    gso_size: u32,
    tstamp_type_and_padding: [u8; 4],
    hwtstamp: u64,
}

struct RunResult {
    action: u32,
    output: Vec<u8>,
    mark: u32,
}

fn remove_pin_dir(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|message| (*message).to_owned()))
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

fn test_run(program: &ProgramFd, input: &[u8], ingress_ifindex: u32) -> std::io::Result<RunResult> {
    let mut output = vec![0_u8; input.len().max(64)];
    // SAFETY: the kernel ABI structs are POD and zero initialization is the
    // canonical construction used by aya/libbpf before fields are populated.
    let mut input_ctx: SkBuffContext = unsafe { std::mem::zeroed() };
    // SAFETY: same POD construction; the kernel fills this output context.
    let mut output_ctx: SkBuffContext = unsafe { std::mem::zeroed() };
    input_ctx.ingress_ifindex = ingress_ifindex;
    // SAFETY: `bpf_attr` is a C union with no destructor.
    let mut attr: bpf_attr = unsafe { std::mem::zeroed() };
    // SAFETY: only the `test` union arm is written and later read.
    let test = unsafe { &mut attr.test };
    test.prog_fd = u32::try_from(program.as_fd().as_raw_fd()).expect("live fd is nonnegative");
    test.data_in = input.as_ptr() as u64;
    test.data_size_in = u32::try_from(input.len()).expect("test frame length fits u32");
    test.data_out = output.as_mut_ptr() as u64;
    test.data_size_out = u32::try_from(output.len()).expect("output length fits u32");
    test.ctx_in = std::ptr::from_ref(&input_ctx) as u64;
    test.ctx_size_in = u32::try_from(std::mem::size_of::<SkBuffContext>()).expect("ctx size");
    test.ctx_out = std::ptr::from_mut(&mut output_ctx) as u64;
    test.ctx_size_out = u32::try_from(std::mem::size_of::<SkBuffContext>()).expect("ctx size");
    test.repeat = 1;

    // SAFETY: pointers remain live for the syscall and sizes match their
    // buffers/ABI structs.
    let result = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_PROG_TEST_RUN as libc::c_int,
            &raw mut attr,
            std::mem::size_of::<bpf_attr>() as libc::c_uint,
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a successful syscall initialized the active `test` arm.
    let returned = unsafe { attr.test };
    output.truncate(usize::try_from(returned.data_size_out).expect("output size fits usize"));
    Ok(RunResult { action: returned.retval, output, mark: output_ctx.mark })
}

fn ipv4_frame(
    destination_mac: [u8; 6],
    source_mac: [u8; 6],
    source_ip: [u8; 4],
    protocol: u8,
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(54);
    frame.extend_from_slice(&destination_mac);
    frame.extend_from_slice(&source_mac);
    frame.extend_from_slice(&[0x08, 0x00]);
    frame.extend_from_slice(&[0x45, 0, 0, 40, 0, 0, 0, 0, 64, protocol, 0, 0]);
    frame.extend_from_slice(&source_ip);
    frame.extend_from_slice(&PEER_IP);
    frame.extend_from_slice(&[0_u8; 20]);
    frame
}

fn arp_frame(sender_mac: [u8; 6], sender_ip: [u8; 4], opcode: u16) -> Vec<u8> {
    let mut frame = Vec::with_capacity(42);
    frame.extend_from_slice(&[0xff; 6]);
    frame.extend_from_slice(&sender_mac);
    frame.extend_from_slice(&[0x08, 0x06]);
    frame.extend_from_slice(&1_u16.to_be_bytes());
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    frame.push(6);
    frame.push(4);
    frame.extend_from_slice(&opcode.to_be_bytes());
    frame.extend_from_slice(&sender_mac);
    frame.extend_from_slice(&sender_ip);
    frame.extend_from_slice(&[0_u8; 6]);
    frame.extend_from_slice(&GATEWAY_IP);
    frame
}

fn assert_single_counter(counters: &Array<&mut aya::maps::MapData, u64>, expected: u32) {
    for slot in 0..8 {
        assert_eq!(
            counters.get(&slot, 0).expect("read classifier counter"),
            u64::from(slot == expected),
            "only counter slot {expected} advances"
        );
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(env)]
fn classifier_partitions_return_one_verdict_and_advance_one_exact_counter() {
    let artifact = super::bpf_artifact::path();
    let pin_dir = std::path::PathBuf::from(format!(
        "/sys/fs/bpf/overdrive-test-guest-tcx-{}",
        std::process::id()
    ));
    remove_pin_dir(&pin_dir).expect("remove stale isolated bpffs pin directory");
    std::fs::create_dir_all(&pin_dir).expect("create isolated bpffs pin directory");
    let primary = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::xdp_pass_test_run::pre_pin_service_map(&pin_dir);
        let mut bpf = EbpfLoader::new()
            .map_pin_path(&pin_dir)
            .allow_unsupported_maps()
            .load_file(&artifact)
            .unwrap_or_else(|error| panic!("load {}: {error}", artifact.display()));
        let program_fd = {
            let program: &mut SchedClassifier = bpf
                .program_mut("gh295c_endpoint")
                .expect("production guest endpoint classifier")
                .try_into()
                .expect("program is SCHED_CLS");
            program.load().expect("load guest endpoint classifier");
            program.fd().expect("classifier fd").try_clone().expect("clone classifier fd")
        };
        {
            let mut endpoints: HashMap<_, u32, Endpoint> =
                HashMap::try_from(bpf.map_mut("ENDPOINTS").expect("production endpoint map"))
                    .expect("typed endpoint map");
            endpoints
                .insert(
                    IFINDEX,
                    Endpoint {
                        source_ip: u32::from_be_bytes(SOURCE_IP),
                        source_mac: SOURCE_MAC,
                        source_pad: [0; 2],
                        bridge_mac: BRIDGE_MAC,
                        bridge_pad: [0; 2],
                    },
                    0,
                )
                .expect("insert endpoint");
        }
        let mut counters: Array<_, u64> =
            Array::try_from(bpf.map_mut("COUNTERS").expect("production counter map"))
                .expect("typed counter array");

        let mut cases = vec![
            (
                "peer tcp",
                ipv4_frame(PEER_MAC, SOURCE_MAC, SOURCE_IP, 6),
                IFINDEX,
                TC_ACT_OK,
                1,
                Some(INTERCEPT_MARK),
            ),
            (
                "gateway tcp",
                ipv4_frame(BRIDGE_MAC, SOURCE_MAC, SOURCE_IP, 6),
                IFINDEX,
                TC_ACT_OK,
                1,
                Some(INTERCEPT_MARK),
            ),
            (
                "gateway udp",
                ipv4_frame(BRIDGE_MAC, SOURCE_MAC, SOURCE_IP, 17),
                IFINDEX,
                TC_ACT_OK,
                0,
                Some(ACCEPTED_MARK),
            ),
            (
                "arp request",
                arp_frame(SOURCE_MAC, SOURCE_IP, 1),
                IFINDEX,
                TC_ACT_OK,
                6,
                Some(ACCEPTED_MARK),
            ),
            (
                "arp reply",
                arp_frame(SOURCE_MAC, SOURCE_IP, 2),
                IFINDEX,
                TC_ACT_OK,
                6,
                Some(ACCEPTED_MARK),
            ),
            (
                "map miss",
                ipv4_frame(PEER_MAC, SOURCE_MAC, SOURCE_IP, 6),
                IFINDEX + 1,
                TC_ACT_SHOT,
                2,
                None,
            ),
            (
                "mac spoof",
                ipv4_frame(PEER_MAC, [0x02, 0, 1, 2, 3, 4], SOURCE_IP, 6),
                IFINDEX,
                TC_ACT_SHOT,
                3,
                None,
            ),
            (
                "ip spoof",
                ipv4_frame(PEER_MAC, SOURCE_MAC, [100, 95, 0, 9], 6),
                IFINDEX,
                TC_ACT_SHOT,
                4,
                None,
            ),
            (
                "peer udp bypass",
                ipv4_frame(PEER_MAC, SOURCE_MAC, SOURCE_IP, 17),
                IFINDEX,
                TC_ACT_SHOT,
                5,
                None,
            ),
        ];
        let mut non_ip = ipv4_frame(PEER_MAC, SOURCE_MAC, SOURCE_IP, 6);
        non_ip[12..14].copy_from_slice(&0x86dd_u16.to_be_bytes());
        cases.push(("non IPv4/ARP", non_ip, IFINDEX, TC_ACT_SHOT, 5, None));
        let mut arp_bad_type = arp_frame(SOURCE_MAC, SOURCE_IP, 1);
        arp_bad_type[14..16].copy_from_slice(&2_u16.to_be_bytes());
        cases.push(("ARP hardware type", arp_bad_type, IFINDEX, TC_ACT_SHOT, 7, None));
        let mut arp_bad_protocol = arp_frame(SOURCE_MAC, SOURCE_IP, 1);
        arp_bad_protocol[16..18].copy_from_slice(&0x86dd_u16.to_be_bytes());
        cases.push(("ARP protocol type", arp_bad_protocol, IFINDEX, TC_ACT_SHOT, 7, None));
        let mut arp_bad_length = arp_frame(SOURCE_MAC, SOURCE_IP, 1);
        arp_bad_length[18] = 5;
        cases.push(("ARP hardware length", arp_bad_length, IFINDEX, TC_ACT_SHOT, 7, None));
        let mut arp_bad_protocol_length = arp_frame(SOURCE_MAC, SOURCE_IP, 1);
        arp_bad_protocol_length[19] = 5;
        cases.push(("ARP protocol length", arp_bad_protocol_length, IFINDEX, TC_ACT_SHOT, 7, None));
        cases.push((
            "ARP opcode",
            arp_frame(SOURCE_MAC, SOURCE_IP, 3),
            IFINDEX,
            TC_ACT_SHOT,
            7,
            None,
        ));
        let mut arp_ethernet_mac_spoof = arp_frame(SOURCE_MAC, SOURCE_IP, 1);
        arp_ethernet_mac_spoof[6..12].copy_from_slice(&[0x02, 0, 1, 2, 3, 4]);
        cases.push((
            "ARP Ethernet source MAC spoof",
            arp_ethernet_mac_spoof,
            IFINDEX,
            TC_ACT_SHOT,
            3,
            None,
        ));
        let mut arp_sender_mac_spoof = arp_frame(SOURCE_MAC, SOURCE_IP, 1);
        arp_sender_mac_spoof[22..28].copy_from_slice(&[0x02, 0, 1, 2, 3, 4]);
        cases.push((
            "ARP sender hardware spoof",
            arp_sender_mac_spoof,
            IFINDEX,
            TC_ACT_SHOT,
            3,
            None,
        ));
        cases.push((
            "ARP sender spoof",
            arp_frame(SOURCE_MAC, [100, 95, 0, 9], 1),
            IFINDEX,
            TC_ACT_SHOT,
            4,
            None,
        ));
        for length in 14..42 {
            cases.push((
                "truncated Ethernet/ARP",
                arp_frame(SOURCE_MAC, SOURCE_IP, 1)[..length].to_vec(),
                IFINDEX,
                TC_ACT_SHOT,
                7,
                None,
            ));
        }
        for length in 34..38 {
            cases.push((
                "truncated IPv4/TCP",
                ipv4_frame(PEER_MAC, SOURCE_MAC, SOURCE_IP, 6)[..length].to_vec(),
                IFINDEX,
                TC_ACT_SHOT,
                7,
                None,
            ));
        }

        for (name, frame, ifindex, action, counter, mark) in cases {
            for slot in 0..8 {
                counters.set(slot, 0, 0).expect("reset classifier counter");
            }
            let result = test_run(&program_fd, &frame, ifindex)
                .unwrap_or_else(|error| panic!("{name} BPF_PROG_TEST_RUN: {error}"));
            assert_eq!(result.action, action, "{name} verdict");
            if let Some(mark) = mark {
                assert_eq!(result.mark, mark, "{name} mark");
            }
            if name == "peer tcp" {
                assert_eq!(&result.output[..6], &BRIDGE_MAC, "peer TCP rewrites destination MAC");
                assert_eq!(
                    &result.output[6..],
                    &frame[6..],
                    "peer TCP preserves source MAC, EtherType, original IPv4 addresses and ports"
                );
            }
            assert_single_counter(&counters, counter);
        }
    }));
    let cleanup = remove_pin_dir(&pin_dir);
    match (primary, cleanup) {
        (Ok(()), Ok(())) => {}
        (Ok(()), Err(cleanup)) => panic!("remove isolated bpffs pin directory: {cleanup}"),
        (Err(primary), Ok(())) => std::panic::resume_unwind(primary),
        (Err(primary), Err(cleanup)) => panic!(
            "classifier body failed: {}; remove isolated bpffs pin directory also failed: {cleanup}",
            panic_message(primary.as_ref())
        ),
    }
}
