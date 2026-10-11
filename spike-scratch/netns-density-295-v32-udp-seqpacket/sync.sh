#!/bin/bash
# push the sources to the metal host private dir (build runs under the lease via launch.py)
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v32-work/src'
scp -q -r bpf/src bpf/Cargo.toml bpf/Cargo.lock $OVERDRIVE_METAL_TARGET:/tmp/v32-work/src/bpf/ 2>/dev/null || { ssh $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v32-work/src/bpf'; scp -q -r bpf/src bpf/Cargo.toml bpf/Cargo.lock $OVERDRIVE_METAL_TARGET:/tmp/v32-work/src/bpf/; }
ssh $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v32-work/src/probe'
scp -q -r probe/src probe/Cargo.toml probe/Cargo.lock $OVERDRIVE_METAL_TARGET:/tmp/v32-work/src/probe/
scp -q build.sh $OVERDRIVE_METAL_TARGET:/tmp/v32-work/build.sh
scp -q vmrun.py $OVERDRIVE_METAL_TARGET:/tmp/v32-work/vmrun.py
ssh $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v32-work/cells'
scp -q cells/*.json $OVERDRIVE_METAL_TARGET:/tmp/v32-work/cells/
