# Test scenarios: workload-resources-human-readable (GH #372)

Specification only. These GIVEN/WHEN/THEN blocks are never parsed or executed and no `.feature` file exists; the acceptance tests are Rust (`testing.md`). Persona: Ana Ortiz (`ana-platform-engineer`).

## US-1: memory in human units

### Scenario: A declared memory size reads back in her own words (AC-1.1, AC-1.5)
GIVEN Ana's `dev-box.toml` declares `[resources] memory = "128MiB"`
WHEN she runs `overdrive deploy dev-box.toml` and then `overdrive workload describe dev-box`
THEN the deploy is admitted, the intent carries 134217728 bytes, and describe prints `Memory:        128MiB`

### Scenario: Equal sizes spelled differently read back identically (AC-1.9)
GIVEN one spec with `memory = "1024MiB"` and an otherwise identical spec with `memory = "1GiB"`
WHEN Ana deploys the first, describes it, then re-deploys the second
THEN describe prints `Memory:        1GiB` for both, the spec digests are equal, and the re-deploy is idempotent, not a conflict

### Scenario: A size that is not a whole number of MiB is refused (AC-1.3, DD-8)
GIVEN a spec with `memory = "1000B"`
WHEN Ana runs `overdrive deploy`
THEN the deploy fails with a message naming the `memory` field and that it must be a whole number of MiB, and nothing is stored

### Scenario: Malformed and zero sizes are refused naming the expected form (AC-1.3, D-4)
GIVEN specs with `memory = "128 megs"`, `memory = "0MiB"`, `memory = "128MB"` and `memory = 134217728`
WHEN Ana deploys each
THEN each fails at `deploy`, nothing is stored, and the message names the expected form (`<n>` plus `B`, `KiB`, `MiB`, `GiB` or `TiB`; zero says `memory must be non-zero`)

### Scenario: The removed key is refused (AC-1.2, D-1)
GIVEN a spec with `[resources] memory_bytes = 134217728`
WHEN Ana deploys it
THEN the deploy fails naming `memory_bytes` as an unknown key and `cpu`, `memory` as the expected keys, and nothing is stored

### Scenario: Every accepted size round-trips (AC-1.4)
GIVEN any accepted `memory` value
WHEN it is rendered and parsed again, and serialised to JSON and back
THEN the value is equal and the JSON is the canonical string

## US-2: CPU as a quantity

### Scenario: Millicores are admitted and read back (AC-2.1, AC-2.8)
GIVEN a spec with `[resources] cpu = "500m"`
WHEN Ana deploys it and runs `overdrive workload describe dev-box`
THEN the intent carries 500 millicores and describe prints `CPU:           500m`

### Scenario: Decimal cores read back canonically (AC-2.4, AC-2.5)
GIVEN specs with `cpu = "0.5"` and `cpu = "2"`
WHEN Ana deploys and describes each
THEN describe prints `CPU:           500m` and `CPU:           2`, and `cpu = "0.5"` and `cpu = "500m"` give the same spec digest

### Scenario: Zero CPU is admitted (AC-2.8)
GIVEN a spec with `cpu = "0"`
WHEN Ana deploys and describes it
THEN the spec is admitted and describe prints `CPU:           0`

### Scenario: A wrong-case suffix is refused (AC-2.3)
GIVEN a spec with `cpu = "500M"`
WHEN Ana deploys it
THEN the deploy fails with a message saying the milli suffix is the lowercase `m`, and nothing is stored

### Scenario: Other malformed quantities are refused (AC-2.3)
GIVEN specs with `cpu = "1.0005"`, `cpu = ""` and `cpu = "five hundred"`
WHEN Ana deploys each
THEN each fails at `deploy` naming the expected grammar (at most three fractional digits for `1.0005`), and nothing is stored

### Scenario: The removed key is refused (AC-2.2, D-1)
GIVEN a spec with `[resources] cpu_milli = 500`
WHEN Ana deploys it
THEN the deploy fails naming `cpu_milli` as an unknown key, and nothing is stored

## Node capacity (R-2, AC-1.10)

Covered by the compiler (typed fields) and the existing `aggregate_validation.rs`; no new executable test (feature delta § AC-1.10 coverage).

### Scenario: Node capacity carries the typed quantities
GIVEN a node declared with `cpu = "4000m"` and `memory = "8GiB"`
WHEN the node is admitted
THEN its capacity reads back as `4` CPU and `8GiB`, and a raw integer cannot be assigned to either field (compiler-enforced)

### Scenario: Zero node memory is refused; sub-MiB capacity is admitted
GIVEN a node declared with `memory = "0B"` and another with `memory = "512KiB"`
WHEN each is admitted
THEN the first is refused as zero memory, and the second is admitted because the whole-MiB rule applies to workloads only
