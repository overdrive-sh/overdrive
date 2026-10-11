#!/bin/bash
# push only this increment dir to the metal host's private /tmp/v30-work (no tree rsync)
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'mkdir -p /tmp/v30-work/guest/src'
scp -q run.py build.sh fd-mode.patch $OVERDRIVE_METAL_TARGET:/tmp/v30-work/
scp -q guest/Cargo.toml $OVERDRIVE_METAL_TARGET:/tmp/v30-work/guest/
scp -q guest/src/main.rs $OVERDRIVE_METAL_TARGET:/tmp/v30-work/guest/src/
