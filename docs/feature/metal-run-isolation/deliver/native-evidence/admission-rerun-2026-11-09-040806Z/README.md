# Admission journey — passed with the controlled E4 owner fixture

This directory preserves 138 raw files copied byte-for-byte from
`/tmp/metal-native-remaining-20261109T040806Z-admission`. The foreground
invocation record reports exit 0, source HEAD
`be7d50659adc8b4c6808b782c2207d87647cc002`, and the dirty source status saved
in `foreground-invocation.json`. The foreground log contains the driver's
success marker:

```text
METAL-ISOLATION-ADMISSION public_guest_timeout=true e4_boundary=controlled-owner-fixture maintenance_waited=true stopped_owner_before_start=true unadmitted_waiter_did_not_block=true
```

The marker explicitly attributes the E4 boundary to the controlled-owner
fixture. This records the admission driver result; it does not claim that the
separate placement journey or complete DELIVER acceptance passed.

## Captured source and owners

All owner snapshots identify commit
`be7d50659adc8b4c6808b782c2207d87647cc002`, source digest
`e052d2bd458e5b763e92ae30bfafcbada8cc70af3584c932a56bd8ea39ea3c28`, and
checkout key
`e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410`.

| Role | Run ID | Class | Final status |
|---|---|---|---|
| Public guest timeout | `afff84c467043e6224e4d80a` | ordinary | Command 100, complete receipt and all three roots absent. |
| Controlled admitted owner | `720d538ab20a3acea6ed6890` | ordinary | Command 0, complete receipt and all three roots absent. |
| Exclusive maintenance | `6be19464e6425bf79800a401` | exclusive | Command 0, complete receipt and all three roots absent. |
| Ordinary waiter | `57d086dd1a7cc29840e3bf63` | ordinary | Command 0, complete receipt and all three roots absent. |

`maintenance-stopping-predecessor.json` and `unadmitted-waiter.json` preserve
the admitted owner’s deactivating state alongside the waiting exclusive owner
and registered unadmitted ordinary waiter. `maintenance-passed-unadmitted-waiter.json`
then records the admitted owner complete while the exclusive owner holds
admission and the waiter remains `waiting_for_lease`. `completed.json` records
all four owners complete, with no descendants or owned roots remaining.

This admission run passed at the recorded source and boundary. The placement
driver that followed is preserved separately and failed during its direct
native case.
