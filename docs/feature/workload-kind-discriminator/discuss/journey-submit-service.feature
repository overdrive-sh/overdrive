Feature: Submit a Service (long-running)
  # Trace: J-OPS-002 (primary), J-OPS-003 (secondary)
  # Platform: CLI / control-plane streaming / alloc status render
  # Kind: Service — replicas-of-eternal-process; restart on exit; never Succeeds
  # Cross-references: journey-submit-service.yaml

  Background:
    Given the operator config at "~/.overdrive/config" names a running control plane
    And the operator has a Service spec at "./payments.toml" with id "payments" and replicas 1

  Scenario: A Service spec is recognised by section presence
    Given a TOML file containing only [service], [exec], [resources]
    When the parser reads the file
    Then a Service-kind workload spec is constructed
    And neither Job nor Schedule kinds are constructed

  Scenario: A spec with both [service] and [job] is rejected with named guidance
    Given a TOML file containing both [service] and [job] blocks
    When the operator runs "overdrive job submit ./mixed.toml"
    Then the CLI prints an error naming both sections as the conflict
    And the CLI suggests "exactly one of [service] or [job] is required"
    And the CLI exits with a non-zero status code

  Scenario: A spec with [service] and [schedule] is rejected (services do not terminate)
    Given a TOML file containing [service] and [schedule] blocks
    When the operator runs "overdrive job submit ./bad.toml"
    Then the CLI prints an error naming "[schedule] is only valid alongside [job], not [service]"
    And the CLI exits with a non-zero status code

  Scenario: Submit echoes the kind discriminator before streaming
    Given a Service spec at "./payments.toml"
    When the operator runs "overdrive job submit ./payments.toml"
    Then the CLI echoes "kind=Service" in its submit-acknowledgement line
    And the echoed spec_digest matches the operator's local computation
    And the echoed endpoint matches the trust triple's endpoint

  Scenario: A stable Service emits a kind-aware running summary with a real duration
    Given a Service spec at "./payments.toml" whose workload stays alive past the stability window
    When the streaming submit observes convergence
    Then the CLI prints "Service 'payments' is running with 1/1 replicas (took <duration>)"
    And the duration is a measured value, not the literal "live"
    And the line uses the word "Service", not "Job"

  Scenario: alloc status renders Service-kind context
    Given a Service "payments" has been running stably for at least 2 seconds
    When the operator runs "overdrive alloc status --job payments"
    Then the output contains "kind: Service"
    And the output contains "Replicas (desired/running): 1/1"
    And the output's spec_digest matches the digest from the original submit

  # --- Listener spec shape (folded in 2026-05-10 from GH #164) -----------------

  Scenario: A Service with two listeners parses with both triples preserved in declaration order
    Given a TOML file with [service] id="frontend" and two [[listener]] blocks (8080/tcp/10.0.0.1 then 8081/udp/none)
    When the parser reads the file
    Then a Service-kind workload spec is constructed
    And the spec carries listeners [(10.0.0.1, 8080, tcp), (none, 8081, udp)] in declaration order

  Scenario: Protocol parsing is case-insensitive and renders lowercase
    Given a TOML file with a [[listener]] whose protocol is "TCP"
    When the parser reads the file
    Then the parsed listener's protocol equals Proto::Tcp
    And every CLI surface that renders the protocol prints "tcp" in lowercase

  Scenario: A Service with zero listeners is rejected with named guidance
    Given a TOML file with [service] but no [[listener]] blocks
    When the operator runs "overdrive job submit ./no-listener.toml"
    Then the CLI prints an error stating "a [service] requires at least one [[listener]] block"
    And the CLI exits with a non-zero status code

  Scenario: A duplicate (vip, port, protocol) triple within a Service is rejected
    Given a TOML file with two [[listener]] blocks both naming (none, 8080, tcp)
    When the operator runs "overdrive job submit ./duplicate.toml"
    Then the CLI prints an error naming the duplicate triple
    And the CLI exits with a non-zero status code

  Scenario: An unsupported protocol value is rejected
    Given a TOML file with a [[listener]] whose protocol is "sctp"
    When the operator runs "overdrive job submit ./bad-proto.toml"
    Then the CLI prints an error naming "sctp" as an unsupported protocol
    And the error names the supported set as "tcp, udp"
    And the CLI exits with a non-zero status code

  Scenario: A listener with port = 0 is rejected
    Given a TOML file with a [[listener]] whose port is 0
    When the operator runs "overdrive job submit ./bad-port.toml"
    Then the CLI prints an error naming "port must be in 1..=65535"
    And the CLI exits with a non-zero status code

  Scenario: Submit echo surfaces every listener with pinned-or-pending VIP
    Given a Service spec with one pinned-VIP listener (10.0.0.1:8080/tcp) and one None-VIP listener (8081/udp)
    When the operator runs "overdrive job submit ./mixed-vip.toml"
    Then the CLI submit echo includes a "Listeners:" section
    And the pinned-VIP listener line equals "10.0.0.1:8080/tcp"
    And the None-VIP listener line equals "(vip: pending allocation - see #167):8081/udp"

  Scenario: alloc status Listeners section byte-equals the submit echo Listeners section
    Given a Service "frontend" with two listeners (one pinned, one pending) was submitted
    When the operator runs "overdrive alloc status --job frontend"
    Then the output contains a "Listeners:" section
    And every line in the Listeners section byte-equals the corresponding submit echo line

  Scenario: A JobSpecInput with N listener triples round-trips bit-equivalently
    Given an arbitrary JobSpecInput with N valid listener triples
    When it is serialised to TOML, parsed back, converted to a Job aggregate, and converted back via JobSpecInput::from(&Job)
    Then the resulting JobSpecInput equals the original

  Scenario: OpenAPI roundtrip passes for Listener and ServiceVip types
    Given the Listener and ServiceVip newtypes derive utoipa::ToSchema
    When the operator runs "cargo openapi-gen" and "cargo openapi-check"
    Then both commands exit with status 0
    And the generated schema includes Listener with port, protocol, and optional vip fields
