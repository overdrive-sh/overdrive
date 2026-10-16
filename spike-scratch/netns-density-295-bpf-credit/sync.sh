#!/bin/bash
# push the sources to the metal host's private dir and build there
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /tmp/bpf-credit-work/src/bpf /tmp/bpf-credit-work/src/probe /tmp/bpf-credit-work/src/elig'
scp -q -r bpf/src bpf/Cargo.toml bpf/Cargo.lock $T:/tmp/bpf-credit-work/src/bpf/
scp -q -r probe/src probe/Cargo.toml probe/Cargo.lock $T:/tmp/bpf-credit-work/src/probe/
scp -q -r elig/src elig/Cargo.toml $T:/tmp/bpf-credit-work/src/elig/
scp -q build.sh vmrun.py $T:/tmp/bpf-credit-work/
ssh $T 'bash /tmp/bpf-credit-work/build.sh'
