#!/usr/bin/env bash
# GH #295 Phase-1 Part B: production kTLS/splice core on two shared-bridge microVM TAPs.
set -euo pipefail

spike_here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
spike_run=$(mktemp -d /var/tmp/gh295b.XXXXXXXX)
exec > >(tee -a "$spike_run/transcript.log") 2>&1
spike_start=$(python3 -c 'import time; print(time.perf_counter())')
printf 'SPIKE_RUN=%s\nSTART=%s\n' "$spike_run" "$(date -u +%FT%TZ)"
printf 'SOURCE_SHA=%s\n' "$(sha256sum "$spike_here/host.rs" "$spike_here/guest.rs" "$spike_here/run.sh" | sha256sum | awk '{print $1}')"
uname -r
uname -m
systemd-detect-virt || [ "$?" = 1 ]
cloud-hypervisor --version

export CARGO_TARGET_DIR="$PWD/target"
test -f "$PWD/target/bpf/overdrive_bpf.o"
export OVERDRIVE_BPF_OBJECT="$PWD/target/bpf/overdrive_bpf.o"
printf 'BPF_OBJECT_OVERRIDE=%s (linked crate build requirement only; this probe executes the mTLS module, not the embedded BPF object)\n' "$OVERDRIVE_BPF_OBJECT"
printf 'COMMAND build production-linked host and static credential-free guest\n'
cargo build --release --manifest-path "$spike_here/Cargo.toml" --bin host
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=cc
cargo build --release --target x86_64-unknown-linux-musl --manifest-path "$spike_here/Cargo.toml" --no-default-features --bin guest
host_bin="$CARGO_TARGET_DIR/release/host"
guest_bin="$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/guest"
file "$host_bin" "$guest_bin"

spike_stage="$spike_run/rootfs"
mkdir -p "$spike_stage"/{proc,dev,sys,tmp,etc}
install -m 0755 "$guest_bin" "$spike_stage/init"
printf 'nameserver 10.95.0.1\noptions timeout:2 attempts:1\n' > "$spike_stage/etc/resolv.conf"
mknod "$spike_stage/dev/console" c 5 1
mknod "$spike_stage/dev/null" c 1 3
truncate -s 64M "$spike_run/rootfs.ext4"
mkfs.ext4 -F -d "$spike_stage" "$spike_run/rootfs.ext4"
cp "$spike_run/rootfs.ext4" "$spike_run/client.ext4"
cp "$spike_run/rootfs.ext4" "$spike_run/server.ext4"

ip netns list | sort > "$spike_run/netns.before"
ip -br link show type veth | sort > "$spike_run/veth.before"
ip -o route show table all | sort > "$spike_run/routes.before"
find /etc/netns -mindepth 1 -maxdepth 3 -print 2>/dev/null | sort > "$spike_run/etc-netns.before"
spike_rp=$(sysctl -n net.ipv4.conf.all.rp_filter)
spike_pids=()
spike_captures=()
spike_trace_pid=""
spike_links=()
spike_table=0
spike_bridge_table=0
spike_rule=0
spike_route=0
spike_cgroups=0

