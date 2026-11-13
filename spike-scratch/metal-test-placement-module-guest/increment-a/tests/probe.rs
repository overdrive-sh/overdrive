//! Probe test binary. Three modules, same four tests each (3 pass, 1 fail):
//! - `shared`  shape A: every test is a client of the module's one shared guest.
//! - `control` shape B: one nextest test boots one guest and runs the module's
//!   four (ignored-on-host) tests inside it.
//! - `pertest` today's behaviour: every test boots its own guest.
use modguest_probe::{body, declare_placement, netns_id, run_in_shared_guest};
use overdrive_testing::guest_suite::{inside_guest, run_in_guest};

const BROKER: &str = env!("CARGO_BIN_EXE_modguest-broker");

fn exe() -> std::path::PathBuf {
    std::env::current_exe().unwrap()
}

macro_rules! shared_tests {
    ($module:literal, $key:literal; $($name:ident),*) => { $(
        #[test]
        fn $name() {
            declare_placement!(::core::stringify!($name));
            if inside_guest() {
                return body(::core::concat!(module_path!(), "::", ::core::stringify!($name)));
            }
            let t = std::time::Instant::now();
            eprintln!("MODGUEST_HOST test={} netns={}", stringify!($name), netns_id());
            let r = run_in_shared_guest(
                std::path::Path::new(BROKER),
                $key,
                &exe(),
                concat!($module, "::", stringify!($name)),
            )
            .unwrap_or_else(|e| panic!("{e}"));
            eprint!("{}", r.console);
            eprintln!(
                "MODGUEST_HOST test={} exit={} runs_served={} broker_pid={} broker_netns={} exec_ms={} wall_ms={}",
                stringify!($name), r.exit, r.runs_served, r.broker_pid, r.broker_netns, r.exec_ms, t.elapsed().as_millis()
            );
            assert_eq!(r.exit, 0, "{} failed inside the shared guest", stringify!($name));
        }
    )* };
}

mod shared {
    use super::*;
    shared_tests!("shared", "probe-shared"; t_pass_1, t_pass_2, t_fail, t_pass_3, t_slow);
}

/// A second module of the same binary: its own broker, its own guest.
mod shared_b {
    use super::*;
    shared_tests!("shared_b", "probe-shared-b"; u_pass_1, u_pass_2, u_slow);
}

macro_rules! pertest_tests {
    ($($name:ident),*) => { $(
        #[test]
        fn $name() {
            declare_placement!(::core::stringify!($name));
            if inside_guest() {
                return body(::core::concat!(module_path!(), "::", ::core::stringify!($name)));
            }
            let t = std::time::Instant::now();
            let r = run_in_guest("cgroup-default", &exe(), concat!("pertest::", stringify!($name)));
            eprintln!("MODGUEST_HOST test={} wall_ms={} result={r:?}", stringify!($name), t.elapsed().as_millis());
            r.unwrap_or_else(|e| panic!("{e}"));
        }
    )* };
}

mod pertest {
    use super::*;
    pertest_tests!(t_pass_1, t_pass_2, t_fail, t_pass_3);
}

/// Control B inner tests: ignored on the host so nextest never schedules them
/// (the wrapper would refuse them unplaced); the control test runs them in
/// one guest with `--include-ignored`.
mod control_inner {
    use super::*;
    macro_rules! inner {
        ($($name:ident),*) => { $(
            #[test]
            #[ignore = "run only inside the control guest"]
            fn $name() { body(concat!(module_path!(), "::", stringify!($name))); }
        )* };
    }
    inner!(t_pass_1, t_pass_2, t_fail, t_pass_3);
}

mod control {
    use super::*;
    use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
    use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestKit, ScriptedBootExtra, WorkDir};
    use overdrive_testing::vm_fixture::default_staging_root;
    use std::time::{Duration, Instant};

    #[test]
    fn module_in_one_guest() {
        declare_placement!("module_in_one_guest");
        let t = Instant::now();
        let exe = exe();
        let staging = default_staging_root();
        let kit = GuestKit::provision(&staging).unwrap();
        let work = WorkDir::create(&staging, "modguest-control").unwrap();
        let mut spec = GuestImageSpec::scripted();
        let env_host = ["/usr/bin/env", "/bin/env"].into_iter().map(std::path::PathBuf::from).find(|p| p.is_file()).unwrap();
        spec.extra = vec![
            GuestFile { host: exe.canonicalize().unwrap(), guest: exe.clone(), with_shared_libraries: true },
            GuestFile { host: env_host, guest: "/usr/bin/env".into(), with_shared_libraries: true },
        ];
        let image = kit.image(work.path(), "ctl", &spec).unwrap();
        let staged = t.elapsed();
        let mut g = kit
            .boot_scripted_with(
                &image,
                FWD_GUEST_BOUNDARY.raw(15),
                &work.path().join("run"),
                &ScriptedBootExtra { cmdline: Vec::new(), memory_mib: Some(1024), vcpus: Some(2) },
            )
            .unwrap();
        let booted = t.elapsed();
        let off = g.vm.console().len();
        let exe_s = exe.display().to_string();
        let code = g
            .peer()
            .exec(
                &["/usr/bin/env", "OVERDRIVE_TEST_IN_GUEST=1", &exe_s, "control_inner::", "--include-ignored", "--nocapture", "--test-threads=1"],
                Duration::from_secs(120),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        eprint!("{}", g.vm.console().get(off..).unwrap_or(""));
        eprintln!(
            "MODGUEST_TIMING control boots=1 staging_ms={} boot_ms={} total_ms={} exit={code}",
            staged.as_millis(),
            (booted - staged).as_millis(),
            t.elapsed().as_millis()
        );
        let _ = g.vm.kill();
        assert_eq!(code, 0, "control_inner module had a failing test (exit {code})");
    }
}
