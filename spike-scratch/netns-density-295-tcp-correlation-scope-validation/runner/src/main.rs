use std::env;
use std::path::PathBuf;
use std::time::Duration;

use overdrive_testing::guest_kit::cid_bands::INIT_IN_GUEST_LANE;
use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestKit, WorkDir};
use overdrive_testing::vm_fixture::default_staging_root;

const GUEST_PROBE: &str = "/opt/tcp-correlation-ingress-probe";
const GUEST_BPF: &str = "/opt/tcp-correlation-ingress-bpf";
const GUEST_CGROUP: &str = "/sys/fs/cgroup/tcp-correlation-ingress-probe";

fn existing(arg: Option<String>, what: &str) -> PathBuf {
    let path = PathBuf::from(arg.unwrap_or_else(|| panic!("missing {what}")));
    assert!(path.is_file(), "{what} is not a file: {}", path.display());
    path
}

fn main() {
    let mut args = env::args().skip(1);
    let probe = existing(args.next(), "probe binary");
    let bpf = existing(args.next(), "BPF object");
    assert!(args.next().is_none(), "usage: runner <probe> <bpf-object>");

    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("provision disposable guest kit");
    let work = WorkDir::create(&staging, "tcp-correlation-scope-validation").expect("work dir");
    let image = kit
        .image(
            work.path(),
            "tcp-correlation-scope-validation",
            &GuestImageSpec {
                extra: vec![
                    GuestFile {
                        host: probe,
                        guest: PathBuf::from(GUEST_PROBE),
                        with_shared_libraries: false,
                    },
                    GuestFile {
                        host: bpf,
                        guest: PathBuf::from(GUEST_BPF),
                        with_shared_libraries: false,
                    },
                ],
                ..GuestImageSpec::scripted()
            },
        )
        .expect("build disposable guest image");
    let cid = INIT_IN_GUEST_LANE.raw(14);
    let mut guest = kit
        .boot_scripted(&image, cid, &work.path().join("run"))
        .expect("boot disposable Cloud Hypervisor guest");
    let status = guest
        .peer()
        .exec(&[GUEST_PROBE, GUEST_BPF, GUEST_CGROUP], Duration::from_secs(180))
        .expect("run probe inside disposable guest");
    let console = guest.vm.console();
    println!("{console}");
    assert_eq!(status, 0, "probe exit {status}; console:\n{console}");
    assert!(console.contains("VERDICT=WORKS"), "missing verdict; console:\n{console}");
    let _ = guest.take_peer().power_off();
    let _ = guest.vm.wait_exit(Duration::from_secs(30));
}
