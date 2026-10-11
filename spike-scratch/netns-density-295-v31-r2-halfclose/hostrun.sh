#!/bin/bash
# runs on the metal host as root: hostrun.sh <label> <loop args...>  -> evidence/<label>.json + compact accounting
L=$1; shift
cd /tmp/v31-work; mkdir -p evidence
ulimit -n 1048576
[ -s offs.args ] || python3 offsets.py | python3 -c 'import json,sys; o=json.load(sys.stdin); print(" ".join(f"--off-{k} {v}" for k,v in {"wseq":o["tcp_write_seq"],"trans":o["vsk_trans"],"txcnt":o["vvs_tx_cnt"],"fpriv":o["file_private"],"skt":o["socket_sk"],"cookie":o["sk_cookie"]}.items()))' > offs.args
OFFS=$(cat offs.args)
nstat -az > evidence/$L.nstat0 2>/dev/null
timeout ${V31_TIMEOUT:-1500} ${V31_BIN:-./out/drainprobe} loop --obj ${V31_OBJ:-out/drain-bpf.o} --events /tmp/v31-work/evidence/$L $OFFS "$@" > evidence/$L.json 2> evidence/$L.err
RC=$?
nstat -az > evidence/$L.nstat1 2>/dev/null
if [ -n "$BRIEF" ]; then python3 /tmp/v31-work/brief.py "$L"; else python3 /tmp/v31-work/acct.py "$L" "$RC" "$*"; fi
