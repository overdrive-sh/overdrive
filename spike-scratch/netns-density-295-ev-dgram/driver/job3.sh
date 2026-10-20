#!/bin/bash
# one leased job on the FINAL build and harness. Steps are independent.
D=/var/tmp/ev-dgram-work/src/driver
bash $D/build.sh || { echo BUILD_FAILED; exit 2; }
for r in a b; do
echo "##### STEP qemu-correctness-$r"
python3 $D/run.py qemu 'sizes-,mixed-,oversize-chain,peer-credit,peer-chunked,peer-oversize,peer-badframe,peer-eof,peer-truncated,err-,slot-release,recvmsg4,neg-' tag=corr3$r timeout=1500
done
echo "##### STEP qemu-stalls"
python3 $D/run.py qemu 'peer-stall,stall-dest,stall-app' tag=stall3 timeout=900
echo "##### STEP ch"
python3 $D/run.py ch 'sizes-0-u,sizes-1-u,sizes-1431-u,sizes-4096-u,sizes-4097-u,sizes-59000-u,sizes-1431-c,sizes-59000-c,mixed-u,mixed-c,credit,chunked,oversize,badframe-len,badframe-magic,truncated,eof,stall' tag=ch3 per_case_timeout=160
echo "##### STEP ch-throughput"
python3 $D/run.py ch 'tp-1-64-r0,tp-1-64-r1,tp-1-1431-r0,tp-1-1431-r1,tp-1-59000-r0,tp-1-59000-r1,tp-64-64-r0,tp-64-64-r1,tp-64-1431-r0,tp-64-1431-r1,tp-64-59000-r0,tp-64-59000-r1' tag=chtp3 per_case_timeout=160
echo "##### STEP qemu-throughput"
python3 $D/run.py qemu 'tp-' tag=tp3 timeout=1800
echo "##### JOB_DONE"
