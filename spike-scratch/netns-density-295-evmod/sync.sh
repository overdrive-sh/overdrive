#!/bin/bash
# Push this directory's sources to the metal host's private /tmp/evmod-work/src (tar over ssh; nothing shared is touched,
# no rsync --delete). Usage: sync.sh [build-cmd-on-host...]
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /tmp/evmod-work/src /tmp/evmod-work/out /tmp/evmod-work/evidence'
tar -czf - --exclude=out --exclude=evidence --exclude=target --exclude=__pycache__ module bench bench-v3 probe driver 2>/dev/null | ssh -o BatchMode=yes $T 'tar -C /tmp/evmod-work/src -xzf -'
echo synced
