#!/usr/bin/env bash
# GH295 Phase 1. Real two-microVM shared-bridge mechanism, no workload netns.
set -euo pipefail
spike_here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
spike_run=$(mktemp -d /var/tmp/gh295-p.XXXXXXXX)
exec > >(tee -a "$spike_run/transcript.log") 2>&1
spike_start=$(python3 -c 'import time; print(time.perf_counter())')
printf 'SPIKE_RUN=%s\nSTART=%s\n' "$spike_run" "$(date -u +%FT%TZ)"
uname -r
uname -m
systemd-detect-virt || [ "$?" = 1 ]
cloud-hypervisor --version
printf 'COMMAND build-and-execute-static-Rust-probe\n'
export CARGO_TARGET_DIR="$spike_here/target"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=cc
cargo build --release --target x86_64-unknown-linux-musl --manifest-path "$spike_here/Cargo.toml"
spike_bin="$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/probe"
file "$spike_bin"
spike_stage="$spike_run/rootfs"
mkdir -p "$spike_stage"/{proc,dev,sys,tmp,etc}
install -m 0755 "$spike_bin" "$spike_stage/init"
printf 'nameserver 10.95.0.1\noptions timeout:2 attempts:1\n' > "$spike_stage/etc/resolv.conf"
mknod "$spike_stage/dev/console" c 5 1
mknod "$spike_stage/dev/null" c 1 3
truncate -s 64M "$spike_run/rootfs.ext4"
mkfs.ext4 -F -d "$spike_stage" "$spike_run/rootfs.ext4"
cp "$spike_run/rootfs.ext4" "$spike_run/client.ext4"
cp "$spike_run/rootfs.ext4" "$spike_run/server.ext4"
spike_rp=$(sysctl -n net.ipv4.conf.all.rp_filter)
spike_pids=()
spike_captures=()
spike_links=()
spike_table=0
spike_bridge_table=0
spike_rule=0
spike_route=0
cleanup() {
  spike_status=$?
  trap - EXIT
  set +e
  printf '\nCLEANUP START prior_exit=%s at=%s\n' "$spike_status" "$(date -u +%FT%TZ)"
  for pid in "${spike_pids[@]}"; do kill -TERM "$pid" 2>/dev/null; done
  for pid in "${spike_captures[@]}"; do kill -INT "$pid" 2>/dev/null; done
  for pid in "${spike_pids[@]}" "${spike_captures[@]}"; do wait "$pid" 2>/dev/null; done
  nft list table bridge gh295p
  nft list table ip gh295p
  ip -br addr
  ip rule
  for f in host.log client.console server.console client.stderr server.stderr capture-lo.stderr capture-tap295a.stderr capture-tap295b.stderr nft-trace.log; do
    if [ -f "$spike_run/$f" ]; then printf '\nCAPTURE %s\n' "$f"; cat "$spike_run/$f"; fi
  done
  [ "$spike_table" = 0 ] || nft delete table ip gh295p
  [ "$spike_bridge_table" = 0 ] || nft delete table bridge gh295p
  [ "$spike_rule" = 0 ] || ip rule del priority 2950 fwmark 0x295 lookup 295
  [ "$spike_route" = 0 ] || ip route del local 0.0.0.0/0 dev lo table 295
  for link in "${spike_links[@]}"; do ip link del "$link"; done
  sysctl -qw "net.ipv4.conf.all.rp_filter=$spike_rp"
  printf 'CLEANUP COMPLETE preserved_evidence=%s original_exit=%s\n' "$spike_run" "$spike_status"
  python3 -c 'import sys,time; print(f"TOTAL_ELAPSED_SECONDS={time.perf_counter()-float(sys.argv[1]):.6f}")' "$spike_start"
  exit "$spike_status"
}
trap cleanup EXIT

set -x
! ip link show br295p
! ip link show tap295a
! ip link show tap295b
! nft list table ip gh295p
! nft list table bridge gh295p
[ -z "$(ip route show table 295 2>/dev/null)" ]
modprobe nft_tproxy
modprobe tls
ip link add br295p type bridge
spike_links+=(br295p)
ip addr add 10.95.0.1/24 dev br295p
ip link set br295p up
for tap in tap295a tap295b; do
  ip tuntap add dev "$tap" mode tap
  spike_links=("$tap" "${spike_links[@]}")
  ip link set "$tap" master br295p
  ip link set "$tap" up
  sysctl -qw "net.ipv4.conf.$tap.rp_filter=0"
