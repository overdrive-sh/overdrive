#!/usr/bin/env python3
"""v30 guest-reset probe: what happens to the VMM, to host vsock flows and to the CID when the guest
resets itself, with Cloud Hypervisor (a) opening /dev/vhost-vsock itself ("stock") and (b) receiving a
pre-claimed fd ("fd", V-25 fd-mode prototype with its once-only guard).

Runs on the metal host as root, under the canonical lease (launched by launch.py), in /tmp/v30-work only.

  run.py image
  run.py case <mode> <case> [--cmdline-extra STR] [--tag TAG]
  run.py matrix                # all modes x cases
  run.py sup <n>               # supervisor-kills-on-'rebooting'-event experiment, n trials

mode: stock | fd-keep (launcher keeps its copy of the fd, as ADR-0170 serve does) | fd-close (launcher closes after spawn)
case: pid1exit | sysrq-c | reboot | rebootf | sysrq-b | triple | triple-panic | poweroff (control)
"""
import sys; sys.dont_write_bytecode = True
import errno, fcntl, json, os, pathlib, re, select, shutil, signal, socket, subprocess, threading, time

W = pathlib.Path("/tmp/v30-work"); OUT = W / "out"
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
VHOST_VSOCK_SET_GUEST_CID = 0x4008AF60
HOST_PORT, GUEST_PORT = 6000, 5000
EVROOT = W / "evidence"; EVROOT.mkdir(parents=True, exist_ok=True)
_cid_counter = [3700]


def now(): return time.monotonic()


def sh(cmd, timeout=120):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, errors="replace")
    return p.returncode, p.stdout


