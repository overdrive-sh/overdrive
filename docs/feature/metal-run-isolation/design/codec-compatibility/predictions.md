# Codec probe predictions

This bounded diagnostic verifies the installed quint-connect 0.1.2 decoding path. It does not drive Metal or assert production conformance. No product API or protocol is changed.

1. Hypothesis: the existing flat `conf_step` cannot be decoded by the public runner with `Config::nondet` naming it. Predicted outcome: an actual named Quint trace fails before `Driver::step`, with the missing sum-type variant/tag diagnostic. Falsification: the unchanged flat trace reaches `Driver::step` successfully.
2. Hypothesis: an ordinary record `{tag: action, value: {actor, value}}` satisfies that codec without adding a Quint sum type. Predicted outcome: all 13 existing named runs and bounded seeded random traces reach `Driver::step` with exactly the model-recorded action and integer picks. Falsification: decode error, wrong action/picks, or omission of any named run.
3. Hypothesis: Idle must record its own tag while REC=true. Predicted outcome: a random trace with a non-Idle predecessor decodes Idle as actor 0/value 0; REC=false remains constant. Falsification: repeated predecessor action or changed REC=false transition semantics.
4. Hypothesis: encoding metadata does not change the checked protocol. Predicted outcome: exactly the unchanged 18 manifest entries pass (16 holds, 2 expected violations), unchanged 13 named runs, and evidence verification succeeds. Falsification: any different outcome, modified manifest/hazard/run definitions, or non-recording protocol diff.
