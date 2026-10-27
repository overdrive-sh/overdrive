#!/usr/bin/env bash
set -euo pipefail

sudo -n tcpdump -i lo -nn -tttt -l 'tcp[tcpflags] & 0x05 != 0' &
capture_pid=$!
cleanup() {
    sudo -n kill -INT "$capture_pid" 2>/dev/null || true
    wait "$capture_pid" 2>/dev/null || true
}
trap cleanup EXIT

sleep 1
set +e
cargo nextest run -p overdrive-worker --features integration-tests \
    -E 'test(dropping_a_claimed_registration_with_the_loopback_leg_half_ended_resets_the_remote_peer)' \
    --no-fail-fast >/dev/null 2>&1
test_status=$?
set -e
sleep 1
cleanup
trap - EXIT
printf 'unchanged_test_exit=%s\n' "$test_status"
