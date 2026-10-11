#!/bin/bash
# V-29: push this directory (sources only) to the metal host's PRIVATE dir /tmp/v29-work. Never touches the shared tree.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
set -a; . "$HERE/../../.env"; set +a
T="$OVERDRIVE_METAL_TARGET"
tar -C "$HERE" --exclude=out --exclude='*/evidence' --exclude=target --exclude=__pycache__ --exclude=.DS_Store -czf /tmp/v29-src.tgz .
ssh -o BatchMode=yes "$T" 'mkdir -p /tmp/v29-work'
scp -q -o BatchMode=yes /tmp/v29-src.tgz "$T:/tmp/v29-work/src.tgz"
ssh -o BatchMode=yes "$T" 'cd /tmp/v29-work && tar -xzf src.tgz && ls'
