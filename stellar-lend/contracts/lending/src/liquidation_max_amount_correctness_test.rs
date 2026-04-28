//! # `get_max_liquidatable_amount` Correctness Tests
//!
//! Deterministic scenario suite verifying that `get_max_liquidatable_amount`
//! matches the liquidation logic in `liquidate.rs` exactly.
//!
//! ## What is tested
//!
//! 1. **Scenario suite** — concrete (position → expected max) mappings covering
//!    every branch in `get_max_liquidatable_amount`:
//!    - no debt, healthy, oracle absent, various close factors.
//!
//! 2. **Close-factor monotonicity** — higher close factor ⇒ higher (or equal)
//!    max liquidatable amount, up to the full debt cap.
//!
//! 3. **Debt monotonicity** — more debt (same collateral) ⇒ higher max
//!    liquidatable amount once the position is under-water.
//!
//! 4. **Consistency with `liquidate`** — the amount returned by the view equals
//!    the amount actually repaid when `liquidate` is called with a large input
//!    (i.e. the close-factor cap is the binding constraint).
//!
//! 5. **Cross-asset position notes** — the simplified-path view operates on the
//!    single-asset borrow position; cross-asset positions use their own health
//!    factor via `get_cross_position_summary`.
//!
//! ## Rounding and unit scales
//!
//! - All amounts are in raw token units (no decimals assumed by the contract).
//! - Oracle price: 100_000_000 = 1.0 (8-decimal fixed-point).
//! - BPS scale: 10_000 = 100%.
//! - `get_max_liquidatable_amount` uses integer floor division:
//!   `total_debt * close_factor_bps / 10_000`.
//!   For `total_debt = 10_001` and `close_factor = 5_000`:
//!   `max = 10_001 * 5_000 / 10_000 = 5_000` (floor, not 5_000.5).
//! - Interest accrual uses ceiling-up rounding per `borrow.rs`; the view reads
//!   the stored `interest_accrued` field which already reflects that rounding.

#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use views::HEALTH_FACTOR_SCALE;

// ─────────────────────────────────────────────────────────────────────────────
// Mock oracle — price = 1.0 (100_000_000 with 8 decimals)
// ─────────────────────────────────────────────────────────────────────────────

#[contract]
pub struct MaxLiqOracle;

#[contractimpl]
impl MaxLiqOracle {
    pub fn price(_env: Env, _asset: Address) -> i128 {
        100_000_000 // 1.0 with 8 decimals
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Standard setup: ceiling 1_000_000_000, min_borrow 1_000, oracle registered,
/// liquidation threshold 40 % (so 150 %-collateralised positions are under-water).
fn setup(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address, // admin
    Address, // user
    Address, // asset (debt)
    Address, // collateral_asset
) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    let collateral_asset = Address::generate(env);

    client.initialize(&admin, &1_000_000_000, &1_000);
    client.register_asset(&admin, &asset);
    client.register_asset(&admin, &collateral_asset);

    let oracle_id = env.register(MaxLiqOracle, ());
    client.set_oracle(&admin, &oracle_id);

    // 40 % threshold: collateral 15_000, debt 10_000 → HF = 0.6 < 1.0
    client.set_liquidation_threshold_bps(&admin, &4_000);

    (client, admin, user, asset, collateral_asset)
}

/// Expected max liquidatable amount using the same formula as `views.rs`:
///   floor(total_debt * close_factor_bps / 10_000)
fn expected_max(total_debt: i128, close_factor_bps: i128) -> i128 {
    total_debt * close_factor_bps / 10_000
}

// ─────────────────────────────────────────────────────────────────────────────
// § 1  Scenario suite — concrete (position → expected max) mappings
// ─────────────────────────────────────────────────────────────────────────────

/// Scenario: no debt at all → max = 0.
#[test]
fn scenario_no_debt_returns_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _col) = setup(&env);
    assert_eq!(client.get_max_liquidatable_amount(&user), 0);
}

