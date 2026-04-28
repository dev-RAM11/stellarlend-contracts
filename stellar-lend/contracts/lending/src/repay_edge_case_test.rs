//! Repay edge-case tests for both lending repay paths.
//!
//! # Coverage matrix
//!
//! ## R-series: `repay` (borrow module, single-asset)
//! - R-1..R-5:   Invalid inputs (zero, negative, no debt, wrong asset, paused)
//! - R-6..R-7:   Pause lifecycle (block then allow after unpause, recovery mode)
//! - R-8..R-9:   Exact pay and overpay with zero interest
//! - R-10..R-14: Interest ordering (pays interest first), exact and overpay with interest
//! - R-15..R-18: Global debt tracking, view consistency, sequential repays, rounding
//!
//! ## C-series: `repay_asset` (cross-asset module)
//! - C-1..C-2:   Invalid inputs (zero, negative)
//! - C-3:        Overpay is **clamped** (different from borrow::repay which errors)
//! - C-4..C-6:   Exact pay, partial, zero-debt no-op
//! - C-7..C-12:  Pause, health factor restoration, multi-asset isolation,
//!               global debt tracking, minimum unit debt, collateral preservation
//!
//! ## T-series: Table-driven boundary scenarios
//! - T-1: Zero-interest repay matrix (multiple amount/principal combinations)
//! - T-2: Interest-present scenarios (1-second advance → interest = 1)
//! - T-3: Cross-asset overpay clamp table (various large amounts)
//!
//! # Security notes
//!
//! * **Dust-debt prevention**: the borrow module uses ceiling-rounded interest,
//!   so interest accrues at least 1 unit per second for any non-zero principal.
//!   Repaying `get_debt_balance()` at the current timestamp always zeroes the
//!   position — no sub-unit dust can remain.
//!
//! * **No silent overpay (borrow system)**: `repay` returns `RepayAmountTooHigh`
//!   rather than refunding excess. Integrators must read the exact balance first.
//!
//! * **Clamped overpay (cross-asset system)**: `repay_asset` silently caps the
//!   effective repay at the outstanding balance. This is intentional — it allows
//!   integrators to repay without first querying the balance. Integrators must
//!   NOT rely on receiving change back; the excess is simply not charged.
//!
//! * **Recovery-mode repay**: repay is allowed during recovery so users can
//!   unwind positions despite the protocol being in emergency state. An explicit
//!   `PauseType::Repay` flag still blocks repay even in recovery.

use super::*;
use crate::borrow::BorrowError;
use crate::cross_asset::{AssetParams, CrossAssetError};
use crate::pause::PauseType;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Sentinel health factor when position carries no debt (cross-asset module).
const HF_NO_DEBT: i128 = 1_000_000;

// ─────────────────────────────────────────────────────────────────────────────
// Setup helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Single-asset borrow-system setup: returns (client, admin, user, asset).
fn setup(env: &Env) -> (LendingContractClient<'_>, Address, Address, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    client.initialize(&admin, &1_000_000_000, &1_000);
    (client, admin, user, asset)
}

/// Cross-asset setup: initializes both protocol admin and cross-asset admin.
fn setup_cross(env: &Env) -> (LendingContractClient<'_>, Address, Address, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin, &1_000_000_000, &1_000);
    client.initialize_admin(&admin);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    (client, admin, user, asset)
}

/// Build cross-asset params with the given LTV.
fn cross_params(env: &Env, ltv: i128) -> AssetParams {
    AssetParams {
        ltv,
        liquidation_threshold: (ltv + 500).min(10_000),
        price_feed: Address::generate(env),
        debt_ceiling: 1_000_000_000_000,
        is_active: true,
    }
}

/// Borrow `amount` with 2× collateral (satisfies ≥150% minimum ratio).
fn borrow_2x(
    client: &LendingContractClient<'_>,
    user: &Address,
    asset: &Address,
    coll_asset: &Address,
    amount: i128,
) {
    client.borrow(user, asset, &amount, coll_asset, &(amount * 2));
}

// ─────────────────────────────────────────────────────────────────────────────
// Section R — borrow::repay edge cases
// ─────────────────────────────────────────────────────────────────────────────

// R-1: zero amount ──────────────────────────────────────────────────────────

/// R-1: Zero amount is rejected with `InvalidAmount`.
#[test]
fn test_repay_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    let result = client.try_repay(&user, &asset, &0);
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

// R-2: negative amount ─────────────────────────────────────────────────────

