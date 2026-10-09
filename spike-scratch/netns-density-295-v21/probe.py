#!/usr/bin/env python3
"""V-21 probe (throwaway). Runs INSIDE netns v21-host (as root). See run.sh for topology.
Oracle for "reached a host socket" = the server-side hit counter, not the client's view."""
import socket, struct, subprocess, threading, time, errno, json, sys, os, collections

OWNER = 0x4          # stand-in for the owner egress mark (value is DELIVER's; must differ from 0x1 TPROXY / 0x2 leg-S)
GW = "100.95.0.1"; HOSTIP = "10.77.21.1"; PEER = "10.77.21.2"
W, L, LEGF = 22022, 22023, 22024      # wildcard "sshd", loopback-only admin, leg-F stand-in
PEERLOG = "/tmp/v21-peer.log"
hits = collections.Counter(); hl = threading.Lock()
def hit(k):
    with hl: hits[k] += 1
def peer_hits(tag):
    try: return sum(1 for l in open(PEERLOG) if l.strip() == tag)
    except FileNotFoundError: return 0

def serve_tcp(addr, port, tag):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((addr, port)); s.listen(512)
    def loop():
        while True:
            try: c, _ = s.accept()
            except OSError: return
            hit(tag + ":accept")
            try:
                c.settimeout(0.5); d = c.recv(16)
                if d: hit(tag + ":data"); c.sendall(b"R")
            except OSError: pass
            c.close()
    threading.Thread(target=loop, daemon=True).start(); return s
def serve_udp(addr, port, tag):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    s.bind((addr, port))
    def loop():
        while True:
            try: d, a = s.recvfrom(70000)
            except OSError: return
            hit(tag + ":dgram")
            try: s.sendto(b"R", a)
            except OSError: pass
    threading.Thread(target=loop, daemon=True).start(); return s

def sh(cmd, check=True):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    if check and r.returncode: raise RuntimeError(f"{cmd}: {r.stderr}")
    return r.stdout

def tcp_try(dst, port, mark=None, bind_dev=None, linger=True, fam=socket.AF_INET, tmo=1.0):
    s = socket.socket(fam, socket.SOCK_STREAM)
    if mark is not None: s.setsockopt(socket.SOL_SOCKET, socket.SO_MARK, mark)
    if bind_dev: s.setsockopt(socket.SOL_SOCKET, socket.SO_BINDTODEVICE, bind_dev.encode())
    if linger: s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    s.settimeout(tmo); t0 = time.monotonic(); out = {"connect": "ok", "data": False}
    try:
        s.connect((dst, port))
        try:
            s.sendall(b"Q"); out["data"] = (s.recv(1) == b"R")
        except OSError as e: out["io"] = errno.errorcode.get(e.errno, str(e))
    except OSError as e:
        out["connect"] = errno.errorcode.get(e.errno, "TIMEOUT" if isinstance(e, socket.timeout) else str(e))
    out["ms"] = round((time.monotonic() - t0) * 1000, 1); s.close(); return out

def udp_try(dst, port, mark=None, size=1, bcast=False, bind_dev=None, sport=None, fam=socket.AF_INET, tmo=0.3):
    s = socket.socket(fam, socket.SOCK_DGRAM)
    if mark is not None: s.setsockopt(socket.SOL_SOCKET, socket.SO_MARK, mark)
    if bcast: s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    if bind_dev: s.setsockopt(socket.SOL_SOCKET, socket.SO_BINDTODEVICE, bind_dev.encode())
    if sport: s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("0.0.0.0", sport))
    s.settimeout(tmo); out = {"connect": "ok", "send": "ok", "reply": False}
    try:
        s.connect((dst, port))
        try:
            s.send(b"Q" * size)
            try: out["reply"] = (s.recv(4) == b"R")
            except socket.timeout: pass
            except OSError as e: out["recv"] = errno.errorcode.get(e.errno, str(e))
            try: s.send(b"Q")           # a pending ICMP error surfaces here
            except OSError as e: out["send2"] = errno.errorcode.get(e.errno, str(e))
        except OSError as e: out["send"] = errno.errorcode.get(e.errno, str(e))
    except OSError as e: out["connect"] = errno.errorcode.get(e.errno, str(e))
    s.close(); return out

