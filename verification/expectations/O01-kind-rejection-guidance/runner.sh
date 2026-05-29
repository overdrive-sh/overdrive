# shellcheck shell=bash
# O01 — Job/Schedule + probe rejected with actionable guidance.
# Client-side fast-fail: runnable without a running control plane.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

tmp="$EVIDENCE_DIR/specs"
mkdir -p "$tmp"

cat >"$tmp/job-with-probe.toml" <<'TOML'
[job]
name = "batch-thing"

[exec]
command = ["/bin/true"]

[[health_check.startup]]
type = "tcp"
port = 8080
TOML

cat >"$tmp/schedule-with-probe.toml" <<'TOML'
[schedule]
name = "nightly-thing"
cron = "0 2 * * *"

[exec]
command = ["/bin/true"]

[[health_check.startup]]
type = "tcp"
port = 8080
TOML

cat >"$tmp/service-with-probe.toml" <<'TOML'
[service]
name = "payments"

[exec]
command = ["/bin/sleep", "3600"]

[[listener]]
port = 8080

[[health_check.startup]]
type = "tcp"
port = 8080
TOML

rc=0

# Sub-claim 1: job + probe must reject with guidance naming the kind.
capture job_probe od deploy "$tmp/job-with-probe.toml" || true
evidence_contains job_probe "job"       || rc=1
# guidance text from S-SHCP-PARSE-05; reviewer confirms it is actionable.
grep -qiE 'completion|no readiness|guidance' "$EVIDENCE_DIR/job_probe.out" \
  && echo "  [PASS] job_probe.out carries guidance text" \
  || { echo "  [FAIL] job_probe.out has no guidance text"; rc=1; }

# Sub-claim 2: schedule + probe must reject with guidance naming the kind.
capture schedule_probe od deploy "$tmp/schedule-with-probe.toml" || true
evidence_contains schedule_probe "schedule" || rc=1
grep -qiE 'per-fire|composes|guidance' "$EVIDENCE_DIR/schedule_probe.out" \
  && echo "  [PASS] schedule_probe.out carries guidance text" \
  || { echo "  [FAIL] schedule_probe.out has no guidance text"; rc=1; }

# Sub-claim 3 (regression guard): service + probe must NOT fail at the kind
# gate. A later failure (e.g. control plane unreachable) is recorded but does
# not fail this sub-claim — we only assert the kind-rejection text is absent.
capture service_probe od deploy "$tmp/service-with-probe.toml" || true
evidence_absent service_probe "ProbesNotAllowedOnKind" || rc=1

echo "O01 sub-claim aggregate exit: $rc"
exit "$rc"