cleanup() {
  spike_status=$?
  trap - EXIT
  set +e
  printf '\nCLEANUP START prior_exit=%s at=%s\n' "$spike_status" "$(date -u +%FT%TZ)"
  for pid in "${spike_pids[@]}"; do kill -TERM "$pid" 2>/dev/null; done
  for pid in "${spike_captures[@]}"; do kill -INT "$pid" 2>/dev/null; done
  [ -z "$spike_trace_pid" ] || kill -INT "$spike_trace_pid" 2>/dev/null
  sleep 0.5
  for pid in "${spike_pids[@]}"; do kill -KILL "$pid" 2>/dev/null; done
  for pid in "${spike_pids[@]}" "${spike_captures[@]}"; do wait "$pid" 2>/dev/null; done
  [ -z "$spike_trace_pid" ] || wait "$spike_trace_pid" 2>/dev/null
  [ "$spike_cgroups" = 0 ] || "$host_bin" cgroup-clean
  nft list table bridge gh295b
  nft list table ip gh295b
  ip -br addr
  ip rule
  [ "$spike_table" = 0 ] || nft delete table ip gh295b
  [ "$spike_bridge_table" = 0 ] || nft delete table bridge gh295b
  [ "$spike_rule" = 0 ] || ip rule del priority 2951 fwmark 0x295 lookup 295
  [ "$spike_route" = 0 ] || ip route del local 0.0.0.0/0 dev lo table 295
  for link in "${spike_links[@]}"; do ip link del "$link" 2>/dev/null; done
  sysctl -qw "net.ipv4.conf.all.rp_filter=$spike_rp"
  ip netns list | sort > "$spike_run/netns.after"
  ip -br link show type veth | sort > "$spike_run/veth.after"
  ip -o route show table all | sort > "$spike_run/routes.after"
  find /etc/netns -mindepth 1 -maxdepth 3 -print 2>/dev/null | sort > "$spike_run/etc-netns.after"
  diff -u "$spike_run/netns.before" "$spike_run/netns.after" > "$spike_run/netns.diff"
  diff -u "$spike_run/veth.before" "$spike_run/veth.after" > "$spike_run/veth.diff"
  diff -u "$spike_run/routes.before" "$spike_run/routes.after" > "$spike_run/routes.diff"
  diff -u "$spike_run/etc-netns.before" "$spike_run/etc-netns.after" > "$spike_run/etc-netns.diff"
  {
    printf 'OWNED_LINK_BRIDGE='; ip link show br295b >/dev/null 2>&1 && echo PRESENT || echo ABSENT
    printf 'OWNED_LINK_TAP_A='; ip link show tap295ba >/dev/null 2>&1 && echo PRESENT || echo ABSENT
    printf 'OWNED_LINK_TAP_B='; ip link show tap295bb >/dev/null 2>&1 && echo PRESENT || echo ABSENT
    printf 'OWNED_NFT_IP='; nft list table ip gh295b >/dev/null 2>&1 && echo PRESENT || echo ABSENT
    printf 'OWNED_NFT_BRIDGE='; nft list table bridge gh295b >/dev/null 2>&1 && echo PRESENT || echo ABSENT
    printf 'OWNED_RULE='; ip rule show | grep -q 'lookup 295' && echo PRESENT || echo ABSENT
    printf 'OWNED_ROUTE='; ip route show table 295 | grep -q . && echo PRESENT || echo ABSENT
    for scope in gh295b-client.scope gh295b-server.scope; do
      printf 'OWNED_CGROUP_%s=' "$scope"
      [ -e "/sys/fs/cgroup/overdrive.slice/workloads.slice/$scope" ] && echo PRESENT || echo ABSENT
    done
  } > "$spike_run/cleanup-complements.log"
  printf 'CLEANUP COMPLEMENTS\n'; cat "$spike_run/cleanup-complements.log"
  printf 'TOPOLOGY_COMPLEMENT_DIFF_BYTES netns=%s veth=%s etc_netns=%s\n' \
    "$(wc -c < "$spike_run/netns.diff")" \
    "$(wc -c < "$spike_run/veth.diff")" \
    "$(wc -c < "$spike_run/etc-netns.diff")"
  printf 'CLEANUP COMPLETE preserved_evidence=%s original_exit=%s\n' "$spike_run" "$spike_status"
  python3 -c 'import sys,time; print(f"TOTAL_ELAPSED_SECONDS={time.perf_counter()-float(sys.argv[1]):.6f}")' "$spike_start"
  exit "$spike_status"
}
trap cleanup EXIT

set -x
! ip link show br295b
! ip link show tap295ba
! ip link show tap295bb
! nft list table ip gh295b
! nft list table bridge gh295b
[ ! -e /sys/fs/cgroup/overdrive.slice/workloads.slice/gh295b-client.scope ]
[ ! -e /sys/fs/cgroup/overdrive.slice/workloads.slice/gh295b-server.scope ]
"$host_bin" cgroup-init
spike_cgroups=1
modprobe nft_tproxy
modprobe tls
ip link add br295b type bridge
spike_links+=(br295b)
ip link set br295b address 02:00:00:95:00:01
ip addr add 10.95.0.1/24 dev br295b
ip link set br295b up
for tap in tap295ba tap295bb; do
  ip tuntap add dev "$tap" mode tap
  spike_links=("$tap" "${spike_links[@]}")
  ip link set "$tap" master br295b
  ip link set "$tap" up
  sysctl -qw "net.ipv4.conf.$tap.rp_filter=0"
