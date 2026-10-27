#!/usr/bin/env bash
# SPIKE V-MDR-1: prove a transient systemd service survives destruction of its
# systemd-run --pipe controller after detaching all inherited descriptors.
# Throwaway validation only; this is not production code or a test lane.
set -euo pipefail

probe_dir="$(mktemp -d /tmp/overdrive-metal-v-mdr1.XXXXXX)"
unit="overdrive-metal-v-mdr1-$(date -u +%Y%m%dT%H%M%SZ)-$$.service"
controller_pid=""
reader_pid=""

cleanup() {
  if [ -n "${controller_pid}" ] && kill -0 "${controller_pid}" 2>/dev/null; then
    kill -KILL "${controller_pid}" 2>/dev/null || true
    wait "${controller_pid}" 2>/dev/null || true
  fi
  if [ -n "${reader_pid}" ] && kill -0 "${reader_pid}" 2>/dev/null; then
    kill -KILL "${reader_pid}" 2>/dev/null || true
    wait "${reader_pid}" 2>/dev/null || true
  fi
  systemctl stop "${unit}" >/dev/null 2>&1 || true
  systemctl reset-failed "${unit}" >/dev/null 2>&1 || true
  rm -rf "${probe_dir}"
}
trap cleanup EXIT INT TERM HUP

wait_for_value() {
  path="$1"
  expected="$2"
  for _attempt in $(seq 1 200); do
    if [ -r "${path}" ] && [ "$(sed -n '1p' "${path}")" = "${expected}" ]; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

for required in systemd-run systemctl flock mkfifo; do
  command -v "${required}" >/dev/null
done

printf 'V_MDR1_BEGIN\n'
printf 'kernel=%s\n' "$(uname -r)"
printf 'systemd=%s\n' "$(systemd-run --version | sed -n '1p')"
printf 'unit=%s\n' "${unit}"
printf 'probe_dir=%s\n' "${probe_dir}"

mkfifo "${probe_dir}/control" "${probe_dir}/controller-output"

cat "${probe_dir}/controller-output" >"${probe_dir}/controller.capture" &
reader_pid=$!

systemd-run \
  --quiet \
  --unit="${unit}" \
  --pipe \
  --wait \
  --collect \
  --expand-environment=no \
  --property=Type=exec \
  --property=KillMode=mixed \
  bash -c '
    set -euo pipefail
    probe_dir="$1"
    exec 8>"${probe_dir}/fixture.lock"
    flock 8
    printf "acquired\n" >"${probe_dir}/phase"
    IFS= read -r control
    [ "${control}" = START ]
    trap "" PIPE
    exec </dev/null >"${probe_dir}/service.log" 2>&1
    printf "running\n" >"${probe_dir}/phase"
    printf "service-detached\n"
    sleep 5
    printf "complete\n" >"${probe_dir}/terminal"
    printf "service-complete\n"
  ' v-mdr1-service "${probe_dir}" \
  <"${probe_dir}/control" >"${probe_dir}/controller-output" 2>&1 &
controller_pid=$!

exec 9>"${probe_dir}/control"
wait_for_value "${probe_dir}/phase" acquired
printf 'phase_before_start=acquired\n'
printf 'START\n' >&9
exec 9>&-

wait_for_value "${probe_dir}/phase" running
printf 'phase_before_disconnect=running\n'
active_before="$(systemctl show "${unit}" --property=ActiveState --value)"
printf 'active_before_disconnect=%s\n' "${active_before}"

kill -KILL "${reader_pid}" "${controller_pid}"
wait "${reader_pid}" 2>/dev/null || true
wait "${controller_pid}" 2>/dev/null || true
reader_pid=""
controller_pid=""
printf 'fixture_controller_destroyed=yes\n'

active_after="$(systemctl show "${unit}" --property=ActiveState --value)"
printf 'active_after_disconnect=%s\n' "${active_after}"
[ "${active_after}" = active ]

if flock -n "${probe_dir}/fixture.lock" -c true; then
  printf 'lock_held_after_disconnect=no\n'
  exit 1
else
  printf 'lock_held_after_disconnect=yes\n'
fi

wait_for_value "${probe_dir}/terminal" complete
printf 'terminal=%s\n' "$(sed -n '1p' "${probe_dir}/terminal")"

if flock -n "${probe_dir}/fixture.lock" -c true; then
  printf 'lock_released_after_completion=yes\n'
else
  printf 'lock_released_after_completion=no\n'
  exit 1
fi

printf 'service_log_begin\n'
sed -n '1,20p' "${probe_dir}/service.log"
printf 'service_log_end\n'
printf 'V_MDR1_VERDICT=WORKS\n'
