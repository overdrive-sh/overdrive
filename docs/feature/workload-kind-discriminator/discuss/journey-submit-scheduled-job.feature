Feature: Submit a Scheduled Job (parse + validate; execution deferred)
  # Trace: J-OPS-002 (primary)
  # Scope of THIS feature: Slice 05 ships parser + composition validation only.
  # Schedule execution is a deferred follow-up — see wave-decisions.md.
  # Cross-references: journey-submit-scheduled-job.yaml

  Background:
    Given the operator config at "~/.overdrive/config" names a running control plane

  Scenario: A Scheduled Job spec is recognised by [job] + [schedule] co-presence
    Given a TOML file with [job], [schedule] (cron = "0 2 * * *"), [exec], [resources]
    When the parser reads the file
    Then a Schedule-kind workload spec is constructed
    And the cron expression is captured as a string field
    And the spec carries no Service variant

  Scenario: A spec with [schedule] but no [job] is rejected
    Given a TOML file containing [schedule] without [job]
    When the operator runs "overdrive job submit ./bad.toml"
    Then the CLI prints an error naming "[schedule] is only valid alongside [job]"
    And the CLI exits with a non-zero status code

  Scenario: A spec with [schedule] and [service] is rejected
    Given a TOML file containing [schedule] and [service] without [job]
    When the operator runs "overdrive job submit ./bad.toml"
    Then the CLI prints an error naming "[schedule] is only valid alongside [job], not [service]"
    And the CLI exits with a non-zero status code

  Scenario: Submitting a Schedule echoes "registered" with an explicit deferral note
    Given a Scheduled Job spec at "./nightly-backup.toml"
    When the operator runs "overdrive job submit ./nightly-backup.toml"
    Then the CLI prints "Submitting schedule 'nightly-backup' (kind=Schedule)"
    And the CLI prints "Schedule registered."
    And the CLI prints a NOTE indicating execution is not yet implemented
    And the NOTE includes the tracking issue URL
    And the CLI exits with status 0

  Scenario: alloc status for a Schedule reflects the deferral consistently
    Given a Scheduled Job has been registered but execution is deferred
    When the operator runs "overdrive alloc status --job nightly-backup"
    Then the output contains "kind: Schedule"
    And the output contains "Cron: 0 2 * * *"
    And the output contains "No allocations have been spawned yet."
    And the output's deferral URL byte-matches the URL printed by the submit echo

  Scenario: A spec with [schedule] missing the cron field is rejected
    Given a TOML file with [job] and [schedule] but no cron field
    When the operator runs "overdrive job submit ./missing-cron.toml"
    Then the CLI prints an error naming "cron field is required when [schedule] is present"
    And the CLI exits with a non-zero status code
