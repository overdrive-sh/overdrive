#!/usr/bin/env python3
"""Generate the instrumented overdrive-init build tree for the workload-cgroup spike (V42).

Nothing from crates/ is stored in the repo: this script reads crates/overdrive-init at BUILD time,
applies a handful of anchored textual patches, and writes the result under <out> (gitignored).
usage: gen_init.py <repo> <out> <wc_listen_obj> [guest_obj]
"""
import pathlib, re, shutil, sys

repo, out, wc_obj = pathlib.Path(sys.argv[1]).resolve(), pathlib.Path(sys.argv[2]).resolve(), sys.argv[3]
guest_obj = sys.argv[4] if len(sys.argv) > 4 else str(repo / 'target/bpf/overdrive_guest_bpf.o')
here = pathlib.Path(__file__).resolve().parent
src_crate = repo / "crates/overdrive-init"
if out.exists():
    shutil.rmtree(out)
dst = out / "overdrive-init"
shutil.copytree(src_crate / "src", dst / "src")
shutil.copy(here / "guest/wc.rs", dst / "src/wc.rs")


def patch(rel, old, new):
    p = dst / rel
    s = p.read_text()
    assert s.count(old) == 1, f"anchor in {rel} matched {s.count(old)}x: {old[:70]!r}"
    p.write_text(s.replace(old, new))


# 1. module
patch("src/main.rs", "mod guest_flow;\nmod transport;\n", "mod guest_flow;\nmod transport;\nmod wc;\n")
# 2. init start mark
patch("src/main.rs", "fn run() -> Result<(), InitError> {\n    let pid = std::process::id();\n",
      "fn run() -> Result<(), InitError> {\n    crate::wc::start();\n    let pid = std::process::id();\n")
# 3. READY sent mark
patch("src/main.rs", "|conn, token: GuestTransportToken| send(conn, &ready_message(pid, &token)),",
      "|conn, token: GuestTransportToken| {\n            let r = send(conn, &ready_message(pid, &token));\n            crate::wc::ready_sent();\n            r\n        },")
# 4. workload spawn: clone3(CLONE_INTO_CGROUP) in placed modes, std Command otherwise
patch("src/main.rs", """    let mut child_command = Command::new(command);
    child_command.args(&argv[1..]).process_group(0);
    let mut child = child_command
        .spawn()
        .map_err(|source| InitError::Spawn { command: command.clone(), source })?;
    let child_pid =
        Pid::from_raw(i32::try_from(child.id()).map_err(|_| InitError::Wait(Errno::EOVERFLOW))?);
    supervise_command(conn, &mut child, child_pid)
""", """    let raw_pid = if crate::wc::places_in_cgroup() {
        crate::wc::spawn_in_workload_cgroup(argv)
            .map_err(|source| InitError::Spawn { command: command.clone(), source })?
    } else {
        let mut child_command = Command::new(command);
        child_command.args(&argv[1..]).process_group(0);
        let child = child_command
            .spawn()
            .map_err(|source| InitError::Spawn { command: command.clone(), source })?;
        i32::try_from(child.id()).map_err(|_| InitError::Wait(Errno::EOVERFLOW))?
    };
    let child_pid = Pid::from_raw(raw_pid);
    let status = supervise_command(conn, child_pid)?;
    crate::wc::finish();
    Ok(status)
""")
patch("src/main.rs", "    conn: &mut File,\n    _child: &mut std::process::Child,\n    child_pid: Pid,\n",
      "    conn: &mut File,\n    child_pid: Pid,\n")
# 5. transport hooks
patch("src/transport/mod.rs", "    ops.create_dummy_device().map_err(|source| GuestTransportSetupError::DummyDevice { source })?;\n",
      "    crate::wc::pre_binds();\n    ops.create_dummy_device().map_err(|source| GuestTransportSetupError::DummyDevice { source })?;\n")
patch("src/transport/mod.rs", "    ops.load_forwarder_module()?;\n", "    crate::wc::post_programs();\n    ops.load_forwarder_module()?;\n")

# 6. same-object hook: after the last production program is attached, hand the production Ebpf to wc.rs
patch("src/transport/kernel.rs", "        }\n        Ok(())\n    }\n\n    fn load_forwarder_module",
      "        }\n        if program == GuestProgram::SockRelease {\n            crate::wc::attach_in_object(bpf);\n        }\n        Ok(())\n    }\n\n    fn load_forwarder_module")

# 7. negative control for the correlation check (mode nocorr): do not attach the root tcp_correlation sockops
patch("src/transport/kernel.rs", "try_into()?;\n                prog.attach(cgroup, CgroupAttachMode::Single)?;\n            }\n            GuestProgram::SockRelease",
      "try_into()?;\n                if !crate::wc::skip_correlation() {\n                    prog.attach(cgroup, CgroupAttachMode::Single)?;\n                }\n            }\n            GuestProgram::SockRelease")

# crate manifest: drop dev-dependencies (not built), keep everything else
m = (src_crate / "Cargo.toml").read_text()
i, j = m.index("[dev-dependencies]"), m.index("[lints]")
m = m[:i] + m[j:]
(dst / "Cargo.toml").write_text(m)

# build script: point at the repo's built guest objects and the spike's listen object
(dst / "build.rs").write_text(f'''fn main() {{
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={wc_obj}");
    println!("cargo:rerun-if-changed={guest_obj}");
    println!("cargo:rustc-env=OVERDRIVE_GUEST_BPF_OBJECT_PATH={guest_obj}");
    println!("cargo:rustc-env=OVERDRIVE_GUEST_TCP_CORRELATION_CANDIDATE_BPF_OBJECT_PATH={repo}/target/bpf/overdrive_guest_tcp_correlation_candidate_bpf.o");
    println!("cargo:rustc-env=WC_LISTEN_OBJ_PATH={wc_obj}");
}}
''')

# workspace root: the repo root manifest, members cut to the one generated crate, paths made absolute
r = (repo / "Cargo.toml").read_text()
r = re.sub(r"members = \[.*?\n\]\n", 'members = ["overdrive-init"]\n', r, count=1, flags=re.S)
r = re.sub(r"default-members = \[.*?\n\]\n", 'default-members = ["overdrive-init"]\n', r, count=1, flags=re.S)
r = r.replace('path = "crates/', f'path = "{repo}/crates/')
(out / "Cargo.toml").write_text(r)
shutil.copy(repo / "Cargo.lock", out / "Cargo.lock")
print("generated", out)
