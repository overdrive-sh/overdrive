# DES audit format repair

## Scope

This note records the one-time, user-approved normalization of three existing
failure-data values in `execution-log.json`. The repair changed only those
three `d` fields from prose-prefixed failure text to the exact canonical value
`FAIL`. It preserved every timestamp, event order, step/phase/status identity,
unrelated value, and the total event count. No phase was executed or appended.

The user exception was:

> Approve the one-time DES format repair: change only the three malformed failure-data strings to exactly FAIL, preserving timestamps, order, outcomes and original text in an audit note? This explicitly overrides append-only handling for those three fields only.

The recorded approval was:

> Approve three-field repair

## Provenance

| Item | Value |
|---|---|
| Original Git revision | `db5b78a5f198d053cb52e65169bdee313d26e2b2` |
| Pre-repair file SHA-256 | `2dcdd46c56b6b6213b7e02725dbe0ae512d9d74c28a116d526ff03417e7f5de1` |
| Post-repair file SHA-256 | `6603439ef608d058381366562d638f6141045fbb4ad7e29142d045a72ce4a489` |
| Event count before and after | `280` / `280` |
| Semantic change | Exactly three `d` values; no other event field changed |

## Preserved original values

| Required audit selector | Original `d` value | Normalized `d` value |
|---|---|---|
| `sid=14-04`, `p=RED`, `s=EXECUTED`, `t=2026-11-01T10:54:50Z` | `FAIL: current-source GuestKit run 0d22828511bb8062eb92e1e0 reached E23; RealCgroupFs tokio::fs write panicked under /ovd-e23-137 before typed io::Error` | `FAIL` |
| `sid=14-04`, `p=GREEN`, `s=EXECUTED`, `t=2026-11-01T11:24:58Z` | `FAIL: current cgroup-default GuestKit run 2572dafb29ff625bdddc3bbb advanced past corrected E23 but failed existing mesh body with Interrupted system call and splice assertion; exact 119 not achieved` | `FAIL` |
| `sid=14-04`, `p=GREEN`, `s=EXECUTED`, `t=2026-11-01T11:55:00Z` | `FAIL: current cgroup-default GuestKit run 5e5f472e4191780088cc1a96 reached V-6 and failed at guest_stack_mtls_egress.rs:1368 because terminated allocation exit_code was None instead of Some(0); exact 119 not achieved` | `FAIL` |

The original failure descriptions remain preserved here alongside the raw
receipts and Git history. No application, test, model, native execution, or
later-wave work was performed for this metadata repair.
