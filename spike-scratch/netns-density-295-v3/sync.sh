#!/bin/bash
# push the sources to the metal host's private dir and build there
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /tmp/v3-work/src/bpf /tmp/v3-work/src/probe'
scp -q -r bpf/src bpf/Cargo.toml bpf/Cargo.lock $T:/tmp/v3-work/src/bpf/
scp -q -r probe/src probe/Cargo.toml probe/Cargo.lock $T:/tmp/v3-work/src/probe/
scp -q build.sh vmrun.py $T:/tmp/v3-work/
ssh $T 'bash /tmp/v3-work/build.sh'
