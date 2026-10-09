#!/bin/sh
# rsync wrapper for metal sync: never touch root-owned evidence directories or the scratch
# trees of other spikes that run on the host concurrently (V-22, V-25, V-26 probes); never
# sync this spike's build output.
exec /opt/homebrew/bin/rsync \
  --exclude="/spike-scratch/netns-density-295-v24/out/" \
  --exclude="/spike-scratch/netns-density-295-v22*/" \
  --exclude="/spike-scratch/netns-density-295-v25*/" \
  --exclude="/spike-scratch/netns-density-295-v26*/" \
  --exclude=".code-review-graph/" \
  --exclude="__pycache__/" \
  --exclude="/spike-scratch/*/increment-*/evidence*/" "$@"
