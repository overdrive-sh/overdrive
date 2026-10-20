#!/bin/bash
# one leased job: final probe. Steps are independent.
D=/var/tmp/ev-dgram-work/src/driver
bash $D/build.sh || { echo BUILD_FAILED; exit 2; }
echo "##### STEP qemu-correctness"
python3 $D/run.py qemu 'sizes-,mixed-,oversize-chain,peer-credit,peer-chunked,peer-oversize,peer-badframe,peer-eof,peer-truncated,egress-,err-,slot-release,recvmsg4,neg-,early' tag=corr5 timeout=1500
echo "##### STEP qemu-churn"
python3 $D/run.py qemu 'churn' tag=churn5 timeout=900
echo "##### STEP qemu-stalls"
python3 $D/run.py qemu 'peer-stall,stall-dest,stall-app' tag=stall5 timeout=900
echo "##### STEP ch"
python3 $D/run.py ch 'sizes-0-u,sizes-1-u,sizes-1431-u,sizes-4096-u,sizes-4097-u,sizes-59000-u,sizes-1431-c,sizes-59000-c,mixed-u,mixed-c,credit,chunked,oversize,badframe-len,badframe-magic,truncated,truncatedhdr,eof,stall' tag=ch5 per_case_timeout=160
echo "##### STEP qemu-throughput"
python3 $D/run.py qemu 'tp-' tag=tp5 timeout=1800
echo "##### JOB_DONE"
