#!/usr/bin/env python3
"""Spike B tap-capture scanner (GH #303, increment-d). Throwaway probe.

    pcap_scan.py <tap.pcap>

Reads a libpcap file (Ethernet, as `tcpdump -i <tap> -w` writes it),
reassembles every TCP stream between the guest and the host by sequence
number, and per direction:
  * parses TLS record framing (type/version/length) on the TLS ports and
    checks that every byte belongs to a record (no stray cleartext);
  * counts the plaintext litmus marker b"IGKM-D-" and every exact
    REQUEST/RESPONSE line the probe uses.
The plain pass-through port (5001) is the positive control: the same scan
must find its plaintext there.
"""
import struct
import sys

TLS_PORTS = {6443, 6444, 6445, 6446, 7443}
GUEST = "192.168.203.2"
MARKER = b"IGKM-D-"
LINES = [
    b"IGKM-D-REQ1 client-first guest->peer\n",
    b"IGKM-D-RESP1 client-first peer->guest\n",
    b"IGKM-D-REQ2 after-relay-kill guest->peer\n",
    b"IGKM-D-RESP2 after-relay-kill peer->guest\n",
    b"IGKM-D-GREETING server-first peer->guest\n",
    b"IGKM-D-REQS server-first guest->peer\n",
    b"IGKM-D-RESPS server-first peer->guest\n",
    b"IGKM-D-REQN nst guest->peer\n",
    b"IGKM-D-RESPN nst peer->guest\n",
    b"IGKM-D-PLAIN-REQ pass-through guest->host\n",
    b"IGKM-D-PLAIN-RESP pass-through host->guest\n",
    b"IGKM-D-REQI inbound peer->guest\n",
    b"IGKM-D-RESPI inbound guest->peer\n",
]
TYPES = {0x14: "CCS", 0x15: "alert", 0x16: "handshake", 0x17: "appdata"}


def packets(path):
    with open(path, "rb") as f:
        gh = f.read(24)
        magic = struct.unpack("<I", gh[:4])[0]
        endian = "<" if magic in (0xA1B2C3D4, 0xA1B23C4D) else ">"
        linktype = struct.unpack(endian + "I", gh[20:24])[0]
        assert linktype == 1, f"expected Ethernet linktype, got {linktype}"
        while True:
            ph = f.read(16)
            if len(ph) < 16:
                return
            ts_s, ts_us, incl, _orig = struct.unpack(endian + "IIII", ph)
            yield ts_s + ts_us / 1e6, f.read(incl)


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
    streams = {}  # (src,sport,dst,dport) -> dict
    for ts, src, sport, dst, dport, seq, flags, payload in tcp_segments(sys.argv[1]):
        key = (src, sport, dst, dport)
        s = streams.setdefault(key, {"isn": None, "segs": {}, "flags": set(), "t0": ts})
        if flags & 0x02:  # SYN
            s["isn"] = seq
            s["flags"].add("SYN")
        if flags & 0x01:
            s["flags"].add("FIN")
        if flags & 0x04:
            s["flags"].add("RST")
        if payload:
            s["segs"].setdefault(seq, payload)

    def data_of(s):
        if not s["segs"]:
            return b""
        base = (s["isn"] + 1) & 0xFFFFFFFF if s["isn"] is not None else min(s["segs"])
        out = bytearray()
        for seq in sorted(s["segs"], key=lambda q: (q - base) & 0xFFFFFFFF):
            rel = (seq - base) & 0xFFFFFFFF
            seg = s["segs"][seq]
            if rel > len(out):
                out += b"\0" * (rel - len(out))  # gap (should not happen)
            out[rel:rel + len(seg)] = seg
        return bytes(out)

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
        print(f"SCAN conn client {cl}:{cport} -> server {sv}:{sport} ({kind})")
        for d in ("guest->peer", "peer->guest"):
            s = dirs.get(d)
            if s is None:
                print(f"SCAN   {d}: no packets")
                continue
            data = data_of(s)
            all_bytes["tls" if kind == "TLS" else "plain"] += data
            flags = ",".join(sorted(s["flags"]))
            line = f"SCAN   {d}: {len(data)} bytes flags[{flags}] marker(IGKM-D-)={data.count(MARKER)}"
            if kind == "TLS":
                recs, i = [], 0
                while i + 5 <= len(data):
                    t, ver, ln = data[i], struct.unpack("!H", data[i + 1:i + 3])[0], struct.unpack("!H", data[i + 3:i + 5])[0]
                    if t not in TYPES or ver not in (0x0301, 0x0303) or i + 5 + ln > len(data):
                        break
                    recs.append(f"0x{t:02x}/{ln}")
                    i += 5 + ln
                stray = len(data) - i
                n17 = sum(1 for r in recs if r.startswith("0x17"))
                line += f" records={len(recs)} (0x17 count={n17}) unframed_bytes={stray}"
                print(line)
                print(f"SCAN     records: {' '.join(recs) if recs else '(none)'}")
            else:
                print(line)
                for L in LINES:
                    if L in data:
                        print(f"SCAN     contains plaintext {L!r}")
    print("SCAN totals:")
    for kind, blob in all_bytes.items():
        print(f"SCAN   {kind} streams: {len(blob)} bytes, marker(IGKM-D-) occurrences={blob.count(MARKER)}")
        for L in LINES:
            c = blob.count(L)
            if c or kind == "tls":
                print(f"SCAN     {kind}: {L!r} occurrences={c}")


if __name__ == "__main__":
    main()
