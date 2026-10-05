#!/bin/sh
exec /opt/homebrew/bin/rsync --exclude=.code-review-graph/ --exclude="/spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-*/evidence/" "$@"
