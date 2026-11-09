# Login-mode cache handoff and preparation argument correction

## Reproduced boundaries

The e21d4fcb source contained two bounded temporary-preparation changes: a
sticky common parent for private run roots and handoff of root-owned entries
under the bound checkout's default `target/debug` tree to the login user.
Focused canonical Lima checks passed the existing preparation-error,
non-Metal/exclusive passthrough and exact-root finalization cases, and
`bash -n infra/metal/bootstrap.sh` passed.

The stable e21d4fcb login pair in [`e21-stable-login-pair/`](e21-stable-login-pair/)
ran the exact existing selector
`placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain`:

- `--no-sudo`: outer command 0;
- `--no-sync --no-sudo`: outer command 0, after the canonical source-identity check.

These commands ran before the next private argument correction. They show that
the login path passed after the common-parent and Cargo-cache changes, but are
not the final-source validation.

## Proven privileged preparation argument defect

The same e21d4fcb source's default privileged invocation of that existing
selector returned 75 before START. Its actual shell diagnostic showed the
fourth `%s` in `REMOTE_TMP_PREP` expanded to `sudo -n -u ubuntu`, so the
privileged command attempted to create a root-owned run root as `ubuntu` and
failed with `Operation not permitted`. The `REMOTE_TMP_PREP` argument array
placed the login-child prefix one slot before its corresponding `%s` after
adding the Cargo-cache handoff. The implementation swaps only those two
argument values; per-run ownership and modes are unchanged.

The [`commit-rewrite-invalidated/`](commit-rewrite-invalidated/) capture is
retained separately. Its first `--no-sudo` run returned 0 at commit
`2ceef898`; during that paired attempt, the commit was administratively
re-attributed to `e21d4fcb` with the identical tree. The following no-sync
command correctly returned 78 `source_mismatch` because the Git binding
changed. This is an invalidated pair, not a runtime regression or no-sync
acceptance result.

## Final-source mode validation

The exact four existing placement-driver mode selectors were rerun after
the argument-order correction on a frozen committed source:

1. ordinary privileged;
2. ordinary privileged with `--no-sync`;
3. ordinary login with `--no-sudo`;
4. ordinary login with `--no-sync --no-sudo`.

No additional placement rows, owner burst, mutation, or model campaigns are
part of this correction. At HEAD
`060359bc1cf88c78252e9d789396821413b41d7e`, all four outer commands returned
0 and each durable owner completed with command status 0. Both no-sync runs
passed source identity checks. The before/after source records are byte
identical and pin `infra/metal/bootstrap.sh` SHA-256
`47e7152b18ef5d7b57a26d288f473769d70258d8086477bdb563cd9c17e2f625`.

| Mode | Run ID | Outer status |
|---|---|---:|
| ordinary privileged | `eac8724e1b762671708565f7` | 0 |
| ordinary privileged, no-sync | `8e7b311b75348b3612cf43ad` | 0 |
| ordinary login, no-sudo | `06ea1168e446d77960b36bb4` | 0 |
| ordinary login, no-sync and no-sudo | `772d30eb605de6b8f5f0e13a` | 0 |

Raw commands, combined stdout/stderr, completion statuses, source metadata
and exact file hashes are retained in
[`final-source-four-mode-pass/`](final-source-four-mode-pass/). These four
existing mode selections pass. They do not claim that the full placement
journey, its final physical-complement assertion, or the entire feature is
complete.
