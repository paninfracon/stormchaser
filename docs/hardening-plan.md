# Hardening plan

Written 2026-10-19, after two whole-project security scans (2026-10-12 and
2026-10-14) and an architecture review. Between them they found about 90
security findings and a long tail of functional bugs. This plan explains why
the test suites, with about 850 tests in 65 integration suites, caught almost
none of them, and what to change so the next round finds less.

## Why the tests did not catch them

The bugs were mostly in places the tests never looked. The stack the tests ran
on was more permissive than production. Each side of a boundary was tested on
its own. The tests were written from the code, not from the documentation or
from what an attacker would try.

| Kind of bug | Examples | Why the tests missed it |
|---|---|---|
| A missing check | A percent-encoded object id skipped the ownership check; any keyed runner could take over any step through `unpacking_sfs`; an event rule could be attached to another user's webhook; `/events/correlate` needed no token | Tests confirm that allowed actions work. A check nobody wrote can't fail a test. There were no tests where user B tries user A's object, or runner X reports on runner Y's step. |
| Test stack more permissive than production | NATS allow-lists that left out the shard runners publish under; `_INBOX.>` readable by every user; the chart never set `STORMCHASER_JWT_SECRET`, which a release build requires | `scripts/test-env.sh` connects every test as the unrestricted engine NATS user. An unconfigured OPA allows everything. Debug builds fall back to a development JWT key. CI starts only NATS and Postgres. |
| Two sides of a boundary that disagree | Log lookups used 8 characters of a step id while runners used all of it; four lists of step types disagreed, so some specs were never validated; `WorkflowRunDetail` was defined three times; the CLI's login sent an id_token where the API expected a code; `/auth/exchange` dropped `groups` | Each side was tested against its own idea of the contract. No test in CI runs a real runner: the Docker runner has no integration tests and the Kubernetes cluster tests are `#[ignore]`d. That is how OCI pushes from the Docker runner and the Ofelia cron engine never worked without anyone noticing. |
| Documented but never built | `retry`, `allow_failure`, a step library's timeout, matching `Wait` steps to webhook events | Nothing compares the code with the documentation, and the DSL parser silently drops attributes and blocks it does not know. |
| Failure handling | A direct run dropped while OPA was unreachable; a step lost when its engine died; late or repeated reports moving finished steps; a fencing token that compared a value that never changes | Every test runs against a healthy stack. These were found by stopping OPA and Postgres on a test install. No test asserted that a stale token was rejected. |
| Hostile input and resource exhaustion | A stack overflow in the HCL parser; a test-report tar bomb; templates and WASM modules without limits; the SSRF guard bypassed by a trailing dot in the host name | There is no fuzzing or property testing in the repository, and hand-written tests use sensible input. |
| Deployment files | `.dockerignore` produced 0-byte binaries; the umbrella chart deployed the Docker runner by default; `setup.sh` paths | Charts are only rendered with `helm template`. Nothing builds the images, installs a chart and runs a workflow in CI. |
| Process | The commit hook passed with a stale advisory database; long-lived branches got no CI | CI runs only on pushes and pull requests to `trunk`. `Cargo.lock` is not tracked, so CI, the hook and releases can resolve different dependency versions. |

## Tier 1: cheap changes that cover the most ground

1. **Run CI on every branch push**, not only on `trunk`.
2. **A nightly dependency check** with a fresh advisory database, so a new
   advisory shows up the next morning and not at the next commit.
3. **A production-shaped test stack.** The test stack already runs the shipped
   `deploy/nats/nats-server.conf`, with its per-role users. Run the permission
   tests that were `#[ignore]`d against it by default. Then move the other
   suites off the unrestricted engine user, and onto the shipped default-deny
   policy, where they don't need either.
4. **A strict DSL parser.** Unknown blocks and attributes, and values of the
   wrong type, are errors, not silently ignored. Every workflow example in the
   documentation is parsed by a test.
5. **Ban the dangerous APIs with clippy** (`clippy.toml`
   `disallowed-methods`/`disallowed-types`): a plain `reqwest` client outside
   the SSRF-guarded one, `hcl::from_str` outside `hcl_guard`, and unpacking a
   `tar::Archive` without limits. The bypasses fixed in the scans can't come
   back by accident.
