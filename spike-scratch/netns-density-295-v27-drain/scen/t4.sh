#!/bin/bash
# T4 (goal exactness extras) and T6 (failure path), loopback vsock
cd /tmp/v27-work
export V27_TIMEOUT=600
C="--diag-seq --io-timeout 20 --events-sample 500"
# --- T4(a): FIN-only vs FIN-on-data calibration on the wire (tcpdump on lo, intake port only)
for FIN in data only; do
  tcpdump -i lo -nn -l -q "tcp dst port 17001 and tcp[tcpflags] & tcp-fin != 0" -w evidence/t4-fin-$FIN.pcap 2>/dev/null &
  TD=$!
  sleep 1
  bash hostrun.sh t4-fin-$FIN --arm owner --trials 300 --conc 16 --flows 64 --n 2000 --m 2000 --fin $FIN $C > /dev/null
  sleep 1; kill $TD; wait $TD 2>/dev/null
  python3 - <<PY
import struct,sys
# minimal pcap reader: count FIN segments to :17001 with and without payload
f=open("evidence/t4-fin-$FIN.pcap","rb").read()
magic=struct.unpack("<I",f[:4])[0]; off=24; fin_data=fin_only=0
while off+16<=len(f):
    ts,tu,incl,orig=struct.unpack("<IIII",f[off:off+16]); off+=16; p=f[off:off+incl]; off+=incl
    if len(p)<14+20+20: continue
    ip=p[14:]; ihl=(ip[0]&15)*4; tot=struct.unpack(">H",ip[2:4])[0]; tcp=ip[ihl:]; doff=(tcp[12]>>4)*4
    pl=tot-ihl-doff
    if tcp[13]&1:
        if pl>0: fin_data+=1
        else: fin_only+=1
print("WIRE t4-fin-$FIN: FIN segments to intake port: with_payload=%d bare_FIN=%d"%(fin_data,fin_only))
PY
done
# --- T4(b): the same, larger sample, kernel-visible overlap diagnostics on
bash hostrun.sh t4-findata-big --arm owner --trials 20000 --conc 256 --flows 512 --n small --m small --fin data $C
bash hostrun.sh t4-finonly-big --arm owner --trials 20000 --conc 256 --flows 512 --n small --m small --fin only $C
# --- T4(c): 12 x 10k with a large intake SO_RCVBUF (no zero-window before accept)
for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
  bash hostrun.sh t4-bigrcvbuf-$i --arm owner --trials 10000 --conc 256 --flows 512 --seed $((100+i)) --intake-rcvbuf 4194304 $C | grep -E "^===|^trials|VIOLATIONS" | sed -E 's/args:.*//'
done
# --- T4(d): flow vsock installed AFTER Paired (backlog queued at install): a server banner is already on the vsock
for B in 100000; do
 for LATE in 0 1 2; do
  bash hostrun.sh t4-vlate$LATE-banner$B --arm owner --v-late $LATE --srv-banner $B --trials 400 --conc 32 --flows 64 --n small --m small --io-timeout 8 --settle-s 4 --diag-seq --events-sample 50
 done
done
# --- T4(e): sub-rcvlowat tail on the vsock legs (SO_RCVLOWAT=64 KiB on both flow vsocks)
bash hostrun.sh t4-rcvlowat64k --arm owner --v-rcvlowat 65536 --trials 600 --conc 32 --flows 64 --n small --m small --io-timeout 5 --settle-s 3 --diag-seq --events-sample 50
