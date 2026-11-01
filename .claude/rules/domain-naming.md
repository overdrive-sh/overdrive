# Name behavior, not delivery coordinates

Use stable domain and responsibility names in maintained documentation,
tests and test support, selectors, and emitted application, test, harness,
and verification logs. Do not embed roadmap step IDs in filenames,
headings, identifiers, messages, or result labels.

Describe the actual owner, operation, boundary, or observable outcome.
Keep delivery-coordinate mappings in the roadmap; use stable feature,
ADR, or acceptance-scenario references for traceability.

Existing names and required delivery/audit formats are migrated under
[issue #333](https://github.com/overdrive-sh/overdrive/issues/333).
Until that tooling migration lands, preserve its required schemas and
filenames; do not spread legacy coordinates into additional prose,
helpers, or log messages. Preserve immutable Git history and original
captured evidence rather than rewriting their provenance.

Review changes for this rule. A naming refactor must preserve behavior,
assertions, exact test selection, accounting, cleanup, links, and evidence
consumers. Domain state names, feature identifiers, protocol versions,
port numbers, and stable scenario identifiers are not roadmap step IDs.
