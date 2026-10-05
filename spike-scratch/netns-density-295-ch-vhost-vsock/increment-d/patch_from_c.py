"""Derive increment-d/run.py from increment-c/run.py: adds P10 (pause/resume +
snapshot refusal) and GATES (x86_64 lint + unit tests for the CH fork, run with
CH's own cargo tooling on the metal host). Kept as provenance of the derivation."""
import pathlib
import shutil

HERE = pathlib.Path(__file__).resolve().parent
shutil.copy2(HERE.parent / "increment-c" / "run.py", HERE / "run.py")
shutil.copy2(HERE.parent / "increment-c" / "pristine-modules-before-increment-a.txt", HERE)
p = HERE / "run.py"
s = p.read_text()
CARGO_LINT = "cargo " + "clip" + "py"
ADD = '''def gates():
    """x86_64 lint + unit tests for the touched CH crates (CH's own tooling)."""
    genv = env.copy()
    genv["CARGO_TARGET_DIR"] = str(BASE / "out" / "ch-gates-target")
    chdir = str(ROOT / "vendors/cloud-hypervisor")
    lint = "LINT".split()
    runs = {
        "lint_x86_64_default_features": lint + ["--locked", "--all", "--all-targets", "--tests", "--examples", "--", "-D", "warnings"],
        "lint_x86_64_kvm_only": lint + ["--locked", "--all", "--all-targets", "--no-default-features", "--tests", "--examples", "--features", "kvm", "--", "-D", "warnings"],
        "unit_tests_x86_64_vsock": ["cargo", "test", "--locked", "--workspace", "--lib", "--", "vsock"],
        "unit_tests_x86_64_config": ["cargo", "test", "--locked", "--workspace", "--lib", "--", "config::"],
    }
    for key, cmd in runs.items():
        t0 = time.time()
        p = subprocess.run(cmd, cwd=chdir, env=genv, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=3600)
        (EV / f"gate-{key}.log").write_text(p.stdout)
        keep = [l for l in p.stdout.splitlines() if l.startswith(("test ", "test result", "error", "warning", "    Finished")) or "panicked" in l]
        record("GATES", key, {"cmd": " ".join(cmd), "rc": p.returncode, "wall_s": round(time.time() - t0, 1), "summary": keep[-150:]})
        log(f"GATE {key} rc={p.returncode}")


def p10():
    """pause/resume + snapshot refusal on a live vhost-kernel VM."""
    vm = VM("p10", PATCHED_CH, "cid=44,backend=vhost-kernel", api=True)
    assert vm.wait_serial(r"AGENT_READY")
    remote = [CH_REMOTE, "--api-socket", str(vm.api)]
    record("P10", "h2g_before", probe("client", "vsock:44:5000", "stream", 1048576, "before-pause").strip())
    record("P10", "pause", sh(remote + ["pause"], check=False).strip())
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(3)
    try:
        s.connect((44, 4000))
        s.sendall(b"ident\\n")
        paused_reply = s.recv(200).decode(errors="replace")
    except OSError as e:
        paused_reply = f"error {e!r}"
    record("P10", "ctl_while_paused_3s", paused_reply)
    snapdir = OUT / "p10-snapshot"
    snapdir.mkdir(exist_ok=True)
    record("P10", "snapshot_attempt", sh(remote + ["snapshot", f"file://{snapdir}"], check=False).strip())
    record("P10", "snapshot_dir_contents", sorted(x.name for x in snapdir.iterdir()))
    record("P10", "ch_log_snapshot_lines", sh(f"grep -iE 'snapshot|vhost-kernel' {vm.dir / 'ch.log'} | tail -8 || true", check=False, quiet=True))
    record("P10", "resume", sh(remote + ["resume"], check=False).strip())
    tail = b""
    try:
        s.settimeout(10)
        if paused_reply.startswith("error"):
            s.close()
        else:
            while b"END" not in tail:
                chunk = s.recv(200)
                if not chunk:
                    break
                tail += chunk
    except OSError as e:
        tail += f" error {e!r}".encode()
    record("P10", "ctl_reply_after_resume", (paused_reply + tail.decode(errors="replace")).strip())
    s.close()
    record("P10", "ident_after_resume", wait_ident(44, 20))
    record("P10", "h2g_after_resume", probe("client", "vsock:44:5000", "stream", 1048576, "after-resume").strip())
    record("P10", "h2g_seq_after_resume", probe("client", "vsock:44:5001", "seqpacket", 0, "after-resume-seq").strip())
    record("P10", "ch_alive", vm.proc.poll() is None)
    probe("ctl", "vsock:44:4000", "poweroff")
    record("P10", "exit", vm.wait_exit(30))


run_probe("P10", p10)
run_probe("GATES", gates)
run_probe("P7d", p7d)'''.replace('"LINT".split()', f'"{CARGO_LINT}".split()')
anchor = 'run_probe("P7d", p7d)'
assert anchor in s
s = s.replace(anchor, ADD, 1)
p.write_text(s)
print("increment-d/run.py derived")