# ---------------------------------------------------------------- substrate / image
def preflight():
    fail = []
    if os.uname().machine != "x86_64": fail.append("not x86_64")
    virt = subprocess.run(["systemd-detect-virt"], text=True, capture_output=True)
    if virt.returncode != 1 or virt.stdout.strip() != "none": fail.append(f"systemd-detect-virt={virt.stdout.strip()!r}")
    cpu = pathlib.Path("/proc/cpuinfo").read_text()
    if not re.search(r"^flags.*\b(vmx|svm)\b", cpu, re.M): fail.append("no vmx/svm")
    try:
        import stat
        if not stat.S_ISCHR(os.stat("/dev/kvm").st_mode): fail.append("/dev/kvm not chardev")
        k = os.open("/dev/kvm", os.O_RDWR); api = fcntl.ioctl(k, 0xAE00); v = fcntl.ioctl(k, 0xAE01, 0); os.close(v); os.close(k)
        if api != 12: fail.append(f"kvm api {api}")
    except Exception as e:
        fail.append(f"kvm probe {e!r}")
    info = {"host_uname_r": os.uname().release, "machine": os.uname().machine, "systemd_detect_virt": virt.stdout.strip(),
            "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "failures": fail}
    print("SUBSTRATE", json.dumps(info), flush=True)
    if fail: sys.exit(3)
    return info


def build_image():
    root = OUT / "initramfs-root"
    if root.exists(): shutil.rmtree(root)
    for d in ["proc", "sys", "dev", "bin", "sbin", "tmp", "mods", "etc"]:
        (root / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2("/usr/bin/busybox", root / "bin/busybox")
    for n in ["sh", "mount", "insmod", "cat", "echo", "uname", "sleep", "rm", "mkdir", "touch", "grep", "init", "reboot", "poweroff", "halt", "true", "mv", "date"]:
        (root / "bin" / n).symlink_to("busybox")
    shutil.copy2(OUT / "v30guest", root / "v30guest")
    (root / "etc/inittab").write_text("::sysinit:/etc/guest-run.sh\n::ctrlaltdel:/bin/reboot\n::shutdown:/bin/true\n")
    (root / "etc/guest-run.sh").write_text("""#!/bin/sh
export PATH=/bin:/sbin
/v30guest > /dev/console 2>&1 &
# watcher: act on the case the host sent over vsock ("GO <case>" -> /tmp/go)
(
  while :; do
    if [ -e /tmp/go ]; then
      c=$(cat /tmp/go); rm -f /tmp/go
      echo "GUEST_ACT $c"
      case "$c" in
        pid1exit) touch /tmp/exit-init ;;
        sysrq-c|triple-panic) echo c > /proc/sysrq-trigger ;;
        sysrq-b) echo b > /proc/sysrq-trigger ;;
        reboot) reboot ;;
        rebootf|triple) reboot -f ;;
        poweroff) poweroff -f ;;
      esac
    fi
    sleep 0.02
  done
) > /dev/console 2>&1 &
""")
    (root / "etc/guest-run.sh").chmod(0o755)
    (root / "init").write_text("""#!/bin/sh
export PATH=/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mount -t tmpfs tmpfs /tmp
echo 1 > /proc/sys/kernel/sysrq
echo "GUEST_INIT uname_r=$(uname -r) cmdline=$(cat /proc/cmdline)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
if grep -q v30crash=1 /proc/cmdline; then echo "GUEST_PID1_CRASHING_AT_BOOT"; exit 0; fi
if grep -q v30init=busybox /proc/cmdline; then exec /bin/busybox init; fi
/etc/guest-run.sh
# PID 1 = this shell. 'pid1exit' makes it exit, which panics the guest kernel ("Attempted to kill init!").
while :; do
  if [ -e /tmp/exit-init ]; then echo "GUEST_PID1_EXITING"; exit 0; fi
  sleep 0.02
done
""")
    (root / "init").chmod(0o755)
    order = []
    for line in sh(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"])[1].splitlines():
        if line.startswith("insmod "):
            src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
            if name in order: continue
            data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
            (root / "mods" / name).write_bytes(data); order.append(name)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {OUT}/initramfs.cpio.gz")
    print("IMAGE built; modules:", order, flush=True)


# ---------------------------------------------------------------- vsock helpers
def vs_listen(port):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.bind((socket.VMADDR_CID_ANY, port)); s.listen(64); s.setblocking(False); return s


def claim(cid):
    """open /dev/vhost-vsock and claim a CID; returns (fd, None) or (None, errno name)"""
    fd = os.open("/dev/vhost-vsock", os.O_RDWR | os.O_CLOEXEC)
    try:
        fcntl.ioctl(fd, VHOST_VSOCK_SET_GUEST_CID, __import__("struct").pack("<Q", cid))
        return fd, None
    except OSError as e:
        os.close(fd); return None, errno.errorcode.get(e.errno, str(e.errno))


class Conn:
    """a held host-side vsock connection, polled non-blocking"""
    def __init__(self, name, s, inc=None):
        self.name, self.s, self.inc, self.buf, self.pings = name, s, inc, b"", 0
        self.state, self.t_end, self.detail = "open", None, None
        s.setblocking(False)

    def poll(self, t0):
        if self.state != "open": return
        try:
            while True:
                d = self.s.recv(65536, socket.MSG_DONTWAIT)
                if d == b"":
                    self.state, self.t_end, self.detail = "eof", now() - t0, "recv()==0 (orderly EOF)"; return
                self.buf += d
                self.pings += d.count(b"PING")
        except BlockingIOError:
            pass
        except OSError as e:
            self.state, self.t_end, self.detail = "error", now() - t0, f"recv: {errno.errorcode.get(e.errno, e.errno)}"
            return
        err = self.s.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR)
        if err:
            self.state, self.t_end, self.detail = "error", now() - t0, f"SO_ERROR={errno.errorcode.get(err, err)}"

    def probe_send(self, t0):
        """after the connection looks open: does a send fail?"""
        try:
            self.s.send(b"x", socket.MSG_NOSIGNAL | socket.MSG_DONTWAIT); return "send ok"
        except OSError as e:
            return f"send: {errno.errorcode.get(e.errno, e.errno)}"

    def summary(self):
        return {"name": self.name, "incarnation": self.inc, "state": self.state, "t_end_s": None if self.t_end is None else round(self.t_end, 3), "detail": self.detail, "pings_rx": self.pings}


def ss_vsock():
    rc, out = sh("ss --vsock -a -n -H 2>&1")
    return [l for l in out.splitlines() if l.strip()]


def ch_vhost_fds(pid):
    res = {}
    try:
        for fd in os.listdir(f"/proc/{pid}/fd"):
            try:
                tgt = os.readlink(f"/proc/{pid}/fd/{fd}")
            except OSError: continue
            if "vhost-vsock" in tgt: res[fd] = tgt
    except OSError as e:
        return {"error": str(e)}
    return res


def read_events(path, pos):
    """the CH event monitor writes pretty-printed JSON objects separated by blank lines (not JSONL)"""
    try:
        with open(path, "rb") as f:
            f.seek(pos); data = f.read().decode(errors="replace")
    except FileNotFoundError:
        return [], pos
    dec, i, out = json.JSONDecoder(), 0, []
    while True:
        while i < len(data) and data[i] in " \r\n\t": i += 1
        if i >= len(data): break
        try:
            obj, j = dec.raw_decode(data, i)
        except ValueError:
            break
        out.append(obj); i = j
    return out, pos + len(data[:i].encode())


# ---------------------------------------------------------------- one scenario
def run_case(mode, case, tag="", supervisor=False, observe_s=22.0):
    cid = _cid_counter[0]; _cid_counter[0] += 1
    name = f"v30-{mode}-{case}{('-' + tag) if tag else ''}-{cid}-{STAMP[-6:]}"
    ev = EVROOT / name; ev.mkdir(parents=True, exist_ok=False)
    timeline = []
    def rec(kind, **kw):
        timeline.append({"t_s": None if T0[0] is None else round(now() - T0[0], 4), "wall": time.time(), "ev": kind, **kw})
    T0 = [None]
    cmdline = "console=ttyS0 panic=1 loglevel=4"
    if case in ("triple", "triple-panic"): cmdline += " reboot=t"
    if case == "reboot": cmdline += " v30init=busybox"
    ch_bin = OUT / ("ch-stock" if mode == "stock" else "ch-fd")
    listener = vs_listen(HOST_PORT)
    launcher_fd = None
    vsock_arg = f"cid={cid},backend=vhost-kernel"
    pass_fds = ()
    if mode != "stock":
        launcher_fd, err = claim(cid)
        if err: return {"name": name, "error": f"claim {err}"}
        vsock_arg += f",fd={launcher_fd}"; pass_fds = (launcher_fd,)
    serial, evfile = ev / "serial.log", ev / "events.jsonl"
    sup_r = sup_w = None
    evmon = f"path={evfile}"
    if supervisor:
        sup_r, sup_w = os.pipe(); evmon = f"fd={sup_w}"; pass_fds = tuple(pass_fds) + (sup_w,)
    argv = [str(ch_bin), "--kernel", KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", cmdline, "--cpus", "boot=1", "--memory", "size=256M",
            "--serial", f"file={serial}", "--console", "off", "--vsock", vsock_arg, "--log-file", str(ev / "ch.log"), "-v", "--event-monitor", evmon]
    (ev / "cmd.txt").write_text(" ".join(argv) + "\n")
    proc = subprocess.Popen(argv, stdout=(ev / "ch.stdout").open("w"), stderr=subprocess.STDOUT, start_new_session=True, pass_fds=pass_fds)
    if mode == "fd-close":
        os.close(launcher_fd); launcher_fd = None
    if sup_w is not None: os.close(sup_w)
    spawn_t = now()
    result = {"name": name, "mode": mode, "case": case, "cid": cid, "cmdline": cmdline, "vsock_arg": vsock_arg, "ch_pid": proc.pid}
    held, accepted = [], []
    attempts, attempts_lock = [], threading.Lock()
    serial_pos, serial_acc = 0, bytearray()
    ev_pos, ev_lines = 0, []
    killed_by_supervisor = [None]

    def sup_thread():
        # supervisor: blocking read of the event-monitor pipe; SIGKILL the VMM on the first 'rebooting' event
        dec, buf = json.JSONDecoder(), ""
        while True:
            try: d = os.read(sup_r, 65536)
            except OSError: return
            if not d: return
            t_rx = now(); buf += d.decode(errors="replace")
            while True:
                buf = buf.lstrip()
                if not buf: break
                try: obj, j = dec.raw_decode(buf)
                except ValueError: break
                buf = buf[j:]
                ev_lines.append({"host_t": t_rx, **obj})
                if obj.get("event") == "rebooting" and killed_by_supervisor[0] is None:
                    killed_by_supervisor[0] = now()
                    try: os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError: pass
                    killed_by_supervisor.append(now() - t_rx)
    if supervisor: threading.Thread(target=sup_thread, daemon=True).start()

    def pump():
        nonlocal serial_pos, ev_pos
        try:
            sz = serial.stat().st_size
            if sz < serial_pos: serial_acc.extend(b"\n==== SERIAL FILE TRUNCATED (VM re-created) ====\n"); serial_pos = 0
            if sz > serial_pos:
                with serial.open("rb") as f: f.seek(serial_pos); d = f.read()
                serial_pos += len(d); serial_acc.extend(d)
        except FileNotFoundError: pass
        if supervisor: return
        new, ev_pos = read_events(evfile, ev_pos)
        for e in new:
            ev_lines.append({"host_t": now(), **e})

    def accept_new():
        while True:
            try: c, _ = listener.accept()
            except BlockingIOError: return
            c.setblocking(True); c.settimeout(2.0)
            try: hello = c.recv(256).decode(errors="replace").strip().split("\n")[0]
            except Exception as e: hello = f"ERR {e!r}"
            inc = hello.split()[1] if hello.startswith("HELLO") else None
            conn = Conn(f"g2h#{len(accepted)+1}", c, inc); accepted.append(conn); held.append(conn)
            rec("host_accepted_guest_connection", incarnation=inc, hello=hello)

    def attempt_connect(label):
        t_a = now() - T0[0]
        s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(1.5)
        r = {"label": label, "t_start_s": round(t_a, 3)}
        try:
            s.connect((cid, GUEST_PORT)); d = s.recv(128).decode(errors="replace").strip(); r.update(result="connected", greeting=d)
        except OSError as e:
            r.update(result=f"{errno.errorcode.get(e.errno, e.errno)}" if e.errno else repr(e))
        except Exception as e:
            r.update(result=repr(e))
        r["t_end_s"] = round(now() - T0[0], 3); s.close()
        with attempts_lock: attempts.append(r)

    try:
        # ---- wait for incarnation #1's outbound connection
        end = now() + 60
        while now() < end and not accepted:
            if proc.poll() is not None: break
            T0[0] = T0[0] or now(); accept_new(); pump(); time.sleep(0.05)
        T0[0] = now()
        if not accepted:
            result.update(error="guest never connected", ch_rc=proc.poll(), serial=bytes(serial_acc).decode(errors="replace")[-3000:]); return result
        inc1 = accepted[0].inc
        # ---- host -> guest connection to incarnation #1
        s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(10)
        for _ in range(60):
            try: s.connect((cid, GUEST_PORT)); break
            except OSError: time.sleep(0.25); s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(10)
        greet = s.recv(128).decode(errors="replace").strip(); connB = Conn("h2g#1", s, greet.split()[-1] if greet else None); held.append(connB)
        rec("host_connected_to_guest", greeting=greet)
        time.sleep(1.0)
        for c in held: c.poll(T0[0])
        result["before"] = {"incarnation_1": inc1, "held": [c.summary() for c in held], "ss_vsock": ss_vsock(), "ch_vhost_fds": ch_vhost_fds(proc.pid),
                            "claim_attempt_from_host_probe": None}
        # a second claim of the same CID (should be EADDRINUSE in both modes)
        fd2, err2 = claim(cid)
        if fd2 is not None: os.close(fd2)
        result["before"]["claim_attempt_from_host_probe"] = err2 or "OK(!)"
        # ---- trigger
        T0[0] = now(); rec("GO", case=case)
        accepted[0].s.setblocking(True); accepted[0].s.sendall(f"GO {case}\n".encode()); accepted[0].s.setblocking(False)
        tn = 0.0; last_attempt = -1.0; ch_exit_t = None; second_inc_t = None; threads = []
        while True:
            el = now() - T0[0]
            if el > observe_s: break
            accept_new(); pump()
            for c in held: c.poll(T0[0])
            if ch_exit_t is None and proc.poll() is not None:
                ch_exit_t = el; rec("ch_exited", rc=proc.returncode)
            if second_inc_t is None and any(c.inc and c.inc != inc1 for c in accepted):
                second_inc_t = el
            if el - last_attempt >= 0.3 and (ch_exit_t is None) and el > 0.2:
                last_attempt = el; th = threading.Thread(target=attempt_connect, args=(f"t={el:.1f}",), daemon=True); th.start(); threads.append(th)
            # early finish
            if second_inc_t is not None and el - second_inc_t > 3.0: break
            if ch_exit_t is not None and el - ch_exit_t > 7.0: break
            if killed_by_supervisor[0] is not None and now() - killed_by_supervisor[0] > 5.0: break
            time.sleep(0.02)
        for th in threads: th.join(timeout=3)
        pump()
        # ---- final state
        result["after"] = {"held": [c.summary() for c in held], "send_probe_on_open_conns": {c.name: c.probe_send(T0[0]) for c in held if c.state == "open"},
                           "ss_vsock": ss_vsock(), "ch_alive": proc.poll() is None, "ch_rc": proc.poll(), "ch_vhost_fds": ch_vhost_fds(proc.pid) if proc.poll() is None else None}
        if mode != "stock":
            fd3, err3 = claim(cid)
            result["after"]["claim_cid_while_launcher_holds" if launcher_fd is not None else "claim_cid_after_vmm_exit"] = err3 or "OK"
            if fd3 is not None: os.close(fd3)
            if mode == "fd-keep" and ch_exit_t is not None:
                # the launcher is still holding its copy: are the host flows still ESTABLISHED to a dead guest?
                for c in held: c.poll(T0[0])
                result["after"]["fd_keep_flows_before_launcher_closes"] = [c.summary() for c in held]
                result["after"]["fd_keep_ss_before_launcher_closes"] = ss_vsock()
                t_close = now() - T0[0]; os.close(launcher_fd); launcher_fd = None
                time.sleep(0.5)
                for c in held: c.poll(T0[0])
                result["after"]["launcher_closed_at_s"] = round(t_close, 3)
                result["after"]["fd_keep_flows_after_launcher_closes"] = [c.summary() for c in held]
                fd4, err4 = claim(cid); result["after"]["claim_cid_after_launcher_close"] = err4 or "OK"
                if fd4 is not None: os.close(fd4)
        result.update(ch_exit_t_s=None if ch_exit_t is None else round(ch_exit_t, 3), ch_rc=proc.poll(), second_incarnation_first_hello_s=None if second_inc_t is None else round(second_inc_t, 3),
                      incarnations_seen=sorted({c.inc for c in accepted if c.inc} | {inc1}), connect_attempts=attempts, supervisor_kill_at_s=None if killed_by_supervisor[0] is None else round(killed_by_supervisor[0] - T0[0], 4),
                      supervisor_kill_latency_after_event_rx_ms=None if len(killed_by_supervisor) < 2 else round(killed_by_supervisor[1] * 1000, 3))
        evs = [{"event": e.get("event"), "source": e.get("source"), "ch_ts": e.get("timestamp"), "host_t_s": round(e["host_t"] - T0[0], 4)} for e in ev_lines if e.get("source") in ("vm", "virtio-device")]
        result["ch_events"] = evs
        reb = [e for e in ev_lines if e.get("event") == "rebooting"]; rbd = [e for e in ev_lines if e.get("event") == "rebooted"]
        if reb and rbd:
            ts = lambda e: e["timestamp"]["secs"] + e["timestamp"]["nanos"] / 1e9
            result["ch_rebooting_to_rebooted_s"] = round(ts(rbd[0]) - ts(reb[0]), 4)
        result["timeline"] = timeline
        return result
    finally:
        pump()
        try: os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: proc.wait(timeout=10)
        except Exception: pass
        if launcher_fd is not None: os.close(launcher_fd)
        for c in held:
            try: c.s.close()
            except Exception: pass
        listener.close()
        (ev / "serial-accumulated.log").write_bytes(bytes(serial_acc))
        (ev / "result.json").write_text(json.dumps(result, indent=1, default=str))
        time.sleep(0.3)


def compact(r):
    if "error" in r:
        print(f"## {r.get('name')}: ERROR {r['error']}", flush=True); return
    a = r.get("after", {})
    held = {c["name"]: f"{c['state']}@{c['t_end_s']}s {c['detail']}" for c in a.get("held", [])}
    atts = r.get("connect_attempts", [])
    ok_new = [x for x in atts if x.get("result") == "connected" and x.get("greeting", "").split()[-1:] != [r["before"]["incarnation_1"]]]
    print(f"## {r['name']}: vmm_exit={'t=%ss rc=%s' % (r['ch_exit_t_s'], r['ch_rc']) if r['ch_exit_t_s'] is not None else 'NO (still running at end)'}"
          f" | 2nd_incarnation_hello={r['second_incarnation_first_hello_s']}s | incarnations={len(r['incarnations_seen'])}"
          f" | old_flows={held} | host_listener_accepted_new_inc={'yes' if r['second_incarnation_first_hello_s'] is not None else 'no'}"
          f" | h2g_connect_results={sorted({x['result'] for x in atts})} | new_inc_via_h2g={len(ok_new)}"
          f" | rebooting->rebooted={r.get('ch_rebooting_to_rebooted_s')} sup_kill={r.get('supervisor_kill_at_s')} kill_lat_ms={r.get('supervisor_kill_latency_after_event_rx_ms')} ch_events={[e['event'] for e in r.get('ch_events', [])]}", flush=True)
    if r["mode"] == "fd-keep":
        print(f"     fd-keep: flows before launcher closes={[(c['name'], c['state']) for c in a.get('fd_keep_flows_before_launcher_closes', [])]} after={[(c['name'], c['state'], c['detail']) for c in a.get('fd_keep_flows_after_launcher_closes', [])]} claim_after_vmm_exit={a.get('claim_cid_while_launcher_holds')} claim_after_launcher_close={a.get('claim_cid_after_launcher_close')}", flush=True)
    elif r["mode"] == "fd-close":
        print(f"     fd-close: claim_after_vmm_exit={a.get('claim_cid_after_vmm_exit')}", flush=True)


def run_loop(mode, secs=15.0):
    """guest PID 1 exits on EVERY boot (v30crash=1): does the VMM ever exit, or does the guest reboot-loop in place?"""
    cid = _cid_counter[0]; _cid_counter[0] += 1
    name = f"v30-loop-{mode}-{cid}-{STAMP[-6:]}"; ev = EVROOT / name; ev.mkdir(parents=True, exist_ok=False)
    ch_bin = OUT / ("ch-stock" if mode == "stock" else "ch-fd")
    launcher_fd, pass_fds, vsock_arg = None, (), f"cid={cid},backend=vhost-kernel"
    if mode != "stock":
        launcher_fd, err = claim(cid); vsock_arg += f",fd={launcher_fd}"; pass_fds = (launcher_fd,)
    serial, evfile = ev / "serial.log", ev / "events.jsonl"
    argv = [str(ch_bin), "--kernel", KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", "console=ttyS0 panic=1 loglevel=4 v30crash=1", "--cpus", "boot=1", "--memory", "size=256M",
            "--serial", f"file={serial}", "--console", "off", "--vsock", vsock_arg, "--log-file", str(ev / "ch.log"), "-v", "--event-monitor", f"path={evfile}"]
    t0 = now(); proc = subprocess.Popen(argv, stdout=(ev / "ch.stdout").open("w"), stderr=subprocess.STDOUT, start_new_session=True, pass_fds=pass_fds)
    samples, exit_t = [], None
    while now() - t0 < secs:
        if proc.poll() is not None: exit_t = now() - t0; break
        evs, _ = read_events(evfile, 0)
        samples.append((round(now() - t0, 2), sum(1 for e in evs if e.get("event") == "rebooted")))
        time.sleep(1.0)
    evs, _ = read_events(evfile, 0)
    res = {"name": name, "mode": mode, "secs_observed": round(now() - t0, 2), "vmm_exited": exit_t is not None, "vmm_exit_t_s": exit_t, "rc": proc.poll(),
           "rebooted_events": sum(1 for e in evs if e.get("event") == "rebooted"), "samples_t_rebooted_count": samples, "vhost_fds": ch_vhost_fds(proc.pid) if proc.poll() is None else None,
           "cpu_s_ch": None}
    try:
        st = pathlib.Path(f"/proc/{proc.pid}/stat").read_text().split(")")[-1].split(); res["cpu_s_ch"] = (int(st[11]) + int(st[12])) / os.sysconf("SC_CLK_TCK")
    except Exception: pass
    try: os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError: pass
    proc.wait(timeout=10)
    if launcher_fd is not None: os.close(launcher_fd)
    (ev / "result.json").write_text(json.dumps(res, indent=1))
    print("## LOOP", json.dumps({k: v for k, v in res.items() if k != "samples_t_rebooted_count"}), "samples", samples, flush=True)
    return res


CASES = ["pid1exit", "sysrq-c", "reboot", "rebootf", "sysrq-b", "triple", "triple-panic", "poweroff"]
MODES = ["stock", "fd-keep", "fd-close"]


def main():
    stage = sys.argv[1]
    info = preflight()
    (EVROOT / f"substrate-{STAMP}.json").write_text(json.dumps(info, indent=1))
    if stage == "image":
        build_image(); return
    if stage == "case":
        mode, case = sys.argv[2], sys.argv[3]
        r = run_case(mode, case, tag=(sys.argv[4] if len(sys.argv) > 4 else "")); compact(r); return
    if stage == "matrix":
        modes = sys.argv[2].split(",") if len(sys.argv) > 2 else MODES
        cases = sys.argv[3].split(",") if len(sys.argv) > 3 else CASES
        allr = []
        for m in modes:
            for c in cases:
                r = run_case(m, c); allr.append(r); compact(r)
        (EVROOT / f"matrix-{STAMP}.json").write_text(json.dumps(allr, indent=1, default=str))
        return
    if stage == "loop":
        for m in (sys.argv[2].split(",") if len(sys.argv) > 2 else ["stock", "fd-keep"]): run_loop(m)
        return
    if stage == "sup":
        n = int(sys.argv[2]) if len(sys.argv) > 2 else 10
        case = sys.argv[3] if len(sys.argv) > 3 else "rebootf"
        load = int(os.environ.get("V30_LOAD", "0")); hogs = []
        for _ in range(load):
            hogs.append(subprocess.Popen([sys.executable, "-c", "while True: pass"], start_new_session=True))
        if hogs: print(f"LOAD: {load} busy-loop processes on {os.cpu_count()} cpus", flush=True); time.sleep(1)
        try:
            for i in range(n):
                r = run_case("stock", case, tag=f"sup{'L' if load else ''}{i}", supervisor=True, observe_s=12.0); compact(r)
        finally:
            for h in hogs:
                try: os.killpg(h.pid, signal.SIGKILL)
                except ProcessLookupError: pass
        return


main()
