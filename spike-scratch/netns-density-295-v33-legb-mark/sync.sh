#!/bin/bash
# push probe sources to the metal host's private dir /tmp/v33-work (no shared path touched) and build there.
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v33-work/src /tmp/v33-work/out /tmp/v33-work/evidence'
scp -q src/main.rs $OVERDRIVE_METAL_TARGET:/tmp/v33-work/src/main.rs
scp -q Cargo.metal.toml $OVERDRIVE_METAL_TARGET:/tmp/v33-work/Cargo.toml
scp -q run.sh s8.sh metal-build.sh $OVERDRIVE_METAL_TARGET:/tmp/v33-work/
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'bash /tmp/v33-work/metal-build.sh'
