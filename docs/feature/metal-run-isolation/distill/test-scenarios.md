# Acceptance scenarios — metal-run-isolation

Specification only, following `.claude/rules/testing.md`. Executable bindings are native Rust tests and existing-tool shell integration drivers; no Gherkin consumer is introduced. Scenario tags and exact bindings are in the feature delta.

## Marcus verifies independent changes together and retains both results

Given Marcus has prepared two independent worktrees from their actual source
When Marcus starts the existing grouped verification from each worktree
Then their ordinary execution intervals overlap with distinct live guest owners
And each result belongs to its own source and invocation
And finishing one preserves the continuing invocation
And the physical test kernel remains unchanged

## Equal branch names cannot merge worktree source or build artifacts

Given two worktrees have the same branch and different local source
When Marcus prepares verification from both worktrees
Then each worktree has its own bound source and build artifacts
And neither worktree changes the other's retained evidence

## A linked worktree keeps its owner when Marcus changes its branch

Given Marcus has prepared a linked worktree
When Marcus prepares it after a branch switch or detachment
Then it keeps the same bound checkout owner

## Ordinary clones and colliding branch spellings select cold independent state

Given Marcus has retained an older branch cache
When Marcus prepares separate clones or branch names with the same sanitised spelling
Then each full source identity has independent state
And the older cache remains intact

## Invalid source discovery or a foreign destination refuses safely

Given a source cannot prove its owner or a destination belongs to another owner
When Marcus asks to prepare verification
Then the request fails with its declared identity reason
And no foreign source or retained artifact changes

## Marcus can retrieve generated evidence after editing local source

Given verification has generated evidence in its bound checkout
When Marcus retrieves the named evidence after making a local source edit
Then he receives the original evidence and keeps his local edit
And retrieval leaves the remote source and build artifacts unchanged

## Missing transfer preparation never installs packages during ordinary work

Given source transfer requires an unavailable prerequisite
When Marcus asks to prepare or retrieve verification inputs
Then he receives the declared preparation failure
And no ordinary operation installs packages or registers a selected command

## Contention distinguishes own-checkout exclusion from physical maintenance

Given an existing owner holds a checkout or exclusive maintenance authority
When another request exhausts its existing admission budget
Then it reports the matching checkout or maintenance timeout
And the held source and continuing owner remain intact

## A pre-start refusal cannot run a verification body

Given an invocation owns preparation and checkout exclusion
When its controller aborts with a declared reason or disconnects before starting
Then no selected body runs
And the retained runner result preserves the exact reason

## Losing the controller after starting keeps the command durable

Given independent invocations are executing
When Marcus loses one controller connection
Then both durable owners keep their own source locks
And the disconnected invocation retains its actual command result exactly once

## Unsupported ordinary requests cannot acquire implicit host authority

Given a request replaces execution, shares foreign artifacts or invokes an undeclared command
When Marcus submits it as ordinary verification
Then it refuses before running a selected body with its declared reason
And it never silently becomes an exclusive or physical-kernel test command

## A selected attempt keeps its actual arguments, helper and retry context

Given Marcus selects a prepared guest verification body with one retry
When nextest executes that selection
Then each actual selected body executes once in a fresh guest
And it receives the requested property budget, selected helper and attempt identity
And its actual result reaches nextest's unchanged aggregate result

## Unknown placement or missing guest preparation never falls back to the host

Given an attempt has unknown placement, invalid context or missing required preparation
When nextest asks to execute it
Then it refuses before the selected body with the exact declared test failure
And it never builds a missing dispatcher on demand

## Namespace permission denial cannot become global guest allocation

Given the launcher cannot create its owned resource context
When it tries to start the selected native body
Then it reports the exact namespace failure before that body
And it never uses a global-CID or raw-host fallback

## Native launcher proof and the named nested subject keep their meanings

Given Marcus selects existing native-launcher and named nested-boundary bodies
When he runs those selections through ordinary verification
Then the native body retains its direct physical launcher and confinement assertions
And the named nested body retains only its approved correctness credit

## Failed native qualification produces no credited evidence

Given one required native, source or owner prerequisite is absent
When verification checks that prerequisite
Then it reports the declared preflight failure and exact probe
And it does not start a selected body or claim native evidence

## Repeated retrieval returns the actual retained command result

Given a legacy or completion-confirmed current invocation has a retained result
When Marcus collects that invocation twice
Then both collections return the same output and actual command status
And source, retained records and ephemeral files remain unchanged by collection

## An incomplete owner cannot be presented as completed verification

Given a command terminal exists while its durable owner is still stopping
When Marcus collects the result
Then collection waits for that owner to stop
And collection does not rewrite records or clean resources

## Cleanup or retained-record failure cannot fabricate a command result

Given cleanup failed or retained records cannot establish completion
When Marcus collects the invocation
Then he receives the exact declared runner failure and precedence
And the actual command terminal remains retained when it exists

## Retrieval without a valid retained invocation preserves absence

Given an invocation identity is malformed or unknown
When Marcus asks to collect it
Then he receives the declared invalid-identity or not-found reason
And no invocation is created or cleaned
