#!/bin/bash
D=/var/tmp/ev-dgram-work/src/driver
bash $D/build.sh || { echo BUILD_FAILED; exit 2; }
echo "##### STEP qemu-attach"
python3 $D/run.py qemu 'attach-errnos,vsock-small-window,peer-truncated,sizes-1431-u' tag=attach6 timeout=600
echo "##### JOB_DONE"
