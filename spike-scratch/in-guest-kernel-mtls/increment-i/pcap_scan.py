#!/usr/bin/env python3
"""Spike D tap-capture scanner (GH #303, increment-i). Backends A/B are the TLS
ports 6443/6444; 5001 is the plaintext pass-through positive control. The mesh
plaintext litmus marker is b"IGKM-D-"; every exact REQUEST/RESPONSE line below
must be 0x on the mesh (TLS) ports and present on 5001. Throwaway probe.

    pcap_scan.py <tap.pcap> [t0-epoch-secs]

Reads a libpcap file (Ethernet, as `tcpdump -i <tap> -w` writes it),
reassembles every TCP stream between the guest and the host by sequence
number, and per direction:
  * parses TLS record framing (type/version/length) on the TLS ports and
    checks that every byte belongs to a record (no stray cleartext);
  * prints each record with the capture time of the segment that carries its
    first byte (t+ relative to the runner's T0, the same clock the host relay
    and peer log against) and groups records by TCP segment, so records that
    were coalesced into one segment are visible;
  * counts the plaintext litmus marker b"IGKM-E-" and every exact
    REQUEST/RESPONSE line the probe uses.
The plain pass-through port (5001) is the positive control: the same scan
must find its plaintext there.
"""
import struct
import sys

TLS_PORTS = {6443, 6444}
GUEST = "192.168.204.2"
MARKER = b"IGKM-D-"
MARKERS = [b"IGKM-D-"]
LINES = [
    b"IGKM-D-REQ-PT-pt guest->peer plaintext\n",
    b"IGKM-D-RESP-PT-pt from-PT peer->guest plaintext\n",
    b"IGKM-D-REQ-svca-c1 guest->peer lb-connect-1\n",
    b"IGKM-D-RESP-svca-c1 from-A peer->guest lb-connect-1\n",
    b"IGKM-D-REQ-svca-c2 guest->peer lb-connect-2-after-health-toggle\n",
    b"IGKM-D-RESP-svca-c2 from-B peer->guest lb-connect-2-after-health-toggle\n",
]
TYPES = {0x14: "CCS", 0x15: "alert", 0x16: "handshake", 0x17: "appdata"}


def packets(path):
    with open(path, "rb") as f:
        gh = f.read(24)
        magic = struct.unpack("<I", gh[:4])[0]
        endian = "<" if magic in (0xA1B2C3D4, 0xA1B23C4D) else ">"
        nano = magic in (0xA1B23C4D, 0x4D3CB2A1)
        linktype = struct.unpack(endian + "I", gh[20:24])[0]
        assert linktype == 1, f"expected Ethernet linktype, got {linktype}"
        while True:
            ph = f.read(16)
            if len(ph) < 16:
                return
            ts_s, ts_frac, incl, _orig = struct.unpack(endian + "IIII", ph)
            yield ts_s + ts_frac / (1e9 if nano else 1e6), f.read(incl)


def tcp_segments(path):
    for ts, frame in packets(path):
        if len(frame) < 14 or struct.unpack("!H", frame[12:14])[0] != 0x0800:
            continue
        ip = frame[14:]
        ihl = (ip[0] & 0x0F) * 4
        if ip[9] != 6:
            continue
        total = struct.unpack("!H", ip[2:4])[0]
        src = ".".join(map(str, ip[12:16]))
        dst = ".".join(map(str, ip[16:20]))
        tcp = ip[ihl:total]
        sport, dport, seq, _ack = struct.unpack("!HHII", tcp[:12])
        off = (tcp[12] >> 4) * 4
        flags = tcp[13]
        yield ts, src, sport, dst, dport, seq, flags, tcp[off:]


