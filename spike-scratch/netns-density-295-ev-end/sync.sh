#!/bin/bash
# Push this directory's sources to the metal host's private /var/tmp/ev-end-work/src (tar over ssh; nothing shared is touched,
# no rsync --delete). No execution happens here; execution goes through launch.py (the canonical lease).
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /var/tmp/ev-end-work/src /var/tmp/ev-end-work/out /var/tmp/ev-end-work/evidence'
tar -czf - --exclude=out --exclude=evidence --exclude=target --exclude=__pycache__ module harness driver | ssh -o BatchMode=yes $T 'tar -C /var/tmp/ev-end-work/src -xzf -'
ssh -o BatchMode=yes $T 'ls /var/tmp/ev-end-work/src'
echo synced
