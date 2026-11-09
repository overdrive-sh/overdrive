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

## Final-source validation pending

The exact four existing placement-driver mode selectors must be rerun after
the argument-order correction on a frozen committed source:

1. ordinary privileged;
2. ordinary privileged with `--no-sync`;
3. ordinary login with `--no-sudo`;
4. ordinary login with `--no-sync --no-sudo`.

No additional placement rows, owner burst, mutation, or model campaigns are
part of this correction. Final-source native results are not included in this
capture and must be added only after their actual invocations complete.
