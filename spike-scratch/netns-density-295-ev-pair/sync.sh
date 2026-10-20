#!/bin/bash
# Push module/probe/driver to the metal host's private /var/tmp/ev-pair-work/src (tar over ssh; nothing shared is touched, no rsync --delete).
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /var/tmp/ev-pair-work/src /var/tmp/ev-pair-work/out /var/tmp/ev-pair-work/evidence'
tar -czf - --exclude=out --exclude=evidence --exclude=__pycache__ module probe driver runlists | ssh -o BatchMode=yes $T 'tar -C /var/tmp/ev-pair-work/src -xzf -'
ssh -o BatchMode=yes $T 'ls /var/tmp/ev-pair-work/src'
echo synced
