#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
bash runplan2.sh loop hunt2 hunt2.json
bash runplan2.sh loop loopkill2 loopkill2.json