results = []; fails = []
def rec(phase, name, claim, got, expect_reach, reached, extra=""):
    ok = (reached == expect_reach)
    results.append((phase, name, ok))
    if not ok: fails.append((phase, name))
    print(f"[{phase}] {'PASS' if ok else 'FAIL'} {name:58s} reached_host_socket={reached!s:5} (expected {expect_reach!s:5}) {got} {extra}", flush=True)

def delta(tag, fn, peer=False):
    time.sleep(0.02)
    b = peer_hits(tag) if peer else sum(v for k, v in hits.items() if k.startswith(tag + ":"))
    r = fn(); time.sleep(0.12)
    a = peer_hits(tag) if peer else sum(v for k, v in hits.items() if k.startswith(tag + ":"))
    return r, a - b

def nft_install(rules_variant):
    sh("nft delete table ip v21-mtls 2>/dev/null; true", check=False)
    sh(f"nft -f - <<'EOF'\n{rules_variant}\nEOF")

def ruleset(order="d26-first", reject="reject", gw_exc=True, localexpr="fib daddr type local", owner_expr=f"meta mark 0x{OWNER:x}", ctaccept=False, prefix=False):
    exc = (f"{owner_expr} ip daddr {GW} udp dport 53 counter accept\n"
           f"    {owner_expr} ip daddr {GW} tcp dport 53 counter accept\n") if gw_exc else ""
    d26 = (f"{owner_expr} ip daddr {{ 127.0.0.0/8, 169.254.0.0/16 }} counter {reject}\n"
           f"    {owner_expr} {localexpr} counter {reject}\n")
    ct = "ct state established,related counter accept\n    " if ctaccept else ""
    pre = ("meta mark 0x2 counter accept\n    ct state established,related accept\n" if prefix else "")
    pfx = ("ip daddr 100.95.0.0/16 ip daddr != 100.95.0.1 counter reject\n" if prefix else "")
    body = {"d26-first": f"{exc}    {d26}    {ct}{pre}    {pfx}",
            "ct-first":  f"{ct}{exc}    {d26}    {pre}    {pfx}"}[order]
    return f"""table ip v21-mtls {{
  chain output {{
    type filter hook output priority filter; policy accept;
    {body}
  }}
  chain input {{
    type filter hook input priority filter; policy accept;
    meta mark 0x{OWNER:x} counter comment "owner-marked packet reached input"
  }}
}}"""

def rule_pkts(substr):
    for l in sh("nft list table ip v21-mtls").splitlines():
        if substr in l and "counter packets" in l:
            return int(l.split("counter packets")[1].split()[0])
    return -1

def counters():
    return sh("nft list table ip v21-mtls")

# ---------------- servers
servers = []
servers += [serve_tcp("0.0.0.0", W, "W-tcp"), serve_udp("0.0.0.0", W, "W-udp")]
servers += [serve_tcp("127.0.0.1", L, "L-tcp"), serve_udp("127.0.0.1", L, "L-udp")]
servers += [serve_tcp(GW, 53, "DNS-tcp"), serve_udp(GW, 53, "DNS-udp")]
servers += [serve_tcp("127.0.0.1", LEGF, "LEGF-tcp")]

# (name, dst, port, server-tag, local?)  -- every case: TCP and UDP
LOCAL_CASES = [
 ("127.0.0.1:L loopback-only service",    "127.0.0.1",   L,  "L"),
 ("127.0.0.1:W wildcard service",         "127.0.0.1",   W,  "W"),
 ("127.0.0.2:W (rest of 127/8)",          "127.0.0.2",   W,  "W"),
 ("host veth address 10.77.21.1:W",       HOSTIP,        W,  "W"),
 (f"gateway {GW}:W (not 53)",             GW,            W,  "W"),
 ("guest-prefix 100.95.0.7:W (local route)", "100.95.0.7", W, "W"),
 ("169.254.1.1:W (local link-local)",     "169.254.1.1", W,  "W"),
 ("192.0.2.55:W (operator-added local addr)", "192.0.2.55", W, "W"),
 ("0.0.0.0:W (kernel maps to loopback)",  "0.0.0.0",     W,  "W"),
]
PEER_CASES = [  # off-host: static 169.254/16 must be denied even though NOT local; others allowed
 ("169.254.169.254:80 metadata (off-host)", "169.254.169.254", 80, "169.254.169.254:80", False),
 ("198.51.100.7:80 off-host public",        "198.51.100.7",    80, "198.51.100.7:80",    True),
 ("peer 10.77.21.2:W off-host neighbour",   PEER,              W,  "10.77.21.2:22022",   True),
]
def reached_tcp(r): return r["data"] or r["connect"] == "ok"

