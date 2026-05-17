# Test Remediation Plan

## Context and Findings

Recent CI instability raised suspicions about the validity of our test suite. A static analysis script was run across all test functions in the workspace to evaluate adherence to the newly established quality standards.

The analysis revealed two significant categories of "fake" tests that report passing but provide no actual validation:

### 1. The `SQL_OFFLINE` Bypass Anti-Pattern

Two integration tests in `crates/stormchaser-engine/tests/integration_step_machine_tests.rs` contained an early return bypass:

```rust
if var("SQL_OFFLINE").is_ok() {
    return Ok(());
}
```

Since our CI coverage scripts (`cargo llvm-cov`) run with `SQL_OFFLINE=true` (which is intended purely as a compiler flag for `sqlx` macros, not a runtime test bypass), these tests were silently returning `Ok(())` immediately. **This has already been corrected via a search-and-replace to remove the early returns.** However, these tests must now be verified to ensure they actually pass when executing their full logic.

### 2. Missing Assertions

The script identified 22 tests that do not contain a single `assert!`, `assert_eq!`, `unwrap()`, or `expect()` statement. These tests execute code but never validate the outcome. They will only fail if a panic occurs during execution.

**Affected Tests (Zero Assertions):**

- `test_handle_orphaned_container_compiles` (stormchaser-runner-docker)
- `test_publish_container_result_compiles` (stormchaser-runner-docker)
- `test_into_result` (stormchaser-runner-docker)
- `test_run_parking_agent_compiles` (stormchaser-runner-docker)
- `test_opa_wasm_evaluate_fails_on_empty_module` (stormchaser-opa)
- `test_routing_helpers` (stormchaser-engine)
- `test_shutdown_telemetry` (stormchaser-engine)
- `test_build_approval_mailer_explicit` (stormchaser-engine)
- `test_build_approval_mailer_env_vars` (stormchaser-engine)
- `test_handle_lambda_invoke_not_enabled` (stormchaser-engine)
- ...and 12 others.

## Action Plan

To restore confidence in the test suite, we will execute the following remediation steps:

### Phase 1: Verify Un-bypassed Integration Tests

1. Run the previously bypassed tests in `integration_step_machine_tests.rs` to see if they pass in a real environment.
2. If they fail, fix the underlying logic or the test setup to ensure they correctly validate the step machine behavior.

### Phase 2: Remediate Assertion-less Tests

Iterate through the 22 identified tests that lack assertions:

1. **Analyze Intent:** Read the test name and the code it executes to determine what *should* be validated.
2. **Add Assertions:** Introduce meaningful `assert!` or `assert_eq!` statements to verify state changes, returned values, or correct error variants.
3. **Refactor if Necessary:** If the test is merely a "does it compile" test (e.g., `..._compiles`), evaluate if it should be expanded into a behavioral test or removed if it provides zero runtime value.

### Phase 3: Address Generic Assertions

(Optional, lower priority)
Iterate through the 95 tests flagged for using generic `is_ok()` / `is_err()` assertions. Update them to assert against specific Ok values or exact Error variants to adhere to the "Meaningful Assertions" standard.
