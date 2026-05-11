Feature: alloc status — kind-aware render
  # Trace: J-OPS-003 (primary), J-OPS-002 (secondary)
  # User quote: "in the terms of a job, it is actually correct behavior. what should
  # happen, is when we check the status using overdrive alloc status --job <id> then
  # it should show that it failed during execution"
  # Cross-references: journey-alloc-status.yaml

  Background:
    Given the operator config at "~/.overdrive/config" names a running control plane

  Scenario: Service alloc status shows replicas, state, restarts, uptime
    Given a Service "payments" running stably with 1 of 1 replicas
    When the operator runs "overdrive alloc status --job payments"
    Then the output contains "kind: Service"
    And the output contains "Replicas (desired/running): 1/1"
    And the output's per-alloc table has columns "Alloc, State, Restarts, Since"
    And the output's per-alloc table contains NO column named "Exit"

  Scenario: Service alloc status renders Listeners section byte-equal to submit echo
    Given a Service "frontend" with two listeners (8080/tcp/10.0.0.1 pinned, 8081/udp pending) was submitted
    When the operator runs "overdrive alloc status --job frontend"
    Then the output contains "kind: Service"
    And the output contains a "Listeners:" section
    And both listener triples appear in declaration order
    And the pinned-VIP line equals "10.0.0.1:8080/tcp"
    And the pending-VIP line equals "(vip: pending allocation - see #167):8081/udp"
    And every line in the Listeners section byte-equals the corresponding submit echo line

  Scenario: Job alloc status shows verdict and per-attempt exit codes (Failed path)
    Given a Job "coinflip" whose three attempts all exited 1 and the reconciler emitted BackoffExhausted
    When the operator runs "overdrive alloc status --job coinflip"
    Then the output contains "kind: Job"
    And the output contains "Verdict: Failed (backoff exhausted)"
    And the output's per-attempt table has columns "Attempt, State, Exit, Started, Duration"
    And every Failed attempt row has Exit "1"
    And the output includes the stderr tail of the last attempt

  Scenario: Job alloc status shows Verdict Succeeded with exit 0 (Succeeded path)
    Given a Job "coinflip" whose first attempt exited 0 within 1.2 seconds
    When the operator runs "overdrive alloc status --job coinflip"
    Then the output contains "kind: Job"
    And the output contains "Verdict: Succeeded"
    And the attempts table contains exactly one row with State "Succeeded" and Exit "0"

  Scenario: Job alloc status shows Verdict In progress for a mid-flight Job
    Given a Job whose first attempt is currently Running and has not yet exited
    When the operator runs "overdrive alloc status --job <id>"
    Then the output contains "kind: Job"
    And the output contains "Verdict: In progress"
    And the attempts table's Exit column shows "—" (em-dash) for the running attempt

  Scenario: Schedule alloc status names the deferral with a tracking URL
    Given a registered Scheduled Job whose execution is deferred
    When the operator runs "overdrive alloc status --job <id>"
    Then the output contains "kind: Schedule"
    And the output contains the cron expression from the original spec
    And the output contains "Schedule execution is not yet implemented"
    And the output contains the deferral tracking issue URL

  Scenario: alloc status for a Job NEVER renders Service phrasing
    Given any Job-kind workload at any state
    When the operator runs "overdrive alloc status --job <id>"
    Then no line of output contains the substring "is running with"
    And no line of output contains the substring "(took live)"

  Scenario: alloc status for an unknown job name yields a typed not-found error
    Given no job named "ghost" exists in the IntentStore
    When the operator runs "overdrive alloc status --job ghost"
    Then the CLI prints a not-found error naming "ghost"
    And the CLI exits with a non-zero status code