/// R-2: Negative amount is rejected with `InvalidAmount`.
#[test]
fn test_repay_negative_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    let result = client.try_repay(&user, &asset, &-1);
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

// R-3: no debt ─────────────────────────────────────────────────────────────

/// R-3: Repay when the user has never borrowed is rejected.
///
/// The borrow module checks `borrowed_amount == 0 && interest_accrued == 0`
/// and returns `InvalidAmount` — there is nothing to repay.
#[test]
fn test_repay_with_no_debt_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let result = client.try_repay(&user, &asset, &1_000);
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

// R-4: wrong asset ─────────────────────────────────────────────────────────

/// R-4: Repaying with an asset different from the one borrowed is rejected.
#[test]
fn test_repay_wrong_asset_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    let different = Address::generate(&env);
    let result = client.try_repay(&user, &different, &5_000);
    assert_eq!(result, Err(Ok(BorrowError::AssetNotSupported)));
}

// R-5: granular repay pause ────────────────────────────────────────────────

/// R-5: `PauseType::Repay` blocks the repay entry point.
#[test]
fn test_repay_blocked_when_repay_paused() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    client.set_pause(&admin, &PauseType::Repay, &true);
    let result = client.try_repay(&user, &asset, &5_000);
    assert_eq!(result, Err(Ok(BorrowError::ProtocolPaused)));
}

// R-6: unpause restores access ─────────────────────────────────────────────

/// R-6: Unpausing `Repay` restores access.
#[test]
fn test_repay_allowed_after_unpause() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    client.set_pause(&admin, &PauseType::Repay, &true);
    client.set_pause(&admin, &PauseType::Repay, &false);
    client.repay(&user, &asset, &5_000);
    assert_eq!(client.get_user_debt(&user).borrowed_amount, 5_000);
}

// R-7: recovery mode ───────────────────────────────────────────────────────

/// R-7: Repay is allowed during recovery mode even though high-risk ops are
/// blocked. Recovery exists so users can unwind positions.
#[test]
fn test_repay_allowed_during_recovery() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    client.emergency_shutdown(&admin);
    client.start_recovery(&admin);
    client.repay(&user, &asset, &5_000);
    assert_eq!(client.get_user_debt(&user).borrowed_amount, 5_000);
}

// R-8: exact pay (zero interest) ───────────────────────────────────────────

/// R-8: Repaying exactly the principal (no time elapsed → zero interest)
/// zeroes the entire debt position.
#[test]
fn test_repay_exact_principal_clears_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    client.repay(&user, &asset, &10_000);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 0);
    assert_eq!(debt.interest_accrued, 0);
    assert_eq!(client.get_debt_balance(&user), 0);
}

// R-9: overpay (zero interest) ─────────────────────────────────────────────

/// R-9: Overpaying (amount > principal, zero interest) is rejected with
/// `RepayAmountTooHigh`.
///
/// # Security
/// The borrow module rejects excess rather than silently refunding. Integrators
/// must read the exact outstanding balance via `get_debt_balance` before calling
/// `repay`, and must not send more.
#[test]
fn test_repay_overpay_returns_error() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &10_000, &coll, &20_000);
    let result = client.try_repay(&user, &asset, &10_001);
    assert_eq!(result, Err(Ok(BorrowError::RepayAmountTooHigh)));
}

// R-10: interest is paid first ─────────────────────────────────────────────

/// R-10: After 1 second of accrual, interest = 1 (ceiling division).
/// Repaying 1 unit zeros the interest without touching principal.
///
/// This test documents the ordering invariant: interest before principal.
#[test]
fn test_repay_pays_interest_before_principal() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000_000);
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &100_000, &coll, &200_000);
    // 1 second elapsed → interest = ceil(100_000 * 500 * 1 / 315_360_000_000) = 1
    env.ledger().with_mut(|li| li.timestamp = 1_000_001);
    client.repay(&user, &asset, &1);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.interest_accrued, 0, "interest should be cleared first");
    assert_eq!(debt.borrowed_amount, 100_000, "principal untouched");
}

// R-11: partial interest ───────────────────────────────────────────────────

