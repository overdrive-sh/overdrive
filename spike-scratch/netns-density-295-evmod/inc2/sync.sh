#!/bin/bash
# Push this directory's sources to the metal host's private /var/tmp/evmod2-work/src (tar over ssh; nothing shared is touched,
# no rsync --delete). /var/tmp is on disk (the host's /tmp is a 32 GiB tmpfs that counts against RAM and is wiped on reboot).
# bench-v3 and valve are included (the first run of the previous increment failed because bench-v3 was missing from this list).
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../../../.env; set +a
T=$OVERDRIVE_METAL_TARGET
ssh -o BatchMode=yes $T 'mkdir -p /var/tmp/evmod2-work/src /var/tmp/evmod2-work/out /var/tmp/evmod2-work/evidence'
tar -czf - --exclude=out --exclude=evidence --exclude=target --exclude=__pycache__ module module-ktls module-audit bench bench-v3 probe driver valve | ssh -o BatchMode=yes $T 'tar -C /var/tmp/evmod2-work/src -xzf -'
ssh -o BatchMode=yes $T 'ls /var/tmp/evmod2-work/src'
echo synced
