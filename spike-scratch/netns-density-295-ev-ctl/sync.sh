#!/bin/bash
# Push this directory's sources to the metal host's private /var/tmp/ev-ctl-work/src (tar over ssh; nothing shared is touched,
# no rsync --delete, no lease needed: it writes only inside the spike's own work dir).
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /var/tmp/ev-ctl-work/src /var/tmp/ev-ctl-work/out /var/tmp/ev-ctl-work/evidence'
tar -czf - --exclude=out --exclude=evidence --exclude=__pycache__ module test driver | ssh -o BatchMode=yes $T 'tar -C /var/tmp/ev-ctl-work/src -xzf -'
ssh -o BatchMode=yes $T 'ls /var/tmp/ev-ctl-work/src'
echo synced