done
sysctl -qw net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.br295b.rp_filter=0
ip rule add priority 2951 fwmark 0x295 lookup 295
spike_rule=1
ip route add local 0.0.0.0/0 dev lo table 295
spike_route=1
nft add table bridge gh295b
spike_bridge_table=1
nft add chain bridge gh295b prerouting '{ type filter hook prerouting priority -300; policy accept; }'
nft add rule bridge gh295b prerouting iifname tap295ba ether type ip ip protocol tcp tcp dport 9000 counter meta nftrace set 1 meta mark set 0x295a ether daddr set 02:00:00:95:00:01 meta pkttype set host
nft add rule bridge gh295b prerouting iifname tap295ba ether type ip ip protocol udp udp dport 53 counter meta nftrace set 1
nft add table ip gh295b
spike_table=1
nft add chain ip gh295b prerouting '{ type filter hook prerouting priority -151; policy accept; }'
nft add rule ip gh295b prerouting iifname tap295ba ip protocol udp udp dport 53 counter
nft add rule ip gh295b prerouting meta mark 0x295a ip protocol tcp tcp dport 9000 counter tproxy to 127.0.0.1:15294 meta mark set 0x295 accept
nft add rule ip gh295b prerouting meta mark 0x295 ip daddr 10.95.0.3 tcp dport 9000 counter tproxy to 127.0.0.1:15295 accept
nft add chain ip gh295b output '{ type route hook output priority -151; policy accept; }'
nft add rule ip gh295b output meta mark != 0x2 ip daddr 10.95.0.3 tcp dport 9000 counter meta mark set 0x295 accept
ip -br addr
ip netns list
ip -br link show type veth
bridge link
ip route show table all
grep -En 'NetSlot|host_veth|ip netns|type veth|/30' "$spike_here"/*.rs "$spike_here/Cargo.toml" || true

"$host_bin" > "$spike_run/host.log" 2>&1 &
spike_host=$!
spike_pids+=("$spike_host")
for _ in $(seq 1 600); do
  set +x
  grep -q 'HOST READY' "$spike_run/host.log" && break
  kill -0 "$spike_host"
  sleep 0.05
  set -x
done
grep 'HOST READY' "$spike_run/host.log"

strace -ff -ttt -yy -s 512 -xx -e trace=clone,clone3,splice,write,writev,sendto,sendmsg -o "$spike_run/strace" -p "$spike_host" > "$spike_run/strace.stdout" 2> "$spike_run/strace.stderr" &
spike_trace_pid=$!
sleep 0.5
for iface in lo tap295ba tap295bb; do
  tcpdump --immediate-mode -U -s 0 -ni "$iface" -w "$spike_run/$iface.pcap" 'net 10.95.0.0/24 and (tcp port 9000 or udp port 53)' > "$spike_run/capture-$iface.stderr" 2>&1 &
  spike_captures+=("$!")
done
sleep 0.4

cloud-hypervisor --cpus boot=1 --memory size=256M --kernel "$OVERDRIVE_METAL_KERNEL" --cmdline 'root=/dev/vda rw console=ttyS0 init=/init panic=0 loglevel=4 spike_role=server' --disk "path=$spike_run/server.ext4,image_type=raw" --net 'tap=tap295bb,mac=02:00:00:95:00:03' --serial "file=$spike_run/server.console" --console off > "$spike_run/server.stderr" 2>&1 &
spike_server=$!
spike_pids+=("$spike_server")
"$host_bin" cgroup-place gh295b-server "$spike_server"
for _ in $(seq 1 400); do
  set +x
  grep -q 'GUEST SERVER READY' "$spike_run/server.console" 2>/dev/null && break
  kill -0 "$spike_server"
  sleep 0.05
  set -x
done
grep 'GUEST SERVER READY' "$spike_run/server.console"

cloud-hypervisor --cpus boot=1 --memory size=256M --kernel "$OVERDRIVE_METAL_KERNEL" --cmdline 'root=/dev/vda rw console=ttyS0 init=/init panic=0 loglevel=4 spike_role=client' --disk "path=$spike_run/client.ext4,image_type=raw" --net 'tap=tap295ba,mac=02:00:00:95:00:02' --serial "file=$spike_run/client.console" --console off > "$spike_run/client.stderr" 2>&1 &
spike_client=$!
spike_pids+=("$spike_client")
"$host_bin" cgroup-place gh295b-client "$spike_client"

for _ in $(seq 1 300); do
  set +x
  grep -q 'PRODUCTION_MTLS_BOTH_ESTABLISHED' "$spike_run/host.log" && break
  kill -0 "$spike_host"
  sleep 0.02
  set -x
done
grep 'PRODUCTION_MTLS_BOTH_ESTABLISHED' "$spike_run/host.log"
ss -H -n -t -i -e > "$spike_run/ss-ktls.log"
grep -A1 -B1 'tcp-ulp-tls' "$spike_run/ss-ktls.log"

for alloc_pid in "gh295b-server:$spike_server" "gh295b-client:$spike_client"; do
  alloc=${alloc_pid%%:*}
  pid=${alloc_pid##*:}
  scope="/sys/fs/cgroup/overdrive.slice/workloads.slice/$alloc.scope"
  grep -qx "$pid" "$scope/cgroup.procs"
  readlink "/proc/$pid/exe"
  cat "/proc/$pid/cgroup"
  printf 'CGROUP_PROOF alloc=%s pid=%s scope=%s cgroup_procs=%s exe=%s\n' \
    "$alloc" "$pid" "$scope" "$(tr '\n' ',' < "$scope/cgroup.procs")" "$(readlink "/proc/$pid/exe")" | tee -a "$spike_run/cgroup-proof.log"
done

wait "$spike_client"
wait "$spike_server"
wait "$spike_host"
grep 'GUEST STEADY ROUNDTRIP SUCCESS' "$spike_run/client.console"
grep 'PRODUCTION_MTLS_TEARDOWN_COMPLETE' "$spike_run/host.log"
sleep 0.4
for pid in "${spike_captures[@]}"; do kill -INT "$pid"; done
for pid in "${spike_captures[@]}"; do wait "$pid"; done
spike_captures=()
kill -INT "$spike_trace_pid" 2>/dev/null || true
wait "$spike_trace_pid" 2>/dev/null || true
spike_trace_pid=""
set +x

python3 - "$spike_run" <<'PY'
import pathlib,re,socket,struct,sys
root=pathlib.Path(sys.argv[1])
request1=b'GH295B-WARMUP-REQUEST\n'
response1=b'GH295B-WARMUP-RESPONSE\n'
request2=b'GH295B-STEADY-STATE-REQUEST-ZEROCOPY-71\n'
response2=b'GH295B-STEADY-STATE-RESPONSE-ZEROCOPY-93\n'
flows={}
tap_a_request=0
tap_b_request=0
tap_b_response=0
tap_b_direct=0
for name in ['lo','tap295ba','tap295bb']:
    raw=(root/f'{name}.pcap').read_bytes()
    endian='<' if raw[:4]==b'\xd4\xc3\xb2\xa1' else '>'
    assert struct.unpack(endian+'I',raw[20:24])[0]==1
    pos=24
    while pos<len(raw):
        _,_,n,wirelen=struct.unpack(endian+'IIII',raw[pos:pos+16]);pos+=16
        frame=raw[pos:pos+n];pos+=n
        assert n==wirelen
        if frame[12:14]!=b'\x08\x00': continue
        ip=frame[14:]
        if ip[9]!=6: continue
        src,dst=socket.inet_ntoa(ip[12:16]),socket.inet_ntoa(ip[16:20])
        ihl=(ip[0]&15)*4
        tcp=ip[ihl:struct.unpack('!H',ip[2:4])[0]]
        sport,dport,seq=struct.unpack('!HHI',tcp[:8])
        payload=tcp[(tcp[12]>>4)*4:]
        if name=='tap295ba': tap_a_request += request2 in payload
        if name=='tap295bb':
            tap_b_direct += src=='10.95.0.2' and dst=='10.95.0.3'
            tap_b_request += src=='10.95.0.1' and request2 in payload
            tap_b_response += src=='10.95.0.3' and response2 in payload
        if name=='lo' and payload and (sport==9000 or dport==9000):
            octets=flows.setdefault((src,sport,dst,dport),{})
            for offset,b in enumerate(payload):
                at=seq+offset
                assert at not in octets or octets[at]==b, 'conflicting retransmission'
                octets[at]=b
assert len(flows)==2, f'expected two leg-B/C stream directions, got {flows.keys()}'
for key,octets in sorted(flows.items()):
    first,last=min(octets),max(octets)
    assert len(octets)==last-first+1, 'gap in leg-B/C stream'
    stream=bytes(octets[i] for i in range(first,last+1))
    clear=sum(marker in stream for marker in [request1,response1,request2,response2])
    assert clear==0, 'application cleartext on leg-B/C wire'
    pos=0; counts={20:0,22:0,23:0}
    while pos<len(stream):
        assert pos+5<=len(stream), 'partial TLS header'
        kind,version,length=struct.unpack('!BHH',stream[pos:pos+5])
        assert kind in counts and version in [0x0301,0x0303], (kind,version,pos)
        assert pos+5+length<=len(stream), 'partial TLS record'
        counts[kind]+=1; pos+=5+length
    assert counts[23]>0
    print(f'WIRE_SCAN {key} stream_bytes={len(stream)} tls_records={counts} application_data_0x17={counts[23]} plaintext={clear} gaps=0')
print(f'SHARED_L2 source_tap_steady_request={tap_a_request} peer_tap_agent_steady_request={tap_b_request} peer_tap_steady_response={tap_b_response} guest_to_guest_bypass_packets={tap_b_direct}')
assert tap_a_request>0 and tap_b_request>0 and tap_b_response>0 and tap_b_direct==0

trace='\n'.join(p.read_text(errors='replace') for p in root.glob('strace*') if p.name not in {'strace.stdout','strace.stderr'})
successful_splices=[line for line in trace.splitlines() if 'splice(' in line and re.search(r'= [1-9][0-9]*$',line)]
assert successful_splices, 'no successful production splice syscall captured'
def rendered_hex(blob): return ''.join(f'\\x{b:02x}' for b in blob)
for marker in [request2,response2]:
    assert marker.decode() not in trace
    assert rendered_hex(marker) not in trace, f'steady marker copied by a traced host write: {marker!r}'
print(f'ZERO_COPY_STRACE successful_splice_syscalls={len(successful_splices)} steady_request_host_write_hits=0 steady_response_host_write_hits=0')

ss=(root/'ss-ktls.log').read_text()
records=ss.splitlines()
ulp=sum('tcp-ulp-tls' in line and ('rxconf: sw' in line or 'rxconf:sw' in line) and ('txconf: sw' in line or 'txconf:sw' in line) for line in records)
assert ulp>=2, f'expected both agent kTLS sockets in ss, got {ulp}\n{ss}'
print(f'KTLS_SS bidirectional_tls13_socket_records={ulp}')
print('VERDICT=WORKS: actual production HostMtlsEnforcement executed TLS1.3 kTLS TX/RX plus zero-copy splice in both directions across two real shared-bridge microVM TAPs; no netns/veth/NetSlot/host_veth or /30 final-path dependency; direct L2 bypass absent')
PY

printf '\nHOST LOG\n'; cat "$spike_run/host.log"
printf '\nCLIENT CONSOLE\n'; cat "$spike_run/client.console"
printf '\nSERVER CONSOLE\n'; cat "$spike_run/server.console"
printf '\nCAPTURE STATS\n'
for iface in lo tap295ba tap295bb; do cat "$spike_run/capture-$iface.stderr"; done
printf '\nNFT COUNTERS\n'; nft list table bridge gh295b; nft list table ip gh295b
printf 'FINAL_RUN_DIR=%s\n' "$spike_run"
