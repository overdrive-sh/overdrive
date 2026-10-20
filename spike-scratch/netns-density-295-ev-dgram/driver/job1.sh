#!/bin/bash
# one leased job: build, Env 1 correctness, Env 1 stalls, Env 2 (Cloud Hypervisor guest) battery. Steps are independent (no set -e).
D=/var/tmp/ev-dgram-work/src/driver
bash $D/build.sh || { echo BUILD_FAILED; exit 2; }
echo "##### STEP qemu-correctness"
python3 $D/run.py qemu 'sizes-,mixed-,oversize-chain,peer-credit,peer-chunked,peer-oversize,peer-badframe,peer-eof,err-,slot-release,recvmsg4,neg-' tag=corr1 timeout=1500
echo "##### STEP qemu-stalls"
python3 $D/run.py qemu 'peer-stall,stall-dest,stall-app' tag=stall1 timeout=900
echo "##### STEP ch"
python3 $D/run.py ch 'sizes-0-u,sizes-1431-c,sizes-59000-u,mixed-c,credit,chunked,oversize,badframe-len,badframe-magic,eof,stall' tag=ch1 per_case_timeout=160
echo "##### JOB_DONE"
