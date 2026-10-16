#!/bin/bash
# push the sources to the metal host's private dir and build there
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v1-work/src'
scp -q -r bpf/src bpf/Cargo.toml bpf/Cargo.lock $OVERDRIVE_METAL_TARGET:/tmp/v1-work/src/bpf/ 2>/dev/null || { ssh $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v1-work/src/bpf'; scp -q -r bpf/src bpf/Cargo.toml bpf/Cargo.lock $OVERDRIVE_METAL_TARGET:/tmp/v1-work/src/bpf/; }
ssh $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v1-work/src/probe'
scp -q -r probe/src probe/Cargo.toml probe/Cargo.lock $OVERDRIVE_METAL_TARGET:/tmp/v1-work/src/probe/
scp -q build.sh $OVERDRIVE_METAL_TARGET:/tmp/v1-work/build.sh
ssh $OVERDRIVE_METAL_TARGET 'bash /tmp/v1-work/build.sh'