done
sysctl -qw net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.br295p.rp_filter=0
ip rule add priority 2950 fwmark 0x295 lookup 295
spike_rule=1
ip route add local 0.0.0.0/0 dev lo table 295
spike_route=1
nft add table bridge gh295p
spike_bridge_table=1
nft add chain bridge gh295p prerouting '{ type filter hook prerouting priority -300; policy accept; }'
nft add rule bridge gh295p prerouting iifname tap295a ether type ip ip protocol tcp tcp dport 9000 counter meta pkttype set host meta broute set 1
nft add rule bridge gh295p prerouting iifname tap295a ether type ip ip protocol udp udp dport 53 counter meta nftrace set 1
nft add table ip gh295p
spike_table=1
nft add chain ip gh295p prerouting '{ type filter hook prerouting priority -151; policy accept; }'
nft add rule ip gh295p prerouting iifname tap295a ip protocol udp udp dport 53 counter
nft add rule ip gh295p prerouting iifname tap295a ip protocol tcp tcp dport 9000 counter tproxy to 127.0.0.1:15294 meta mark set 0x295 accept
nft add rule ip gh295p prerouting meta mark 0x295 ip daddr 10.95.0.3 tcp dport 9000 counter tproxy to 127.0.0.1:15295 accept
nft add chain ip gh295p output '{ type route hook output priority -151; policy accept; }'
nft add rule ip gh295p output meta mark 0x2951 ip daddr 10.95.0.3 tcp dport 9000 counter meta mark set 0x295 accept
ip -br addr
ip netns list
bridge link
nft monitor trace > "$spike_run/nft-trace.log" 2>&1 &
spike_pids+=("$!")
for iface in lo tap295a tap295b; do
  tcpdump -U -s 0 -ni "$iface" -w "$spike_run/$iface.pcap" 'net 10.95.0.0/24 and (tcp port 9000 or udp port 53)' > "$spike_run/capture-$iface.stderr" 2>&1 &
  spike_captures+=("$!")
done
sleep 0.4
timeout 45 "$spike_bin" > "$spike_run/host.log" 2>&1 &
spike_host=$!
spike_pids+=("$spike_host")
for i in $(seq 1 100); do
  if grep -q 'HOST READY' "$spike_run/host.log"; then break; fi
  sleep 0.05
done
grep 'HOST READY' "$spike_run/host.log"
timeout 40 cloud-hypervisor --cpus boot=1 --memory size=256M --kernel "$OVERDRIVE_METAL_KERNEL" --cmdline 'root=/dev/vda rw console=ttyS0 init=/init panic=0 loglevel=4 spike_role=server' --disk "path=$spike_run/server.ext4,image_type=raw" --net 'tap=tap295b,mac=02:00:00:95:00:03,offload_tso=off,offload_ufo=off,offload_csum=off' --serial "file=$spike_run/server.console" --console off > "$spike_run/server.stderr" 2>&1 &
spike_server=$!
spike_pids+=("$spike_server")
for i in $(seq 1 200); do
  if grep -q 'GUEST SERVER READY' "$spike_run/server.console" 2>/dev/null; then break; fi
  sleep 0.05
done
grep 'GUEST SERVER READY' "$spike_run/server.console"
timeout 35 cloud-hypervisor --cpus boot=1 --memory size=256M --kernel "$OVERDRIVE_METAL_KERNEL" --cmdline 'root=/dev/vda rw console=ttyS0 init=/init panic=0 loglevel=4 spike_role=client' --disk "path=$spike_run/client.ext4,image_type=raw" --net 'tap=tap295a,mac=02:00:00:95:00:02,offload_tso=off,offload_ufo=off,offload_csum=off' --serial "file=$spike_run/client.console" --console off > "$spike_run/client.stderr" 2>&1 &
spike_client=$!
spike_pids+=("$spike_client")
wait "$spike_client"
wait "$spike_server"
wait "$spike_host"
grep 'GUEST ROUNDTRIP SUCCESS' "$spike_run/client.console"
sleep 0.4
for pid in "${spike_captures[@]}"; do kill -INT "$pid"; done
for pid in "${spike_captures[@]}"; do wait "$pid"; done
spike_captures=()
set +x