def run_matrix(phase, expect_denied_marked):
    for name, dst, port, st in LOCAL_CASES:
        for proto in ("tcp", "udp"):
            tag = f"{st}-{proto}"
            if proto == "tcp":
                r, d = delta(tag, lambda: tcp_try(dst, port, OWNER))
            else:
                r, d = delta(tag, lambda: udp_try(dst, port, OWNER))
            rec(phase, f"marked {proto} -> {name}", "", r, not expect_denied_marked, d > 0)
    for name, dst, port, tag, allowed in PEER_CASES:
        for proto in ("tcp", "udp"):
            t = f"{proto} {tag}"
            fn = (lambda: tcp_try(dst, port, OWNER)) if proto == "tcp" else (lambda: udp_try(dst, port, OWNER))
            r, d = delta(t, fn, peer=True)
            exp = True if not expect_denied_marked else allowed
            rec(phase, f"marked {proto} -> {name}", "", r, exp, d > 0)

def main():
    which = sys.argv[1:] or ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7"]
    print("uname -r:", os.uname().release, os.uname().machine)
    print("nft:", sh("nft --version").strip())
    print("ip rule:", sh("ip rule").replace("\n", " | "))
    print("local table:", sh("ip route show table local").replace("\n", " | "))
    sh("nft delete table ip v21-mtls 2>/dev/null; true", check=False)

    if "p0" in which:
        print("\n=== P0 positive control: NO nft table. Every marked + unmarked dial must reach its server (oracle works; rule is the cause)")
        run_matrix("P0", expect_denied_marked=False)
        # gateway:53 reachability control
        for proto in ("tcp", "udp"):
            fn = (lambda: tcp_try(GW, 53, OWNER)) if proto == "tcp" else (lambda: udp_try(GW, 53, OWNER))
            r, d = delta(f"DNS-{proto}", fn); rec("P0", f"marked {proto} -> gateway:53", "", r, True, d > 0)

    if "p1" in which:
        print("\n=== P1 design rule (D26): output hook, exact owner mark, static ranges + fib daddr type local, gateway:53 exception")
        nft_install(ruleset())
        run_matrix("P1", expect_denied_marked=True)
        for proto in ("tcp", "udp"):
            fn = (lambda: tcp_try(GW, 53, OWNER)) if proto == "tcp" else (lambda: udp_try(GW, 53, OWNER))
            r, d = delta(f"DNS-{proto}", fn); rec("P1", f"marked {proto} -> gateway:53 (exception)", "", r, True, d > 0)
        r, d = delta("DNS-udp", lambda: udp_try(GW, 53, OWNER, size=59000)); rec("P1", "marked udp 59000B -> gateway:53 (exception)", "", r, True, d > 0)
        r, d = delta("W-udp", lambda: udp_try("127.0.0.1", W, OWNER, size=59000)); rec("P1", "marked udp 59000B -> 127.0.0.1:W", "", r, False, d > 0)
        # unmarked / other marks unaffected
        for lab, mk in (("unmarked", None), ("mark 0x1 (TPROXY)", 0x1), ("mark 0x2 (leg-S)", 0x2)):
            for name, dst, port, st in [LOCAL_CASES[0], LOCAL_CASES[1], LOCAL_CASES[3], LOCAL_CASES[5]]:
                for proto in ("tcp", "udp"):
                    tag = f"{st}-{proto}"
                    fn = (lambda: tcp_try(dst, port, mk)) if proto == "tcp" else (lambda: udp_try(dst, port, mk))
                    r, d = delta(tag, fn); rec("P1", f"{lab} {proto} -> {name}", "", r, True, d > 0)
        for proto in ("tcp",):
            r, d = delta("LEGF-tcp", lambda: tcp_try("127.0.0.1", LEGF, None, linger=False)); rec("P1", "unmarked tcp -> leg-F 127.0.0.1 (mesh path)", "", r, True, d > 0)
            r, d = delta("LEGF-tcp", lambda: tcp_try("127.0.0.1", LEGF, OWNER)); rec("P1", "MARKED tcp -> leg-F (would be denied: leg-F dial must stay unmarked)", "", r, False, d > 0)
        print("--- ruleset counters after P1:\n" + counters())

    if "p2" in which:
        print("\n=== P2 race: address added after the owner's 'not local' check, rule still rejects")
        sh("nft delete table ip v21-mtls 2>/dev/null; true", check=False)
        sh("ip addr del 192.0.2.77/32 dev lo 2>/dev/null; true", check=False)
        sh("ip route replace default via 10.77.21.2 dev v21-h")   # 192.0.2.77 non-local -> routed off-host
        for ctrl in (True, False):
            if not ctrl: nft_install(ruleset())
            for proto in ("tcp", "udp"):
                sh("ip addr del 192.0.2.77/32 dev lo 2>/dev/null; true", check=False)
                chk = sh("ip route get 192.0.2.77").strip().splitlines()[0]
                assert "local" not in chk, chk
                sh("ip addr add 192.0.2.77/32 dev lo")      # the window: after the check, before the connect
                tag = f"W-{proto}"
                fn = (lambda: tcp_try("192.0.2.77", W, OWNER)) if proto == "tcp" else (lambda: udp_try("192.0.2.77", W, OWNER))
                r, d = delta(tag, fn)
                rec("P2", f"{'control(no rule)' if ctrl else 'rule'} marked {proto} -> address added after check", "", r, ctrl, d > 0, f"(check said: {chk[:40]})")
        sh("ip addr del 192.0.2.77/32 dev lo 2>/dev/null; true", check=False)
        # address added to the gateway bridge / removed from veth after check: same primitive; one more: new address on a dummy
        sh("ip link add v21-dum type dummy; ip link set v21-dum up; ip addr add 198.18.9.9/32 dev v21-dum")
        r, d = delta("W-tcp", lambda: tcp_try("198.18.9.9", W, OWNER)); rec("P2", "marked tcp -> address on a fresh dummy iface", "", r, False, d > 0)
        sh("ip link set v21-dum down")
        pk = rule_pkts("fib daddr type local")
        r2 = tcp_try("198.18.9.9", W, OWNER, tmo=0.4)
        print(f"   address on an administratively DOWN interface: {r2}; fib-local reject counter {pk} -> {rule_pkts('fib daddr type local')} (a rise = the kernel still classes it local and the rule rejected it)")
        sh("ip link del v21-dum")

    if "p3" in which:
        print("\n=== P3 bypass attempts / enumerated hazards")
        nft_install(ruleset())
        # exact-match vs mark with extra bits
        for mk in (OWNER | 0x1, OWNER | 0x100):
            r, d = delta("W-tcp", lambda: tcp_try("127.0.0.1", W, mk))
            print(f"   [info] tcp mark 0x{mk:x} (owner bit + extra bits) -> 127.0.0.1:W reached={d>0} {r}   (exact `meta mark 0x4` does NOT match composite marks)")
        # IPv4-mapped v6 socket
        s6 = socket.socket(socket.AF_INET6, socket.SOCK_STREAM); s6.setsockopt(socket.SOL_SOCKET, socket.SO_MARK, OWNER); s6.settimeout(1)
        try: s6.connect(("::ffff:127.0.0.1", W)); res = "ok"
        except OSError as e: res = errno.errorcode.get(e.errno, str(e))
        time.sleep(0.1); s6.close()
        rec("P3", "marked AF_INET6 v4-mapped ::ffff:127.0.0.1:W", "", res, False, res == "ok")
        # bound-to-device
        r, d = delta("W-tcp", lambda: tcp_try(HOSTIP, W, OWNER, bind_dev="v21-h")); rec("P3", "marked tcp SO_BINDTODEVICE=v21-h -> own veth address", "", r, False, d > 0)
        r, d = delta("W-tcp", lambda: tcp_try("127.0.0.1", W, OWNER, bind_dev="v21-h")); rec("P3", "marked tcp SO_BINDTODEVICE=v21-h -> 127.0.0.1", "", r, False, d > 0)
        # directed / limited broadcast (fib type broadcast, not local): needs SO_BROADCAST, which the owner never sets
        for lab, b in (("directed bcast 10.77.21.3 (veth /30)", "10.77.21.3"), ("guest-prefix bcast 100.95.255.255", "100.95.255.255")):
            r0, d0 = delta("W-udp", lambda: udp_try(b, W, OWNER, bcast=False))
            r1, d1 = delta("W-udp", lambda: udp_try(b, W, OWNER, bcast=True))
            print(f"   [info] marked udp -> {lab}: SO_BROADCAST off -> reached={d0>0} {r0}; SO_BROADCAST on -> reached={d1>0} {r1}")
        # multicast / 0.0.0.0 reached via static Policy (owner) - show kernel rule alone
        r, d = delta("W-udp", lambda: udp_try("224.0.0.1", W, OWNER)); print(f"   [info] marked udp -> 224.0.0.1 (owner's static Policy covers it; rule does not): reached={d>0} {r}")
        # conntrack-established accept placed BEFORE the D26 rule: UDP tuple pre-existing from an unmarked flow
        for order in ("ct-first", "d26-first"):
            nft_install(ruleset(order=order, ctaccept=True))
            sp = 41000 if order == "ct-first" else 41001
            a = udp_try("127.0.0.1", W, None, sport=sp)      # unmarked flow creates a replied (ESTABLISHED) conntrack entry
            time.sleep(0.05)
            r, d = delta("W-udp", lambda: udp_try("127.0.0.1", W, OWNER, sport=sp))   # marked socket reusing the 5-tuple
            rec("P3", f"ordering {order}: marked udp reuses sport of an established unmarked flow to 127.0.0.1:W", "", r, False, d > 0)
        nft_install(ruleset())

    if "p4" in which:
        print("\n=== P4 reject flavour: what the owner sees (design: 'rejects'; projected as DestinationUnreachable)")
        for rj in ("reject", "reject with tcp reset", "reject with icmp type admin-prohibited", "drop"):
            nft_install(ruleset(reject=rj))
            rt = tcp_try("127.0.0.1", W, OWNER, tmo=2.0); ru = udp_try("127.0.0.1", W, OWNER)
            print(f"   {rj:42s} tcp: {rt}   udp: {ru}")
        nft_install(ruleset())

    if "p5" in which:
        print("\n=== P5 rule maintenance: marked connect storm while the table is replaced")
        def storm(seconds, replace_fn):
            stop = time.monotonic() + seconds; leaks = [0]; tried = [0]
            b = sum(v for k, v in hits.items() if k.startswith("W-tcp:accept"))
            def cl():
                while time.monotonic() < stop:
                    tcp_try("127.0.0.1", W, OWNER, tmo=0.5); tried[0] += 1
            ts = [threading.Thread(target=cl) for _ in range(4)]; [t.start() for t in ts]
            n = 0
            while time.monotonic() < stop: replace_fn(); n += 1
            [t.join() for t in ts]; time.sleep(0.2)
            a = sum(v for k, v in hits.items() if k.startswith("W-tcp:accept"))
            return tried[0], a - b, n
        rs = ruleset()
        open("/tmp/v21-rs.nft", "w").write(rs)
        def atomic(): sh("nft -f /tmp/v21-rs.nft 2>&1 | head -1; true", check=False)       # table already exists -> use flush+rebuild below
        open("/tmp/v21-atomic.nft", "w").write("delete table ip v21-mtls\n" + rs + "\n")
        def atomic_tx(): sh("nft -f /tmp/v21-atomic.nft", check=False)
        def nonatomic(): sh("nft delete table ip v21-mtls", check=False); sh("nft -f /tmp/v21-rs.nft", check=False)
        nft_install(rs)
        t, l, n = storm(3.0, atomic_tx); rec("P5", f"atomic delete+add in ONE transaction ({n} replacements, {t} marked connects)", "", {"leaked": l}, False, l > 0)
        nft_install(rs)
        t, l, n = storm(3.0, nonatomic); print(f"   [info] NON-atomic delete then add ({n} replacements, {t} marked connects): leaked={l}")
        nft_install(rs)

    if "p6" in which:
        print("\n=== P6 composed with the other constant output rules (leg-S exemption, ct established accept, ADR-0172 prefix reject)")
        nft_install(ruleset(prefix=True))
        r, d = delta("W-tcp", lambda: tcp_try("100.95.0.7", W, 0x2)); rec("P6", "leg-S mark 0x2 tcp -> guest prefix 100.95.0.7 (exempt)", "", r, True, d > 0)
        r, d = delta("W-tcp", lambda: tcp_try("100.95.0.7", W, OWNER)); rec("P6", "owner-marked tcp -> guest prefix (D26 first)", "", r, False, d > 0)
        r, d = delta("W-tcp", lambda: tcp_try("100.95.0.7", W, None)); rec("P6", "unmarked tcp -> guest prefix (ADR-0172 reject, not D26)", "", r, False, d > 0)
        r, d = delta("DNS-tcp", lambda: tcp_try(GW, 53, OWNER)); rec("P6", "owner-marked tcp -> gateway:53 (exception survives composition)", "", r, True, d > 0)
        r, d = delta("DNS-udp", lambda: udp_try(GW, 53, OWNER)); rec("P6", "owner-marked udp -> gateway:53", "", r, True, d > 0)
        print("--- counters:\n" + counters())

    if "p7" in which:
        print("\n=== P7 candidate hardening: `fib daddr type != unicast` instead of `== local` (covers broadcast / multicast / anycast too)")
        nft_install(ruleset(localexpr="fib daddr type != unicast"))
        for lab, b, port in (("directed bcast 10.77.21.3", "10.77.21.3", W), ("guest-prefix bcast 100.95.255.255", "100.95.255.255", W), ("multicast 224.0.0.1", "224.0.0.1", W)):
            r1, d1 = delta("W-udp", lambda: udp_try(b, port, OWNER, bcast=True)); rec("P7", f"marked udp (SO_BROADCAST on) -> {lab}", "", r1, False, d1 > 0)
        for name, dst, port, st in LOCAL_CASES:
            for proto in ("tcp", "udp"):
                fn = (lambda: tcp_try(dst, port, OWNER)) if proto == "tcp" else (lambda: udp_try(dst, port, OWNER))
                r, d = delta(f"{st}-{proto}", fn); rec("P7", f"marked {proto} -> {name}", "", r, False, d > 0)
        for name, dst, port, tag, allowed in PEER_CASES:
            if not allowed: continue
            for proto in ("tcp", "udp"):
                fn = (lambda: tcp_try(dst, port, OWNER)) if proto == "tcp" else (lambda: udp_try(dst, port, OWNER))
                r, d = delta(f"{proto} {tag}", fn, peer=True); rec("P7", f"marked {proto} -> {name} (must still be allowed)", "", r, True, d > 0)
        for proto in ("tcp", "udp"):
            fn = (lambda: tcp_try(GW, 53, OWNER)) if proto == "tcp" else (lambda: udp_try(GW, 53, OWNER))
            r, d = delta(f"DNS-{proto}", fn); rec("P7", f"marked {proto} -> gateway:53 (exception)", "", r, True, d > 0)

    sh("nft delete table ip v21-mtls 2>/dev/null; true", check=False)
    print(f"\nSUMMARY: {sum(1 for _,_,ok in results if ok)}/{len(results)} checks as predicted; unexpected: {fails}")
    sys.exit(1 if fails else 0)
main()