def main():
    t0 = float(sys.argv[2]) if len(sys.argv) > 2 else None
    rel = (lambda t: f"t+{t - t0:.6f}s") if t0 is not None else (lambda t: f"{t:.6f}")
    streams = {}
    for ts, src, sport, dst, dport, seq, flags, payload in tcp_segments(sys.argv[1]):
        key = (src, sport, dst, dport)
        s = streams.setdefault(key, {"isn": None, "segs": {}, "flags": set(), "t0": ts})
        if flags & 0x02:
            s["isn"] = seq
            s["flags"].add("SYN")
        if flags & 0x01:
            s["flags"].add("FIN")
        if flags & 0x04:
            s["flags"].add("RST")
        if payload and seq not in s["segs"]:
            s["segs"][seq] = (payload, ts)  # first transmission; retransmits ignored

    def data_of(s):
        """Reassembled bytes plus [(rel_start, rel_end, ts, seg_no)] per segment."""
        if not s["segs"]:
            return b"", []
        base = (s["isn"] + 1) & 0xFFFFFFFF if s["isn"] is not None else min(s["segs"])
        out = bytearray()
        segs = []
        for n, seq in enumerate(sorted(s["segs"], key=lambda q: (q - base) & 0xFFFFFFFF), 1):
            r = (seq - base) & 0xFFFFFFFF
            seg, ts = s["segs"][seq]
            if r > len(out):
                out += b"\0" * (r - len(out))
            out[r:r + len(seg)] = seg
            segs.append((r, r + len(seg), ts, n))
        return bytes(out), segs

    def seg_at(segs, off):
        for a, b, ts, n in segs:
            if a <= off < b:
                return ts, n
        return None, 0

    conns = {}
    for (src, sport, dst, dport), s in streams.items():
        server_port = dport if dport in TLS_PORTS | {5001} else sport
        client_port = sport if server_port == dport else dport
        label = ("guest" if src == GUEST else "peer") + "->" + ("guest" if dst == GUEST else "peer")
        s["client_is_guest"] = (src == GUEST) == (server_port == dport)
        conns.setdefault((client_port, server_port), {})[label] = s

    all_bytes = {"tls": b"", "plain": b""}
    for (cport, sport) in sorted(conns, key=lambda k: min(v["t0"] for v in conns[k].values())):
        dirs = conns[(cport, sport)]
        kind = "TLS" if sport in TLS_PORTS else "PLAIN"
        cig = next(iter(dirs.values()))["client_is_guest"]
        cl, sv = ("guest", "peer") if cig else ("peer", "guest")
        first = min(v["t0"] for v in dirs.values())
        print(f"SCAN conn client {cl}:{cport} -> server {sv}:{sport} ({kind}) first packet {rel(first)}")
        for d in ("guest->peer", "peer->guest"):
            s = dirs.get(d)
            if s is None:
                print(f"SCAN   {d}: no packets")
                continue
            data, segs = data_of(s)
            all_bytes["tls" if kind == "TLS" else "plain"] += data
            flags = ",".join(sorted(s["flags"]))
            mk = " ".join(f"marker({m.decode()})={data.count(m)}" for m in MARKERS)
            line = f"SCAN   {d}: {len(data)} bytes flags[{flags}] {mk}"
            if kind == "TLS":
                recs, i = [], 0
                while i + 5 <= len(data):
                    t, ver = data[i], struct.unpack("!H", data[i + 1:i + 3])[0]
                    ln = struct.unpack("!H", data[i + 3:i + 5])[0]
                    if t not in TYPES or ver not in (0x0301, 0x0303) or i + 5 + ln > len(data):
                        break
                    ts, n = seg_at(segs, i)
                    recs.append((f"0x{t:02x}/{ln}", ts, n))
                    i += 5 + ln
                stray = len(data) - i
                n17 = sum(1 for r in recs if r[0].startswith("0x17"))
                line += f" records={len(recs)} (0x17 count={n17}) unframed_bytes={stray}"
                print(line)
                print(f"SCAN     records: {' '.join(r[0] for r in recs) if recs else '(none)'}")
                groups = {}
                for name, ts, n in recs:
                    groups.setdefault(n, (ts, []))[1].append(name)
                for n in sorted(groups):
                    ts, names = groups[n]
                    tag = " COALESCED" if len(names) > 1 else ""
                    print(f"SCAN       seg#{n} {rel(ts) if ts else '?'}: {' '.join(names)}{tag}")
            else:
                print(line)
                for L in LINES:
                    if L in data:
                        print(f"SCAN     contains plaintext {L!r}")
    print("SCAN totals:")
    for kind, blob in all_bytes.items():
        mk = ", ".join(f"marker({m.decode()}) occurrences={blob.count(m)}" for m in MARKERS)
        print(f"SCAN   {kind} streams: {len(blob)} bytes, {mk}")
        for L in LINES:
            c = blob.count(L)
            if c or kind == "tls":
                print(f"SCAN     {kind}: {L!r} occurrences={c}")


if __name__ == "__main__":
    main()
