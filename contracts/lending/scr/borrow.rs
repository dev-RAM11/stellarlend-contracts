// Add at top of file
mod rounding_strategy;
use rounding_strategy::{
    calculate_interest_with_rounding, RoundingMode, SECONDS_PER_YEAR, 
    BASIS_POINTS_SCALE, InterestCalcResult,
};
// ════════════════════════════════════════════════════════════════
// UPDATED: Interest Calculation with Rounding Strategy
// ════════════════════════════════════════════════════════════════
/// Calculate interest with rounding drift protection
///
/// # Security
/// - Uses checked arithmetic to prevent overflow
/// - Applies banker's rounding to reduce systematic bias
/// - Tracks drift for long-horizon reconciliation
pub fn calculate_interest(env: &Env, debt_position: &DebtPosition) -> Result<i128, BorrowError> {
    let now = env.ledger().timestamp();
    let elapsed = if now > debt_position.last_update {
        now - debt_position.last_update
    } else {
        return Ok(0); // No time has passed
    };
    if debt_position.borrowed_amount == 0 || elapsed == 0 {
        return Ok(0);
    }
    // Use banker's rounding for better long-horizon behavior
    let result = calculate_interest_with_rounding(
        debt_position.borrowed_amount,
        elapsed,
        500, // 5% APR (fixed rate)
        RoundingMode::Bankers, // ← KEY: Use banker's rounding
    )
    .map_err(|_| BorrowError::Overflow)?;

    // Ensure non-negative
    if result.interest < 0 {
        return Err(BorrowError::Overflow);
    }
    Ok(result.interest)
}
/// Get user's debt with interest accrual (view function)
///
/// # Security
/// - Non-mutating: only reads state
/// - Uses saturating arithmetic to prevent overflow on repeated calls
pub fn get_user_debt(env: &Env, user: &Address) -> DebtPosition {
    let mut position: DebtPosition = env
        .storage()
        .persistent()
        .get(&BorrowDataKey::BorrowUserDebt(user.clone()))
        .unwrap_or(DebtPosition {
            borrowed_amount: 0,
            interest_accrued: 0,
            last_update: env.ledger().timestamp(),
            asset: Address::generate(env),
        });
    // Accrue interest
    match calculate_interest(env, &position) {
        Ok(accrued) => {
            // Use saturating_add to prevent overflow on view calls
            position.interest_accrued = position.interest_accrued.saturating_add(accrued);
        }
        Err(_) => {
            // Overflow: cap at i128::MAX
            position.interest_accrued = i128::MAX;
        }
    }
    position
}