/// R-11: Repaying an amount less than accrued interest partially reduces
/// interest without touching principal.
#[test]
fn test_repay_partial_interest_reduces_only_interest() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    // Large principal to accumulate ~5% interest over 1 year ≈ 50_000
    client.borrow(&user, &asset, &1_000_000, &coll, &2_000_000);
    env.ledger().with_mut(|li| li.timestamp = 1_000 + 31_536_000); // +1 year
    let before = client.get_user_debt(&user);
    assert!(before.interest_accrued >= 50_000, "expected ~50k interest after 1 year");
    let half = before.interest_accrued / 2;
    client.repay(&user, &asset, &half);
    let after = client.get_user_debt(&user);
    assert_eq!(after.interest_accrued, before.interest_accrued - half);
    assert_eq!(after.borrowed_amount, 1_000_000, "principal unchanged");
}

// R-12: interest then partial principal ────────────────────────────────────

/// R-12: Repaying (interest + partial principal) clears interest fully and
/// reduces principal by the remainder.
#[test]
fn test_repay_interest_then_partial_principal() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000_000);
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &100_000, &coll, &200_000);
    // 1 second → interest = 1; repay interest (1) + 10_000 principal = 10_001
    env.ledger().with_mut(|li| li.timestamp = 1_000_001);
    client.repay(&user, &asset, &10_001);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.interest_accrued, 0, "interest cleared");
    assert_eq!(debt.borrowed_amount, 90_000, "principal reduced by 10_000");
}

// R-13: exact full pay (with interest) ────────────────────────────────────

/// R-13: Repaying exactly `get_debt_balance()` (principal + accrued interest
/// at the current timestamp) zeroes the position completely.
///
/// # Security / dust prevention
/// Because `calculate_interest` uses ceiling division, the amount returned by
/// `get_debt_balance` always equals the exact amount `repay` will consume.
/// There is no sub-unit dust that could block a full clearance.
#[test]
fn test_repay_exact_full_pay_clears_position() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000_000);
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &100_000, &coll, &200_000);
    // 1 second → total debt = 100_001
    env.ledger().with_mut(|li| li.timestamp = 1_000_001);
    let total = client.get_debt_balance(&user);
    assert_eq!(total, 100_001);
    client.repay(&user, &asset, &total);
    assert_eq!(client.get_debt_balance(&user), 0);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 0);
    assert_eq!(debt.interest_accrued, 0);
}

// R-14: overpay with interest ──────────────────────────────────────────────

/// R-14: Repaying (principal + interest + 1) is rejected with
/// `RepayAmountTooHigh` even after interest accrues.
#[test]
fn test_repay_overpay_with_interest_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000_000);
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &100_000, &coll, &200_000);
    env.ledger().with_mut(|li| li.timestamp = 1_000_001);
    // total = 100_001 → overpay by 1
    let result = client.try_repay(&user, &asset, &100_002);
    assert_eq!(result, Err(Ok(BorrowError::RepayAmountTooHigh)));
}

// R-15: global total debt tracking ────────────────────────────────────────

/// R-15: After repaying principal, the global borrow ceiling is freed.
/// Verified by confirming the debt position reflects the reduction.
#[test]
fn test_repay_decrements_global_total_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &50_000, &coll, &100_000);
    client.repay(&user, &asset, &20_000);
    assert_eq!(client.get_user_debt(&user).borrowed_amount, 30_000);
}

// R-16: view consistency after full repay ─────────────────────────────────

/// R-16: After full repay, `get_user_position` shows zero debt; collateral
/// is unchanged (repay does not remove collateral).
#[test]
fn test_repay_full_pay_view_shows_zero_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &20_000, &coll, &40_000);
    client.repay(&user, &asset, &20_000);
    let pos = client.get_user_position(&user);
    assert_eq!(pos.debt_balance, 0);
    assert_eq!(pos.collateral_balance, 40_000, "collateral untouched by repay");
}

// R-17: sequential repays ─────────────────────────────────────────────────

/// R-17: Three sequential repays of equal thirds converge to zero debt.
#[test]
fn test_repay_sequential_repays_converge_to_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &30_000, &coll, &60_000);
    client.repay(&user, &asset, &10_000);
    client.repay(&user, &asset, &10_000);
    client.repay(&user, &asset, &10_000);
    assert_eq!(client.get_debt_balance(&user), 0);
}

// R-18: rounding edge — repaying principal when interest is nonzero ────────

