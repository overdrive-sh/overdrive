# every_retained_record_still_validates
01:16:47Z `metal run -- cargo nextest run -p overdrive-bench --features integration-tests --test integration -E 'test(every_retained_record_still_validates)'` run 227585af7539ff8f9519e9c8, exit 0: `1 test run: 1 passed, 86 skipped` (the retained dirs then included the incomplete E-G4 record).

## Verified against the record
The test run itself (run 227585af..., exit 0, 1 passed) is box-side and not in the repo. In the repo, all four directories hold well-formed record.jsonl (E-G4 killed record ends without a terminal line, matching the receipt's "incomplete" claim). The test's pass is evidence only as reported.
