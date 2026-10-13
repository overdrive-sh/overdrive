#!/usr/bin/env bash
# go.sh <phase>... : run phases through Lima as root, trimming xtask chatter, saving to out/<phase>.txt
D=/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-v33-legb-mark
cd /Users/marcus/conductor/workspaces/helios/wellington-v2
for p in "$@"; do
  cargo xtask lima run -- bash -lc "timeout 90 bash $D/run.sh $p" 2>&1 | grep -v "^\s*Finished\|Running .target/debug\|^/opt/homebrew\|xtask: running\|sudo: preserving" > $D/out/$p.txt
  echo "wrote out/$p.txt"
done