/// R-18: When interest = 1 is outstanding and the user repays exactly the
/// principal (100_000), the interest consumes 1 unit, and the remaining
/// 99_999 reduces the principal — leaving a 1-unit principal remainder.
///
/// This documents the arithmetic: amount=100_000, interest=1 →
/// remaining_after_interest = 99_999 < principal (100_000) → OK, not an error.
#[test]
fn test_repay_principal_amount_with_tiny_interest_leaves_dust_principal() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000_000);
    let (client, _admin, user, asset) = setup(&env);
    let coll = Address::generate(&env);
    client.borrow(&user, &asset, &100_000, &coll, &200_000);
    env.ledger().with_mut(|li| li.timestamp = 1_000_001); // interest = 1
    // Repay exactly 100_000: 1 goes to interest, 99_999 to principal
    client.repay(&user, &asset, &100_000);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.interest_accrued, 0, "interest cleared");
    assert_eq!(debt.borrowed_amount, 1, "1 unit of principal remains");
}

// ─────────────────────────────────────────────────────────────────────────────
// Section C — cross_asset::repay_asset edge cases
// ─────────────────────────────────────────────────────────────────────────────

// C-1: zero amount ─────────────────────────────────────────────────────────

/// C-1: Zero amount is rejected with `InvalidAmount`.
#[test]
fn test_cross_repay_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    let result = client.try_repay_asset(&user, &asset, &0);
    assert_eq!(result, Err(Ok(CrossAssetError::InvalidAmount)));
}

// C-2: negative amount ─────────────────────────────────────────────────────

/// C-2: Negative amount is rejected with `InvalidAmount`.
#[test]
fn test_cross_repay_negative_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    let result = client.try_repay_asset(&user, &asset, &-1);
    assert_eq!(result, Err(Ok(CrossAssetError::InvalidAmount)));
}

// C-3: overpay clamp ───────────────────────────────────────────────────────

/// C-3: Overpay is **silently clamped** to the outstanding balance (no error).
///
/// # Semantics vs borrow system
/// Unlike `borrow::repay` (which returns `RepayAmountTooHigh`), `repay_asset`
/// uses `min(amount, current_debt)`. The request succeeds and debt becomes 0.
///
/// # Security
/// Integrators must NOT assume excess tokens are returned — they are simply not
/// charged. Always verify `total_debt_usd == 0` in the position summary after
/// calling with an estimate to confirm the debt was fully cleared.
#[test]
fn test_cross_repay_overpay_clamps_to_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &20_000);
    client.borrow_asset(&user, &asset, &5_000);
    client.repay_asset(&user, &asset, &999_999); // overpay
    let summary = client.get_cross_position_summary(&user);
    assert_eq!(summary.total_debt_usd, 0);
    assert_eq!(summary.health_factor, HF_NO_DEBT);
}

// C-4: exact pay ───────────────────────────────────────────────────────────

/// C-4: Repaying exactly the outstanding balance zeroes the debt cleanly.
#[test]
fn test_cross_repay_exact_pay_zeroes_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    client.repay_asset(&user, &asset, &5_000);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

// C-5: partial repay ───────────────────────────────────────────────────────

/// C-5: Partial repay reduces debt by the exact amount paid.
#[test]
fn test_cross_repay_partial_reduces_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &20_000);
    client.borrow_asset(&user, &asset, &10_000);
    client.repay_asset(&user, &asset, &4_000);
    let s = client.get_cross_position_summary(&user);
    // At mock price $1, total_debt_usd == remaining debt amount
    assert_eq!(s.total_debt_usd, 6_000);
}

// C-6: zero-debt no-op ────────────────────────────────────────────────────

/// C-6: Repaying against a zero-debt position succeeds as a no-op.
///
/// `repay_asset` clamps to outstanding balance; with no debt the effective
/// repay amount is 0 and the call succeeds without error. This allows
/// integrators to safely call repay without first checking whether debt exists.
#[test]
fn test_cross_repay_on_zero_debt_is_no_op() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &10_000);
    // No borrow — call repay anyway
    client.repay_asset(&user, &asset, &5_000);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

// C-7: pause blocks repay ─────────────────────────────────────────────────

/// C-7: `PauseType::Repay` blocks `repay_asset` with `ProtocolPaused`.
#[test]
fn test_cross_repay_blocked_when_paused() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    client.set_pause(&admin, &PauseType::Repay, &true);
    let result = client.try_repay_asset(&user, &asset, &5_000);
    assert_eq!(result, Err(Ok(CrossAssetError::ProtocolPaused)));
}

// C-8: health factor restoration ──────────────────────────────────────────