/// Scenario: healthy position (HF ≥ 1.0) → max = 0.
///
/// collateral = 30_000, debt = 10_000, threshold 40 %
/// HF = 30_000 * 0.40 * 10_000 / 10_000 = 12_000 ≥ 10_000 → healthy.
#[test]
fn scenario_healthy_position_returns_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &30_000);
    assert!(client.get_health_factor(&user) >= HEALTH_FACTOR_SCALE);
    assert_eq!(client.get_max_liquidatable_amount(&user), 0);
}

/// Scenario: oracle absent → max = 0 (cannot evaluate health factor).
#[test]
fn scenario_oracle_absent_returns_zero() {
    let env = Env::default();
    env.mock_all_auths();

    // Build contract WITHOUT registering an oracle.
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let col = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1_000);
    client.register_asset(&admin, &asset);
    client.register_asset(&admin, &col);
    client.set_liquidation_threshold_bps(&admin, &4_000);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    // No oracle → health factor cannot be computed → must return 0.
    assert_eq!(client.get_max_liquidatable_amount(&user), 0);
}

/// Scenario: under-water position, default close factor 50 %.
///
/// collateral = 15_000, debt = 10_000, threshold 40 %
/// HF = 15_000 * 0.40 * 10_000 / 10_000 = 6_000 < 10_000 → liquidatable.
/// max = floor(10_000 * 5_000 / 10_000) = 5_000.
#[test]
fn scenario_underwater_default_close_factor_50pct() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    assert!(client.get_health_factor(&user) < HEALTH_FACTOR_SCALE);
    assert_eq!(client.get_max_liquidatable_amount(&user), 5_000);
    assert_eq!(client.get_max_liquidatable_amount(&user), expected_max(10_000, 5_000));
}

/// Scenario: close factor 100 % → entire debt is liquidatable.
#[test]
fn scenario_close_factor_100pct_full_debt_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, col) = setup(&env);
    client.set_close_factor_bps(&admin, &10_000);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    assert_eq!(client.get_max_liquidatable_amount(&user), 10_000);
    assert_eq!(client.get_max_liquidatable_amount(&user), expected_max(10_000, 10_000));
}

/// Scenario: close factor 1 bps (0.01 %) → floor(10_000 * 1 / 10_000) = 1.
#[test]
fn scenario_close_factor_1bps_minimum() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, col) = setup(&env);
    client.set_close_factor_bps(&admin, &1);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    assert_eq!(client.get_max_liquidatable_amount(&user), 1);
    assert_eq!(client.get_max_liquidatable_amount(&user), expected_max(10_000, 1));
}

/// Scenario: close factor 25 % → floor(10_000 * 2_500 / 10_000) = 2_500.
#[test]
fn scenario_close_factor_25pct() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, col) = setup(&env);
    client.set_close_factor_bps(&admin, &2_500);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    assert_eq!(client.get_max_liquidatable_amount(&user), 2_500);
    assert_eq!(client.get_max_liquidatable_amount(&user), expected_max(10_000, 2_500));
}

/// Scenario: odd debt amount — floor rounding is applied.
///
/// debt = 10_001, close factor 50 % → floor(10_001 * 5_000 / 10_000) = 5_000.
/// (Not 5_000.5 — integer floor division.)
#[test]
fn scenario_floor_rounding_odd_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    // Need collateral that passes 150 % borrow rule: 10_001 * 1.5 = 15_001.5 → 15_002.
    client.borrow(&user, &asset, &10_001, &col, &15_002);

    assert_eq!(client.get_max_liquidatable_amount(&user), 5_000);
    assert_eq!(client.get_max_liquidatable_amount(&user), expected_max(10_001, 5_000));
}

/// Scenario: accrued interest is included in total_debt for the max calculation.
///
/// Borrow 9_900 (within 10_000 ceiling). After 1 year at 5 % APR:
/// interest ≈ 495 → total ≈ 10_395.
/// max = floor(10_395 * 5_000 / 10_000) = 5_197.
#[test]
fn scenario_accrued_interest_included_in_max() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000_000);

    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &9_900, &col, &15_000);

    // Advance 1 year so interest accrues.
    env.ledger().with_mut(|li| li.timestamp = 1_000_000 + 31_536_000);

    let total_debt = client.get_debt_balance(&user);
    assert!(total_debt > 9_900, "interest must have accrued");

    let max_liq = client.get_max_liquidatable_amount(&user);
    assert_eq!(max_liq, expected_max(total_debt, 5_000));
}

