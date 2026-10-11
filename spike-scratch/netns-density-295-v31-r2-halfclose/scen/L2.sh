#!/bin/bash
cd /tmp/v31-work
ulimit -n 1048576
bash batchB.sh
bash batchC.sh
