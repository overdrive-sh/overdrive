#!/bin/bash
# pull the spike's evidence dir from its private work dir on the metal host (read-only; no lease needed)
cd "$(dirname "$0")"
set -a; . ../../.env; set +a
ssh -o BatchMode=yes $OVERDRIVE_METAL_TARGET 'sudo -n tar -C /var/tmp/ev-ctl-work -czf - evidence' | tar -xzf - 2>/dev/null
ADDR=${OVERDRIVE_METAL_TARGET#*@}
grep -rl "$ADDR" evidence 2>/dev/null | xargs -I{} sed -i.bak "s/$ADDR/<metal-address>/g" {} 2>/dev/null
find evidence -name '*.bak' -delete
ls evidence