python3 - "$spike_run" <<'PCAP'
import pathlib,re,socket,struct,sys
root=pathlib.Path(sys.argv[1])
request=b'GH295-PLAINTEXT-GUEST-REQUEST-7\n'
response=b'GH295-BYTE-DISTINCT-PEER-RESPONSE-42\n'
host=(root/'host.log').read_text()
wire=re.search(r'LEG_B WIRE local=(\d+\.\d+\.\d+\.\d+):(\d+) peer=(\d+\.\d+\.\d+\.\d+):(\d+)',host)
assert wire, 'missing actual leg-B socket tuple'
local,localport,remote,remoteport=wire.groups()
localport,remoteport=int(localport),int(remoteport)
flows={}
tap_a_request=0
tap_b_request=0
tap_b_response=0
tap_b_direct=0
for name in ['lo','tap295a','tap295b']:
    raw=(root/f'{name}.pcap').read_bytes()
    endian='<' if raw[:4]==b'\xd4\xc3\xb2\xa1' else '>'
    linktype=struct.unpack(endian+'I',raw[20:24])[0]
    assert linktype==1, f'unsupported pcap linktype {linktype}'
    pos=24
    while pos<len(raw):
        ts,us,n,wirelen=struct.unpack(endian+'IIII',raw[pos:pos+16]);pos+=16
        frame=raw[pos:pos+n];pos+=n
        assert n==wirelen, 'truncated capture'
        if frame[12:14]!=b'\x08\x00':continue
        ip=frame[14:]
        if ip[9]!=6:continue
        src,dst=socket.inet_ntoa(ip[12:16]),socket.inet_ntoa(ip[16:20])
        ihl=(ip[0]&15)*4
        tcp=ip[ihl:struct.unpack('!H',ip[2:4])[0]]
        sport,dport,seq=struct.unpack('!HHI',tcp[:8])
        payload=tcp[(tcp[12]>>4)*4:]
        if name=='tap295a':tap_a_request+=request in payload
        if name=='tap295b':
            tap_b_direct+=src=='10.95.0.2' and dst=='10.95.0.3'
            tap_b_request+=src=='10.95.0.1' and request in payload
            tap_b_response+=src=='10.95.0.3' and response in payload
        if name=='lo' and payload:
            key=(src,sport,dst,dport)
            assert key in [(local,localport,remote,remoteport),(remote,remoteport,local,localport)],f'unexpected loopback flow {key}'
            octets=flows.setdefault(key,{})
            for offset,b in enumerate(payload):
                at=seq+offset
                assert at not in octets or octets[at]==b,'conflicting retransmission'
                octets[at]=b
assert len(flows)==2, f'expected two inter-agent stream directions: {flows.keys()}'
for key,octets in sorted(flows.items()):
    first,last=min(octets),max(octets)
    assert len(octets)==last-first+1,'gap in inter-agent stream capture'
    stream=bytes(octets[i] for i in range(first,last+1))
    cleartext=int(request in stream)+int(response in stream)
    assert cleartext==0,'cleartext on inter-agent wire'
    pos=0;counts={20:0,22:0,23:0}
    while pos<len(stream):
        assert pos+5<=len(stream),'partial TLS header'
        kind,version,n=struct.unpack('!BHH',stream[pos:pos+5])
        assert kind in counts and version in [0x0301,0x0303],f'non-TLS bytes at {pos}'
        assert pos+5+n<=len(stream),'partial TLS record'
        counts[kind]+=1;pos+=5+n
    assert counts[23]>0,'no TLS application_data records'
    print(f'WIRE_SCAN {key} stream_bytes={len(stream)} tls_records={counts} application_data_0x17={counts[23]} plaintext={cleartext} gaps=0')
print(f'SHARED_L2 source_tap_plaintext_request={tap_a_request} peer_tap_agent_plaintext_request={tap_b_request} peer_tap_plaintext_response={tap_b_response} guest_to_guest_bypass_packets={tap_b_direct}')
assert tap_a_request>0 and tap_b_request>0 and tap_b_response>0 and tap_b_direct==0
assert 'version=Some(TLSv1_3)' in host
assert host.count('MTLS_AUTHENTICATED')==2
assert host.count('KTLS_ARMED')==4
print('VERDICT=WORKS: two real microVMs, guest plaintext DNS dial, per-port broute+TPROXY, mutual TLS1.3 with kTLS TX/RX, zero cleartext on leg-B/leg-C, no direct shared-L2 bypass')
PCAP
