#!/bin/bash
# push the sources to the metal host's private dir (/tmp/v28-work) and build there. No shared path touched.
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v28-work/src/probe /tmp/v28-work/src/bpf /tmp/v28-work/out /tmp/v28-work/evidence'
tar -C probe -cf - src Cargo.toml $( [ -f probe/Cargo.lock ] && echo Cargo.lock ) | ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'tar -C /tmp/v28-work/src/probe -xf -'
tar -C bpf -cf - src Cargo.toml | ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'tar -C /tmp/v28-work/src/bpf -xf -'
scp -q build.sh run.py $OVERDRIVE_METAL_TARGET:/tmp/v28-work/
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'bash /tmp/v28-work/build.sh'