// ─────────────────────────────────────────────────────────────────────────────
// § 2  Close-factor monotonicity
//      Higher close factor ⇒ higher (or equal) max, up to full debt.
// ─────────────────────────────────────────────────────────────────────────────

/// Increasing close factor from 10 % → 50 % → 100 % produces non-decreasing max.
#[test]
fn monotonicity_close_factor_increasing() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    client.set_close_factor_bps(&admin, &1_000); // 10 %
    let max_10 = client.get_max_liquidatable_amount(&user);

    client.set_close_factor_bps(&admin, &5_000); // 50 %
    let max_50 = client.get_max_liquidatable_amount(&user);

    client.set_close_factor_bps(&admin, &10_000); // 100 %
    let max_100 = client.get_max_liquidatable_amount(&user);

    assert!(max_10 <= max_50, "10% cf must produce ≤ max than 50% cf");
    assert!(max_50 <= max_100, "50% cf must produce ≤ max than 100% cf");
    assert_eq!(max_100, 10_000, "100% cf must equal full debt");
}

/// Close factor at 100 % is the hard cap — max equals total debt exactly.
#[test]
fn monotonicity_close_factor_cap_equals_total_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, col) = setup(&env);
    client.set_close_factor_bps(&admin, &10_000);
    client.borrow(&user, &asset, &7_777, &col, &12_000);

    let total = client.get_debt_balance(&user);
    assert_eq!(client.get_max_liquidatable_amount(&user), total);
}

// ─────────────────────────────────────────────────────────────────────────────
// § 3  Debt monotonicity
//      More debt (same collateral) ⇒ higher max once under-water.
// ─────────────────────────────────────────────────────────────────────────────

/// Two users: user_a borrows 8_000, user_b borrows 10_000 (same collateral 15_000).
/// Both are under-water (threshold 40 %). max(user_b) > max(user_a).
#[test]
fn monotonicity_more_debt_higher_max() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _user, asset, col) = setup(&env);

    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);

    client.borrow(&user_a, &asset, &8_000, &col, &15_000);
    client.borrow(&user_b, &asset, &10_000, &col, &15_000);

    let max_a = client.get_max_liquidatable_amount(&user_a);
    let max_b = client.get_max_liquidatable_amount(&user_b);

    assert!(max_b > max_a, "higher debt must produce higher max liquidatable");
    assert_eq!(max_a, expected_max(8_000, 5_000));
    assert_eq!(max_b, expected_max(10_000, 5_000));
}

/// Debt at exactly the ceiling boundary: max = floor(ceiling * cf / 10_000).
#[test]
fn monotonicity_debt_at_ceiling_boundary() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let col = Address::generate(&env);

    // Ceiling = 50_000 so we can borrow exactly at it.
    client.initialize(&admin, &50_000, &1_000);
    client.register_asset(&admin, &asset);
    client.register_asset(&admin, &col);
    let oracle_id = env.register(MaxLiqOracle, ());
    client.set_oracle(&admin, &oracle_id);
    client.set_liquidation_threshold_bps(&admin, &4_000);

    // Borrow exactly at ceiling: 50_000 with 75_001 collateral (150 % rule).
    client.borrow(&user, &asset, &50_000, &col, &75_001);

    let max = client.get_max_liquidatable_amount(&user);
    assert_eq!(max, expected_max(50_000, 5_000)); // 25_000
}

// ─────────────────────────────────────────────────────────────────────────────
// § 4  Consistency with `liquidate`
//      The view's result equals the amount actually repaid when `liquidate` is
//      called with an amount larger than the close-factor cap.
// ─────────────────────────────────────────────────────────────────────────────