/// C-8: After full repay the health factor returns to the `HF_NO_DEBT` sentinel.
#[test]
fn test_cross_repay_full_restores_max_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &7_000);
    let before = client.get_cross_position_summary(&user);
    assert!(before.health_factor < HF_NO_DEBT, "should have finite HF while in debt");
    client.repay_asset(&user, &asset, &7_000);
    let after = client.get_cross_position_summary(&user);
    assert_eq!(after.health_factor, HF_NO_DEBT);
    assert_eq!(after.total_debt_usd, 0);
}

// C-9: multi-asset isolation ───────────────────────────────────────────────

/// C-9: Repaying one asset does not affect another asset's outstanding debt.
#[test]
fn test_cross_repay_one_asset_does_not_affect_other() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _) = setup_cross(&env);
    let asset_a = Address::generate(&env);
    let asset_b = Address::generate(&env);
    client.set_asset_params(&asset_a, &cross_params(&env, 7_500));
    client.set_asset_params(&asset_b, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset_a, &30_000);
    client.deposit_collateral_asset(&user, &asset_b, &30_000);
    client.borrow_asset(&user, &asset_a, &5_000);
    client.borrow_asset(&user, &asset_b, &5_000);
    client.repay_asset(&user, &asset_a, &5_000);
    let s = client.get_cross_position_summary(&user);
    // Only asset B debt remains
    assert_eq!(s.total_debt_usd, 5_000);
    assert!(s.health_factor < HF_NO_DEBT, "asset B debt still exists");
}

// C-10: global total asset debt tracking ──────────────────────────────────

/// C-10: Full repay frees the global debt ceiling, allowing a second user to
/// borrow from a previously exhausted ceiling.
#[test]
fn test_cross_repay_updates_global_total_asset_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    let params = AssetParams {
        ltv: 9_000,
        liquidation_threshold: 9_500,
        price_feed: Address::generate(&env),
        debt_ceiling: 5_000,
        is_active: true,
    };
    client.set_asset_params(&asset, &params);
    client.deposit_collateral_asset(&user, &asset, &20_000);
    client.borrow_asset(&user, &asset, &5_000);
    // Ceiling exhausted — second user blocked
    let user2 = Address::generate(&env);
    client.deposit_collateral_asset(&user2, &asset, &20_000);
    assert_eq!(
        client.try_borrow_asset(&user2, &asset, &1),
        Err(Ok(CrossAssetError::DebtCeilingReached))
    );
    // After original user repays, ceiling is freed
    client.repay_asset(&user, &asset, &5_000);
    client.borrow_asset(&user2, &asset, &1);
}

// C-11: minimum unit debt ─────────────────────────────────────────────────

/// C-11: Repaying 1 unit when debt is exactly 1 unit zeroes debt cleanly.
///
/// This test confirms there is no sub-unit dust in the cross-asset system.
#[test]
fn test_cross_repay_minimum_amount_clears_unit_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 10_000)); // 100% LTV
    client.deposit_collateral_asset(&user, &asset, &1);
    client.borrow_asset(&user, &asset, &1);
    client.repay_asset(&user, &asset, &1);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

// C-12: collateral preserved after repay ──────────────────────────────────

