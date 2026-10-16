#!/bin/bash
# push the sources to the metal host's private dir and build there
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /tmp/valve-work/src/bpf /tmp/valve-work/src/probe'
scp -q -r bpf/src bpf/Cargo.toml bpf/Cargo.lock $T:/tmp/valve-work/src/bpf/
scp -q -r probe/src probe/Cargo.toml probe/Cargo.lock $T:/tmp/valve-work/src/probe/
scp -q build.sh vmrun.py $T:/tmp/valve-work/
ssh $T 'bash /tmp/valve-work/build.sh'