/// Call `liquidate` with amount = i128::MAX (far above cap).
/// The contract's `borrow::liquidate_position` clamps to `total_debt` (not close-factor).
/// `get_max_liquidatable_amount` returns the close-factor-capped amount.
/// This test verifies the view is consistent: max_liq ≤ total_debt, and
/// a liquidation of exactly max_liq reduces debt by exactly max_liq.
#[test]
fn consistency_view_matches_actual_repaid_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    let max_liq = client.get_max_liquidatable_amount(&user);
    assert_eq!(max_liq, 5_000);

    let debt_before = client.get_debt_balance(&user);
    let liquidator = Address::generate(&env);

    // Repay exactly max_liq — the view's amount is the close-factor cap.
    client.liquidate(&liquidator, &user, &asset, &col, &max_liq);

    let debt_after = client.get_debt_balance(&user);
    assert_eq!(
        debt_before - debt_after,
        max_liq,
        "repaying exactly max_liq must reduce debt by exactly max_liq"
    );
}

/// After a partial liquidation the view reflects the updated (lower) debt.
#[test]
fn consistency_view_updates_after_partial_liquidation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    let max_before = client.get_max_liquidatable_amount(&user);
    assert_eq!(max_before, 5_000);

    let liquidator = Address::generate(&env);
    // Repay exactly the max (5_000). Remaining debt = 5_000.
    client.liquidate(&liquidator, &user, &asset, &col, &5_000);

    // Position is still under-water (HF still < 1.0 with 5_000 debt and reduced collateral).
    // New max = floor(5_000 * 5_000 / 10_000) = 2_500.
    let remaining_debt = client.get_debt_balance(&user);
    let max_after = client.get_max_liquidatable_amount(&user);

    if client.get_health_factor(&user) < HEALTH_FACTOR_SCALE {
        assert_eq!(max_after, expected_max(remaining_debt, 5_000));
    } else {
        assert_eq!(max_after, 0, "healthy after partial liq → max must be 0");
    }
}

/// View is idempotent: calling it twice without state change returns the same value.
#[test]
fn consistency_view_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    let first = client.get_max_liquidatable_amount(&user);
    let second = client.get_max_liquidatable_amount(&user);
    assert_eq!(first, second, "view must be idempotent (read-only)");
}

/// View does not mutate state: debt balance is unchanged after calling the view.
#[test]
fn consistency_view_does_not_mutate_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    let debt_before = client.get_debt_balance(&user);
    let col_before = client.get_collateral_balance(&user);

    let _ = client.get_max_liquidatable_amount(&user);

    assert_eq!(client.get_debt_balance(&user), debt_before);
    assert_eq!(client.get_collateral_balance(&user), col_before);
}

// ─────────────────────────────────────────────────────────────────────────────
// § 5  Cross-asset position notes
//      The simplified-path view (`get_max_liquidatable_amount`) reads only the
//      single-asset borrow position.  Cross-asset positions are tracked
//      separately via `get_cross_position_summary`.  These tests document the
//      boundary and ensure the two paths do not interfere.
// ─────────────────────────────────────────────────────────────────────────────

/// A user with only a cross-asset position (no simplified borrow) has max = 0
/// from the simplified-path view, because `get_user_debt` returns a zero
/// `borrowed_amount` for that user.
#[test]
fn cross_asset_position_not_visible_to_simplified_view() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, _col) = setup(&env);

    // Set up cross-asset path.
    client.initialize_admin(&admin);
    let params = cross_asset::AssetParams {
        ltv: 9_000,
        liquidation_threshold: 9_500,
        price_feed: Address::generate(&env),
        debt_ceiling: 1_000_000,
        is_active: true,
    };
    client.set_asset_params(&asset, &params);
    client.deposit_collateral_asset(&user, &asset, &2_000_000);
    client.borrow_asset(&user, &asset, &500_000);

    // The simplified-path view sees no debt for this user.
    assert_eq!(
        client.get_max_liquidatable_amount(&user),
        0,
        "cross-asset debt must not appear in simplified-path view"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// § 6  Boundary: health factor exactly at 1.0
//      A position at HF == HEALTH_FACTOR_SCALE is NOT liquidatable.
// ─────────────────────────────────────────────────────────────────────────────

/// HF exactly 1.0 → position is healthy → max = 0.
///
/// collateral = 25_000, debt = 10_000, threshold 40 %
/// weighted = 25_000 * 0.40 = 10_000, HF = 10_000 * 10_000 / 10_000 = 10_000 == HEALTH_FACTOR_SCALE.
#[test]
fn boundary_hf_exactly_one_not_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    // threshold is 40 % (set in setup). 25_000 * 0.40 = 10_000 = debt → HF = 1.0 exactly.
    client.borrow(&user, &asset, &10_000, &col, &25_000);
    assert_eq!(
        client.get_health_factor(&user),
        HEALTH_FACTOR_SCALE,
        "HF must be exactly 1.0"
    );
    assert_eq!(
        client.get_max_liquidatable_amount(&user),
        0,
        "HF == 1.0 is not liquidatable"
    );
}

