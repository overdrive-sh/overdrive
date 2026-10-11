#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh loop crashfin3 crashfin3.json
