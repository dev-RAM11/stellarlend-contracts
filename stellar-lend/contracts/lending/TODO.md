# Adversarial Test Implementation TODO

## Task
Add adversarial tests that attempt to borrow and immediately withdraw collateral in ways that might exploit rounding, timing, or view inconsistencies. Ensure the contract rejects any path that would leave positions undercollateralized.

## Steps

- [x] 1. Analyze existing codebase and adversarial tests
- [x] 2. Identify gaps in realistic multi-step sequences with price changes
- [x] 3. Create comprehensive test plan
- [x] 4. Create `borrow_withdraw_sequence_adversarial_test.rs`
- [x] 5. Update `lib.rs` to include new test module
- [x] 6. Run tests — Rust toolchain not installed in this environment; tests should be run via `cargo test --package lending borrow_withdraw_sequence`
- [x] 7. Update TODO with results

