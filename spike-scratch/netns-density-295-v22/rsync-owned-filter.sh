#!/bin/sh
# rsync wrapper for metal sync: never touch root-owned evidence dirs (any spike's) or pycache
# that concurrent runs leave on the host.
exec /opt/homebrew/bin/rsync --exclude=.code-review-graph/ --exclude="/spike-scratch/*/increment-*/evidence*/" --exclude="/spike-scratch/*/out/" --exclude="__pycache__/" "$@"
