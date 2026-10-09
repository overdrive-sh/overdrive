#!/usr/bin/env bash
# V-16 physical-NIC data point (throwaway). Run as root on the metal host. The probe attaches the D5a program (TCX egress,
# tuple-gated) to the uplink for the duration of the run; the link is process-owned and un-pinned, so it detaches when the
# probe exits (watchdog: timeout 120). Only traffic to 198.51.100.16 (TEST-NET-2) on registered ports is touched.
# usage: run-nic.sh <outdir> <ifname> <srcip> [variant: plain|unreg]
set -u
OUT=${1:?}; IF=${2:?}; SRC=${3:?}; VAR=${4:-reg}
BIN=/tmp/v16-out/v16; OBJ=/tmp/v16-out/v16-bpf.o
DST=198.51.100.16
mkdir -p "$OUT"
echo "== V-16 NIC run $(date -u +%FT%TZ) variant=$VAR"
echo "uname -r: $(uname -r)  virt: $(systemd-detect-virt)"
echo "-- ethtool -i $IF"; ethtool -i $IF | head -4
echo "-- ethtool -k $IF"; ethtool -k $IF | egrep "udp|tcp-segm|generic-seg|scatter-gather:|tx-checksumming|gso|rx-gro-list"
echo "-- mtu: $(cat /sys/class/net/$IF/mtu)"
echo "-- tcx before:"; bpftool net show dev $IF 2>&1 | sed -n 1,8p
"$BIN" cases all > "$OUT/cases.txt"
tcpdump -i $IF -s 0 -U -w "$OUT/nic-$VAR.pcap" "udp and dst host $DST" 2> "$OUT/tcpdump-$VAR.err" &
TP=$!
sleep 1.5
flags=""; [ "$VAR" = unreg ] && flags="--unregistered"
timeout 120 "$BIN" tx "$OBJ" $IF $SRC $DST all $flags
echo "tx exit: $?"
sleep 1; kill $TP; wait $TP 2>/dev/null
cat "$OUT/tcpdump-$VAR.err" | tail -3
echo "-- tcx after:"; bpftool net show dev $IF 2>&1 | sed -n 1,8p
python3 /tmp/v16-out/verify_pcap.py "$OUT/nic-$VAR.pcap" "$OUT/cases.txt"
