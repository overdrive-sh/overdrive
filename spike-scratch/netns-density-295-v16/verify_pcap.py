#!/usr/bin/env python3
"""V-16: reassemble a wire capture (classic pcap, Ethernet/IPv4) and check, per case, that the bytes that left the
NIC reassemble to exactly the application payload (what a real receiver would deliver). Case table comes from
`v16 cases all` (printed lines: id Kind n=.. gso=.. wire=.. sport=.. dport=..)."""
import struct, sys, subprocess, collections
pcap, cases_txt = sys.argv[1], sys.argv[2]
MAGIC = b"ZUD1"
cases = {}
for line in open(cases_txt):
    p = line.split()
    if len(p) < 7 or not p[0].isdigit():
        continue
    cid = int(p[0]); kind = p[1]; n = int(p[2][2:]); gso = p[3] == "gso=true"; dport = int(p[6][6:])
    v = bytearray(((i * 131 + cid * 17 + (i >> 8) * 7 + 1) & 0xFF) for i in range(n))
    if kind == "Bare" and v: v[0] |= 0x80
    if kind == "Escaped": v[0:4] = MAGIC; v[4:8] = struct.pack(">I", n - 8)
    if kind == "NearMiss": v[0:4] = MAGIC; v[4:8] = struct.pack(">I", n)
    cases[dport] = dict(id=cid, kind=kind, n=n, gso=gso, exp=bytes(v))
data = open(pcap, "rb").read()
magic = struct.unpack("<I", data[:4])[0]
assert magic in (0xA1B2C3D4, 0xA1B23C4D), hex(magic)
pos = 24
frags = collections.OrderedDict()   # (src,dst,id) -> list of (off, mf, payload, first_pkt_index)
pkts = 0; nfrag_pkts = 0; maxwire = 0
order = []
while pos + 16 <= len(data):
    ts, tu, cl, ol = struct.unpack("<IIII", data[pos:pos + 16]); pos += 16
    pk = data[pos:pos + cl]; pos += cl
    if len(pk) < 34 or pk[12:14] != b"\x08\x00": continue
    ihl = (pk[14] & 0xF) * 4
    if pk[14 + 9] != 17: continue
    tot = struct.unpack(">H", pk[16:18])[0]
    ident = struct.unpack(">H", pk[18:20])[0]
    fw = struct.unpack(">H", pk[20:22])[0]
    off = (fw & 0x1FFF) * 8; mf = bool(fw & 0x2000)
    key = (pk[26:30], pk[30:34], ident, pkts)
    pl = pk[14 + ihl:14 + tot]
    pkts += 1; maxwire = max(maxwire, len(pk))
    if mf or off: nfrag_pkts += 1
    # group fragments by (src,dst,id) among nearby packets: unfragmented => own entry
    gk = (pk[26:30], pk[30:34], ident)
    if not mf and off == 0:
        order.append(("whole", pl, pk[26:30], pk[30:34]))
    else:
        ent = frags.setdefault(gk, [])
        ent.append((off, mf, pl))
        if not mf:
            ent.sort()
            buf = b""; okc = True
            for o, m, p in ent:
                if o != len(buf): okc = False
                buf += p
            order.append(("reasm" if okc else "reasm-GAP", buf, gk[0], gk[1]))
            del frags[gk]
leftover = len(frags)
got = collections.defaultdict(list)
def csum_state(src, dst, ipp):
    ck = struct.unpack(">H", ipp[6:8])[0]
    if ck == 0: return "zero"
    ps = src + dst + b"\x00\x11" + struct.pack(">H", len(ipp))
    b = ps + ipp + (b"\x00" if len(ipp) % 2 else b"")
    t = sum(struct.unpack(">%dH" % (len(b) // 2), b))
    while t >> 16: t = (t & 0xFFFF) + (t >> 16)
    return "valid" if t == 0xFFFF else "invalid"
for kind, ip_payload, esrc, edst in order:
    if len(ip_payload) < 8: continue
    sport, dport, ulen, ck = struct.unpack(">HHHH", ip_payload[:8])
    body = ip_payload[8:]
    got[dport].append((kind, ulen, len(body), ck, bytes(body), csum_state(esrc, edst, ip_payload)))
ok = bad = 0
for dport, c in sorted(cases.items(), key=lambda kv: kv[1]["id"]):
    g = got.get(dport, [])
    if c["gso"]:
        cat = b"".join(x[4] for x in g)
        good = cat == c["exp"] and (bool(g) or c["n"] == 0)
        det = "datagrams=%d sizes=%s" % (len(g), [x[2] for x in g][:6])
    else:
        good = len(g) == 3 and all(x[4] == c["exp"] and x[1] == c["n"] + 8 for x in g)
        det = "datagrams=%d body_lens=%s udp_len_fields=%s reasm=%s udp_csum=%s" % (len(g), sorted(set(x[2] for x in g)), sorted(set(x[1] for x in g)), sorted(set(x[0] for x in g)), sorted(set(x[5] for x in g)))
    ok += good; bad += (not good)
    print("WIRE %s id=%d %s n=%d gso=%s %s" % ("BYTE-EXACT" if good else "FAIL", c["id"], c["kind"], c["n"], c["gso"], det))
print("WIRE-SUMMARY byte-exact=%d fail=%d total=%d  ip_packets_on_wire=%d fragment_packets=%d max_frame=%d unfinished_reassemblies=%d" % (ok, bad, len(cases), pkts, nfrag_pkts, maxwire, leftover))
