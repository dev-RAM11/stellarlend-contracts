# Interest Numeric Assumptions and Safety Limits

This note documents numeric assumptions for long-horizon interest accrual and the overflow/underflow protections validated by tests.

## Scope

- `contracts/lending/src/borrow.rs` (`calculate_interest`, `get_user_debt`)
- `contracts/hello-world/src/interest_rate.rs` (`calculate_borrow_rate`, `calculate_accrued_interest`)

## Assumptions

- Arithmetic uses signed `i128` values for balances and rates.
- Time is represented in seconds (`u64` timestamps).
- `lending` interest model is simple APR at fixed `500` bps (5%/year).
- `hello-world` rate model is utilization-based and bounded by configured floor/ceiling.

## Numeric Safety Properties

### Lending contract (`borrow.rs`)

- Interest calculation uses `I256` intermediates to avoid intermediate multiplication overflow.
- Positive fractional borrower interest is rounded up on accrual so debt cannot leak due to truncation.
- Conversion back to `i128` is clamped with `unwrap_or(i128::MAX)`, producing a saturating upper bound.
- `get_user_debt` applies `saturating_add` when accumulating interest, preventing overflow on repeated reads/accrual events.

### Hello-world contract (`interest_rate.rs`)

- Accrued interest uses checked arithmetic (`checked_mul`, `checked_div`) and returns `InterestRateError::Overflow` instead of panicking.
- Positive fractional borrower interest is rounded up after division so utilization changes cannot undercharge debt by repeated sub-unit truncation.
- Borrow rate is explicitly clamped with:
  - `max(rate_floor_bps)`
  - `min(rate_ceiling_bps)`
- Utilization is capped at 100% (`10000` bps), even when borrows exceed deposits.

## Rounding Direction

- Borrow interest accrual rounds positive fractional results up, favoring lender/protocol safety over borrower convenience.
- Numeric proof used in tests:
  - principal = `100_000`
  - rate = `500` bps
  - elapsed = `1` second
  - exact interest = `100_000 * 500 * 1 / (10_000 * 31_536_000) = 50_000_000 / 315_360_000_000`
  - exact result is greater than `0` and less than `1`, so conservative accrual stores `1` unit rather than `0`

## Long-Horizon / Extreme Scenarios Covered

- Multi-decade to centuries-scale timestamp jumps (including `u64::MAX` in lending tests).
- Maximum configured annual rate (10000 bps) for accrued-interest monotonicity checks.
- Overflow boundary test where the last safe elapsed second succeeds and the next second returns overflow.
- Extreme high-utilization + aggressive configuration + emergency adjustment still clamped to ceiling.
- Extreme negative emergency adjustment still clamped to floor.

## Security Notes

- No test relies on unchecked casts for financial results.
- Expected behavior under extreme inputs is deterministic:
  - Saturation in `lending`
  - Explicit error in `hello-world`
- This prevents silent wraparound and protects debt/accounting invariants under adversarial time jumps and parameter settings.


# Interest Numeric Assumptions

## Precision and Scale

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| **Decimal places** | 7 | Stellar native asset precision |
| **Internal scale** | 10^7 | All amounts stored as i128 * 10^7 |
| **Rate precision** | Basis points (1/10000) | Fine-grained rate control |
| **Interest scale** | 10^9 | Intermediate calculation precision |

## Rounding Strategy

StellarLend uses **Half-Up rounding** for interest accrual:

```rust
// From rounding_strategy.rs
pub fn compute_compound_interest(
    env: &Env,
    principal: i128,
    rate_bps: i128,
    periods: u32,
    mode: RoundingMode,
) -&gt; i128 {
    // interest = principal * rate_bps * periods / 10000
    // Rounded according to mode
}

| Rounding Mode | Behavior          | Use Case                          |
| ------------- | ----------------- | --------------------------------- |
| `HalfUp`      | 0.5 rounds up     | Default interest accrual          |
| `Down`        | Always round down | Conservative liquidation          |
| `Up`          | Always round up   | Fee calculation (favors protocol) |


Drift Bounds
| Scenario                    | Max Acceptable Drift | Test                                       |
| --------------------------- | -------------------- | ------------------------------------------ |
| Daily accrual, 1 year       | 1 bps (0.01%)        | `test_daily_accrual_drift_over_one_year`   |
| Hourly vs Daily convergence | 1 bps                | `test_hourly_vs_daily_accrual_convergence` |
| Small principal, high rate  | No negative drift    | `test_small_principal_high_rate_drift`     |
| Zero rate                   | Zero drift           | `test_zero_rate_no_drift`                  |
| Rounding mode divergence    | ≤ periods \* 1 unit  | `test_rounding_mode_divergence_bound`      |


Regression Gate
The interest_drift_regression_test.rs integration test is a mandatory CI gate:
# Run the drift regression test
cargo test --test interest_drift_regression_test

# Run all lending tests
cargo test -p stellar-lend-contract


If this test fails, the PR cannot merge. It guards against:
Floating-point precision changes
Rounding strategy modifications
Rate model parameter changes that affect compounding
Compiler optimization changes affecting numeric stability


Interest Rate Model
borrow_rate = base_rate + (utilization * multiplier / SCALE)
            + (if utilization > kink then (utilization - kink) * jump_multiplier / SCALE else 0)

supply_rate = borrow_rate * utilization * (1 - reserve_factor) / SCALE
Where:
utilization = total_borrows / total_deposits
SCALE = 10_000 (basis points scale)
All intermediate values use i128 to prevent overflow



Known Limitations
Daily compounding approximation: We use 365-day year, not exact calendar days
Rate granularity: 1 bps minimum rate change (0.01%)
Small principal rounding: Sub-1-unit interest rounds to 0 (dust)

