#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh loop crashfin2 crashfin2.json