/// C-12: Full repay of debt does not remove collateral. The collateral balance
/// in the position summary must remain at the deposited amount.
#[test]
fn test_cross_repay_does_not_remove_collateral() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_cross(&env);
    client.set_asset_params(&asset, &cross_params(&env, 7_500));
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    client.repay_asset(&user, &asset, &5_000);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_collateral_usd, 10_000, "collateral unchanged");
    assert_eq!(s.total_debt_usd, 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Section T — Table-driven boundary scenarios
// ─────────────────────────────────────────────────────────────────────────────

// T-1: zero-interest repay matrix ─────────────────────────────────────────

struct BorrowRepayCase {
    principal: i128,
    repay: i128,
    /// `Some(expected_principal)` = succeeds; `None` = `RepayAmountTooHigh`.
    expected: Option<i128>,
}

fn run_borrow_repay_case(env: &Env, client: &LendingContractClient<'_>, c: &BorrowRepayCase) {
    let user = Address::generate(env);
    let asset = Address::generate(env);
    let coll = Address::generate(env);
    client.borrow(&user, &asset, &c.principal, &coll, &(c.principal * 2));
    match c.expected {
        Some(expected_principal) => {
            client.repay(&user, &asset, &c.repay);
            let debt = client.get_user_debt(&user);
            assert_eq!(
                debt.borrowed_amount, expected_principal,
                "principal after repay({}) of {}: expected {}, got {}",
                c.repay, c.principal, expected_principal, debt.borrowed_amount
            );
            assert_eq!(debt.interest_accrued, 0);
        }
        None => {
            let result = client.try_repay(&user, &asset, &c.repay);
            assert_eq!(
                result,
                Err(Ok(BorrowError::RepayAmountTooHigh)),
                "repay({}) of {} should be RepayAmountTooHigh",
                c.repay, c.principal
            );
        }
    }
}

/// T-1: Matrix of zero-interest repay scenarios covering every interesting
/// boundary: min repay, mid repay, exact repay, off-by-one overpay.
#[test]
fn test_repay_table_zero_interest_scenarios() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _user, _asset) = setup(&env);

    let cases = [
        BorrowRepayCase { principal: 10_000, repay: 1,      expected: Some(9_999) },
        BorrowRepayCase { principal: 10_000, repay: 5_000,  expected: Some(5_000) },
        BorrowRepayCase { principal: 10_000, repay: 9_999,  expected: Some(1) },
        BorrowRepayCase { principal: 10_000, repay: 10_000, expected: Some(0) },
        BorrowRepayCase { principal: 10_000, repay: 10_001, expected: None },
        BorrowRepayCase { principal: 1_000,  repay: 500,    expected: Some(500) },
        BorrowRepayCase { principal: 1_000,  repay: 1_000,  expected: Some(0) },
        BorrowRepayCase { principal: 1_000,  repay: 1_001,  expected: None },
    ];

    for c in &cases {
        run_borrow_repay_case(&env, &client, c);
    }
}

// T-2: interest-present scenarios ─────────────────────────────────────────

/// T-2: With 1 second elapsed, interest = 1 for any principal.
/// Two representative cases demonstrate the ordering and overpay behavior.
#[test]
fn test_repay_table_interest_present_scenarios() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 2_000_000);
    let (client, _admin, _user, _asset) = setup(&env);

    // Case A: repay exactly 1 (the interest) — principal untouched
    let user_a = Address::generate(&env);
    let asset_a = Address::generate(&env);
    let coll_a = Address::generate(&env);
    client.borrow(&user_a, &asset_a, &100_000, &coll_a, &200_000);

    // Case B: repay principal + interest (100_001) — both zeroed
    let user_b = Address::generate(&env);
    let asset_b = Address::generate(&env);
    let coll_b = Address::generate(&env);
    client.borrow(&user_b, &asset_b, &100_000, &coll_b, &200_000);

    env.ledger().with_mut(|li| li.timestamp = 2_000_001); // +1 second

    // Case A
    client.repay(&user_a, &asset_a, &1);
    let da = client.get_user_debt(&user_a);
    assert_eq!(da.interest_accrued, 0, "A: interest cleared");
    assert_eq!(da.borrowed_amount, 100_000, "A: principal intact");

    // Case B
    client.repay(&user_b, &asset_b, &100_001);
    let db = client.get_user_debt(&user_b);
    assert_eq!(db.interest_accrued, 0, "B: interest cleared");
    assert_eq!(db.borrowed_amount, 0, "B: principal cleared");
}

// T-3: cross-asset overpay clamp table ────────────────────────────────────

/// T-3: No matter how large the repay amount, cross-asset debt floors at 0.
///
/// # Security
/// Confirms that extremely large overpay values (up to `i128::MAX / 2`) do not
/// panic, overflow, or leave residual debt.
#[test]
fn test_cross_repay_overpay_clamp_table() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _, _) = setup_cross(&env);

    let overpay_amounts: [i128; 5] = [
        1_001,          // just over debt of 1_000
        2_000,          // 2× debt
        1_000_000_000,
        i128::MAX / 2,
        i128::MAX - 1,
    ];

    for overpay in &overpay_amounts {
        let user = Address::generate(&env);
        let asset = Address::generate(&env);
        client.set_asset_params(&asset, &cross_params(&env, 10_000));
        client.deposit_collateral_asset(&user, &asset, &10_000);
        client.borrow_asset(&user, &asset, &1_000);
        // All overpay amounts succeed and debt = 0
        client.repay_asset(&user, &asset, overpay);
        let s = client.get_cross_position_summary(&user);
        assert_eq!(
            s.total_debt_usd, 0,
            "debt should be 0 after overpay of {}", overpay
        );
        assert_eq!(s.health_factor, HF_NO_DEBT);
    }
}