6. **Check the deployment configuration against the code.** Compare every
   environment variable a binary reads with what the Helm charts and compose
   set. This would have caught the missing JWT secret.
7. **Authorization matrices that fail on omission:**
   - Every API route (already listed from the router for the OpenAPI test)
     against anonymous, another user, the owner and an admin. A route with no
     entry fails the build.
   - Every NATS subject against each role.
   - Every step event handler against the wrong runner, a finished step and a
     replayed message.

## Tier 2: find what nobody thought to test

1. **System tests in CI.** Run the compose stack with real binaries and the
   Docker runner on every pull request (`scripts/smoke-test-docker.sh`). Add a
   nightly k3d job that installs the umbrella chart and runs
   `scripts/smoke-test-k8s.sh` and the dogfood workflow.
   Load tests (`integration_load_tests`, `scripts/e2e-load-test*.sh`) stay off
   GitHub's runners: they run on the build host, nightly or before a release.
2. **Fault injection.** Bring the system-test chaos scripts (stop Postgres,
   stop OPA, kill an engine under load) into the repository as a nightly suite.
3. **Fuzzing.** Add `cargo-fuzz` targets for the HCL parser and `hcl_guard`,
   test-report unpacking, webhook signature parsing, the authorization path
   parser and `net_guard` URL handling. Add `proptest` for redaction and path
   normalisation.
4. **Mutation testing.** Run `cargo-mutants` on `authz`, `net_guard`, `redact`,
   `runner_auth` and the step event handlers. A surviving mutant is a check
   that no test asserts.

## Tier 3: keep it from coming back

1. **Enforce rules in types and the database.**
   - An object reaches a handler only through an `Authorized<T>`, which only
     the authorization layer can create.
   - The database refuses step transitions the state machine forbids (started
     in f1b9318).
2. **One configuration module per service.** It validates every setting at
   start-up and refuses insecure values in release builds. The chart values
   and the documentation are generated from it.
3. **A threat model per trust boundary**: users, workflow authors, runners,
   step containers and webhook senders. Add a pull request checklist item: a
   new route, subject or handler is added to the matrix.
4. **A scoped security review before each release**, rather than a
   whole-project scan every few months.

Open decisions that affect this plan: per-runner NATS credentials wait for the
architecture review (they must work for autoscaled runners), and `Cargo.lock`
stays untracked by choice.

## State transitions audit (2026-10-19)

Every run and step status change should go through `WorkflowMachine` or
`StepMachine`. The machines' own save path (`persistence.rs`) is called only by the
machines, but many changes go around them:

| Where | Move | Problem |
|---|---|---|
| `DELETE /api/v1/runs/{id}` | a run in any status is deleted | A running run disappears mid-flight; runners are not told to stop. |
| `handler/workflow/direct.rs`, `abandon.rs` | run to `failed` | Plain SQL, no machine. |
| Step event handlers | run rebuilt as `Running`, then `succeed()`/`fail()` | No status check; nothing guards run transitions in the database. |
| `handler/mod.rs` (quota refused, dispatch failed) | step to `pending` | No check that the step is still `initializing`; errors discarded. A dispatch that fails after publishing can put back a step a runner is running. |
| `claim_pending_step_instances_for_run`, `reset_lost_dispatches`, `reset_lost_engine_steps`, `reset_step_for_retry` | step to `initializing` or `pending` | Plain SQL; the retry reset has no status condition. |
| Built-in step handlers | `initializing` step rebuilt as `Pending`, then `start()` | Wrong state type; JQ and WASM discard the result. |
| Completed and failed handlers | any unfinished step rebuilt as `Running` | Documented, unchecked. |
| `StepMachine::from_instance`, `WorkflowMachine::new_from_run` | any | Build a machine in any state without checking the stored status; 14 of 35 callers never check. |
| `step_status_transition_allowed` trigger | any | Lets an unfinished step move to almost any status. |
| `runners.status` | online/offline | Three SQL writers, no machine; a heartbeat revives an offline runner. |

Fixes, in order:

1. `DELETE /runs/{id}` refuses a run that has not finished.
2. The SQL transitions become machine methods that compare and set
   (`UPDATE … WHERE status = <expected>`), failing when nothing matched.
3. Machines are loaded with a check that the stored status is the state's.
4. A database trigger for run transitions, and the step trigger tightened to the
   documented diagram.
5. Dead transition code removed; built-in handlers use the right state.
6. A test that fails on a status write outside the machines.

## Progress

Branch `hardening/low-hanging-fruit`, 2026-10-19.

| Item | Status | Notes |
|---|---|---|
| 1. CI on every branch | Done | `ci.yml` runs on pushes to any branch, nightly and on demand. A newer push cancels a branch's run in progress (never on `trunk`). |
| 2. Nightly dependency check | Done | The nightly run includes `cargo deny`, which fetches a fresh advisory database. With `Cargo.lock` untracked, the nightly build also catches a new dependency release that breaks the build. |
| 3. Production-shaped test stack | Started | The three NATS permission tests no longer need `#[ignore]`: the test stack already runs the shipped `nats-server.conf`, so they connect as its per-role users. They pass. Next: move suites off the unrestricted engine user and onto the shipped policy where they don't need them. |
| 4. Strict DSL parser | Done | Unknown blocks and attributes, a `step` outside `steps`, a second `workflow` block and wrongly-typed values are errors. A new test parses every `.storm` file and every HCL example in the docs. **What it found:** the README's main example, `tests/approval-wait.storm` and two fixtures had their steps outside `steps`, so they parsed to workflows with no steps; `version`, a provision `name` and storage `preserve` were set but dropped. Changelog has an upgrade note. |
| 5. Clippy bans | Done | `clippy.toml` bans plain `reqwest` clients, `hcl::from_str`/`hcl::parse`, `hcl` `evaluate` and `tar::Archive::unpack`. Each of the 24 permitted uses (operator-configured endpoints, the API's own clients, guarded parses, test files) has an `#[expect]` saying why it is safe. |
| 6. Deployment settings against the code | Half done | A test fails when a chart or compose sets a variable its service never reads. It removed 17 dead settings (11 in the charts, 6 in compose), the ones listed by hand in `docs/environments.md`, which the test now replaces. A new CI job (`scripts/check-charts.sh`) lints and renders the charts and fails when the umbrella chart's vendored subchart archives differ from their sources, which they did. Not done: the other direction (a variable a release build requires that the chart does not set, as with `STORMCHASER_JWT_SECRET`); required settings are read ad hoc, so this needs the configuration module from Tier 3 first. |
| 7. Authorization matrices | Not started | |
| State transitions audit, fixes 1 to 6 | Done | `DELETE /runs/{id}` refuses unfinished runs (409; the old test asserted the bug). Machines load with a status check and save with compare-and-set; `Unfinished` models a step's end arriving out of order; the SQL transitions became machine methods (`step_machine/bulk.rs`, `requeue`, `retry`); a dispatch that failed after publishing fails the step instead of putting it back. Migration `guard_run_transitions` adds a run trigger and tightens the step trigger to the diagram, with tests both ways. `status_writes_tests.rs` fails on a status update outside the machines. Removed never-called transitions and dead SQL. **Found:** the run timeout left `initializing` steps as they were; the initializing transition never saved the runner id. Not done: `runners.status` (online/offline) still has no machine; the web UI's wasm build was not compiled here. |

Found along the way:

- **A real race in the lost-engine sweep**, found by a flaky test
  (`integration_engine_loss_tests`, about one failure in three). The sweep re-runs
  lost `JQ`/`JinjaRender` steps, then fails the other lost steps in a second query. A
  step that became lost between the two queries was failed instead of re-run. Fixed,
  with a deterministic test.
- **Step spec settings are not refused at run time.** Only `stormchaser lint` checks a
  spec against the step type's schema, and the engine deserialises specs leniently. A
  misspelt `imagee` is ignored. Next: `deny_unknown_fields` on the spec types, and
  validate when a run is submitted.
- The API's calls to the identity provider, the JWKS fetch and the log backend client
  use `reqwest` clients with no timeout, so a stalled server holds a request open.
