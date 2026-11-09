# Guest-lane attempt missing its staged environment helper

The current ordinary two-cell run is preserved in `foreground.log` with its
source and command receipt in `foreground-invocation.json`. The captured
source commit is `1b623e6e167c4d3113efcf0d393c74748fbe9834`; the complete dirty
source status is retained in that receipt.

Before this run, the canonical `cargo xtask guest-kit-prepare` completed with
run ID `c2b882bc307fe54c8e599dae`, exit 0, at the same source commit. Its raw
output and invocation receipt are under `guest-kit-prepare/`.

The native run used outer run ID `941dffc5130c826ce3eeeb89` and Nextest run
ID `3deaf61e-dd91-4af8-b71d-9c1c46a867a9`. Both selected cells failed and the
outer command returned 100. The owner cell's scripted guest booted, then its
console reported:

```text
run-with-modules: exec /usr/bin/env: No such file or directory (os error 2)
```

The owner host script then received no script-channel connection within its
existing 60-second bound. The operator cell also failed, after 2.046 seconds;
its retained output does not establish a more specific errno. This capture
proves the attempted guest command lacked its existing environment helper. It
does not prove either selected cell passed.

## Earlier attempts

`prior-085114-stale-unprepared-dispatch/` preserves an earlier run at source
`8b1dd117d9ea020c6fb0429896f7a32186574331`. That run had not rebuilt the
prepared dispatcher after the P4 implementation commit, and reports the old
empty-group refusal. It is not post-preparation dispatcher evidence.

`prior-085627-empty-fixture/` preserves the later attempt at the same captured
source commit. Its failure is the empty closed-placement fixture field and
happens before guest creation. It is not evidence about the guest environment
helper and is not a pass.

All copied raw logs and invocation receipts were checked byte-for-byte against
their original `/tmp/metal-native-remaining-*` captures. No native rerun of
the environment-helper correction is included here; that rerun remains
pending focused review.