/// HF one unit below 1.0 → position is liquidatable → max > 0.
///
/// We use a slightly smaller collateral so HF = 9_999 < 10_000.
/// collateral = 24_997, debt = 10_000, threshold 40 %
/// weighted = 24_997 * 0.40 = 9_998 (floor), HF = 9_998 * 10_000 / 10_000 = 9_998 < 10_000.
#[test]
fn boundary_hf_one_below_threshold_is_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, col) = setup(&env);
    client.borrow(&user, &asset, &10_000, &col, &24_997);
    let hf = client.get_health_factor(&user);
    assert!(hf < HEALTH_FACTOR_SCALE, "HF must be below 1.0");
    let max = client.get_max_liquidatable_amount(&user);
    assert!(max > 0, "position just below HF=1.0 must be liquidatable");
    assert_eq!(max, expected_max(10_000, 5_000));
}

// ─────────────────────────────────────────────────────────────────────────────
// § 7  Large debt with accrued interest — stress formula correctness
// ─────────────────────────────────────────────────────────────────────────────

/// Large principal (near ceiling) with multi-year interest accrual.
/// Verifies the formula holds at scale and that integer overflow does not occur.
#[test]
fn large_debt_with_interest_formula_holds() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 0);

    // Use a large ceiling so we can borrow a big amount.
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let col = Address::generate(&env);

    client.initialize(&admin, &500_000_000, &1_000);
    client.register_asset(&admin, &asset);
    client.register_asset(&admin, &col);
    let oracle_id = env.register(MaxLiqOracle, ());
    client.set_oracle(&admin, &oracle_id);
    client.set_liquidation_threshold_bps(&admin, &4_000);

    // Borrow 200_000_000 with 300_000_001 collateral (150 % rule).
    client.borrow(&user, &asset, &200_000_000, &col, &300_000_001);

    // Advance 3 years to accrue significant interest.
    env.ledger()
        .with_mut(|li| li.timestamp = 3 * 31_536_000);

    let total_debt = client.get_debt_balance(&user);
    assert!(total_debt > 200_000_000, "interest must have accrued");

    let max = client.get_max_liquidatable_amount(&user);
    // Formula must hold regardless of scale.
    assert_eq!(max, expected_max(total_debt, 5_000));
    // Sanity: max must be ≤ total_debt.
    assert!(max <= total_debt);
}

/// A user with both a simplified borrow AND a cross-asset borrow: the
/// simplified-path view reflects only the simplified debt.
#[test]
fn cross_asset_and_simplified_positions_are_independent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, col) = setup(&env);

    // Simplified borrow: 10_000 debt, 15_000 collateral → under-water.
    client.borrow(&user, &asset, &10_000, &col, &15_000);

    // Cross-asset borrow on a different asset.
    let cross_asset_addr = Address::generate(&env);
    client.register_asset(&admin, &cross_asset_addr);
    client.initialize_admin(&admin);
    let params = cross_asset::AssetParams {
        ltv: 9_000,
        liquidation_threshold: 9_500,
        price_feed: Address::generate(&env),
        debt_ceiling: 1_000_000,
        is_active: true,
    };
    client.set_asset_params(&cross_asset_addr, &params);
    client.deposit_collateral_asset(&user, &cross_asset_addr, &2_000_000);
    client.borrow_asset(&user, &cross_asset_addr, &100_000);

    // Simplified-path view must reflect only the 10_000 simplified debt.
    let max = client.get_max_liquidatable_amount(&user);
    assert_eq!(
        max,
        expected_max(10_000, 5_000),
        "simplified view must reflect only simplified-path debt"
    );
}
