Feature: Submit a Job (run-to-completion)
  # Trace: J-OPS-002 (primary) — the "trust what the CLI tells me" job
  # Closes: docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md
  # Kind: Job — exit 0 = success, non-zero = failure (after backoff)
  # Cross-references: journey-submit-job.yaml

  Background:
    Given the operator config at "~/.overdrive/config" names a running control plane

  Scenario: A Job spec is recognised by [job] section presence
    Given a TOML file at "./coinflip.toml" with [job], [exec], [resources] and no [service] / [schedule] blocks
    When the parser reads the file
    Then a Job-kind workload spec is constructed
    And the constructed spec carries no Service or Schedule fields

  Scenario: A spec with both [job] and [service] is rejected with named guidance
    Given a TOML file containing both [job] and [service] blocks
    When the operator runs "overdrive job submit ./mixed.toml"
    Then the CLI prints an error naming both sections as the conflict
    And the CLI suggests "exactly one of [service] or [job] is required"
    And the CLI exits with a non-zero status code

  Scenario: Submit echoes "Job, run-to-completion" before streaming
    Given a Job spec at "./coinflip.toml"
    When the operator runs "overdrive job submit ./coinflip.toml"
    Then the CLI prints "Submitting job 'coinflip' (kind=Job, run-to-completion)"
    And the echoed spec_digest matches the operator's local computation

  Scenario: A Job that exits 0 reports Succeeded with exit_code and duration
    Given a Job spec whose workload exits 0 on its first attempt within 1.2 seconds
    When the streaming submit observes the exit
    Then the CLI prints a single terminal line "Job 'coinflip' succeeded."
    And the line names exit code 0
    And the line names a measured duration (not the literal "live")
    And the CLI process exits with status 0

  Scenario: A Job that exits non-zero on every attempt reports Failed with exit_code and attempts
    Given a Job spec whose workload exits 1 on every attempt up to backoff_limit
    When the streaming submit observes the final BackoffExhausted
    Then the CLI prints a single terminal line "Job 'coinflip' failed."
    And the line names exit code 1
    And the line names attempts as "3 of 3 (backoff exhausted)"
    And the line includes the stderr tail
    And the CLI process exits with a non-zero status code

  Scenario: An intermediate attempt failure does not close the stream
    Given a Job spec whose first attempt exits 1 and whose backoff_limit is 3
    When the streaming submit observes the first failed attempt
    Then the CLI prints "Job 'coinflip' attempt 1 failed (exit 1, ...). Retrying in ..."
    And the stream remains open awaiting the next attempt's outcome
    And the CLI has NOT yet exited

  # The structural anti-scenario — guards against regression
  Scenario: A Job submit can never render a "running with N/M replicas" line
    Given any Job spec
    When the streaming submit runs to a terminal event
    Then no line of CLI output contains the substring "is running with"
    And no line of CLI output contains the substring "(took live)"
