#!/bin/bash
# Push this directory's sources to the metal host's private /var/tmp/ev-dgram-work/src (tar over ssh; nothing shared is touched,
# no rsync --delete).
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /var/tmp/ev-dgram-work/src /var/tmp/ev-dgram-work/out /var/tmp/ev-dgram-work/evidence'
tar -czf - --exclude=out --exclude=evidence --exclude=target --exclude=__pycache__ --exclude='*.log' module bpf probe driver 2>/dev/null | ssh -o BatchMode=yes $T 'tar -C /var/tmp/ev-dgram-work/src -xzf -'
ssh -o BatchMode=yes $T 'ls /var/tmp/ev-dgram-work/src'
echo synced
