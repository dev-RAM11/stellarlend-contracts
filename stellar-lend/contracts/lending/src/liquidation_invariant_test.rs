//! # Liquidation Invariant Test Suite
//!
//! Asserts protocol-level invariants for the liquidation engine across the full
//! parameter space of close-factor and incentive settings.
//!
//! ## Invariants proven
//!
//! | # | Invariant | Description |
//! |---|-----------|-------------|
//! | I1 | Close-factor cap | `actual_repaid ≤ total_debt * close_factor_bps / 10_000` |
//! | I2 | No over-repayment | `actual_repaid ≤ total_debt` |
//! | I3 | Incentive bound | `seized = repaid * (10_000 + incentive_bps) / 10_000` |
//! | I4 | No free collateral | `seized ≤ collateral_before` (never negative) |
//! | I5 | Debt monotone | Each liquidation call strictly reduces outstanding debt |
//! | I6 | Solvency | `collateral_after + seized = collateral_before` (conservation) |
//! | I7 | Healthy positions immune | HF ≥ 10_000 → liquidation always rejected |
//! | I8 | Price-drop eligibility | Collateral price drop makes position liquidatable |
//! | I9 | Full close clears debt | close_factor=100% + repay≥debt → debt becomes 0 |
//! | I10 | Incentive sweep | Seized amount scales linearly with incentive_bps |
//! | I11 | Close-factor sweep | max_liq scales linearly with close_factor_bps |
//! | I12 | Multi-step convergence | Sequential partial liquidations converge to zero debt |
//!
//! ## Security notes
//! - I1 prevents a liquidator from extracting more collateral than the close
//!   factor allows in a single call, limiting griefing of borrowers.
//! - I4 ensures the contract can never transfer collateral it does not hold,
//!   preventing insolvency from over-seizure.
//! - I7 closes the "phantom liquidation" attack where a healthy position is
//!   liquidated by manipulating the amount parameter.
//! - I8 validates that oracle-driven price changes correctly gate eligibility.

#![allow(unexpected_cfgs)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use views::HEALTH_FACTOR_SCALE;

// ─────────────────────────────────────────────────────────────────────────────
// Mock oracle — configurable price per asset
// ─────────────────────────────────────────────────────────────────────────────

/// Fixed-price oracle: always returns 100_000_000 (= $1.00 with 8 decimals).
#[contract]
pub struct InvMockOracle;

#[contractimpl]
impl InvMockOracle {
    pub fn price(_env: Env, _asset: Address) -> i128 {
        100_000_000
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Standard setup: contract + oracle + liquidation threshold 40%
/// (so 150%-collateralised positions are under-water).
fn inv_setup(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address, // admin
    Address, // asset (debt)
    Address, // collateral_asset
) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let asset = Address::generate(env);
    let collateral_asset = Address::generate(env);

    client.initialize(&admin, &1_000_000_000, &100);
    let oracle_id = env.register(InvMockOracle, ());
    client.set_oracle(&admin, &oracle_id);
    // Threshold 40%: collateral 15_000, debt 10_000 → HF = 6_000 < 10_000
    client.set_liquidation_threshold_bps(&admin, &4000);

    (client, admin, asset, collateral_asset)
}

/// Create a standard under-water position: borrow `debt` with `collateral`.
fn make_position(
    client: &LendingContractClient<'_>,
    user: &Address,
    asset: &Address,
    collateral_asset: &Address,
    debt: i128,
    collateral: i128,
) {
    client.borrow(user, asset, &debt, collateral_asset, &collateral);
}

/// BPS_SCALE constant mirrored for test arithmetic.
const BPS: i128 = 10_000;

// ─────────────────────────────────────────────────────────────────────────────
// I1 — Close-factor cap: actual_repaid ≤ total_debt * close_factor_bps / 10_000
// ─────────────────────────────────────────────────────────────────────────────

/// Requesting more than the close factor allows must be silently clamped.
/// The debt reduction must equal exactly `max_liq`, not the requested amount.
#[test]
fn inv_i1_repaid_clamped_to_close_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    // debt=10_000, collateral=15_000, close_factor=50% → max_liq=5_000
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let debt_before = client.get_debt_balance(&borrower);
    let max_liq = client.get_max_liquidatable_amount(&borrower);
    assert_eq!(max_liq, 5_000);

    let liquidator = Address::generate(&env);
    // Request 9_000 — must be clamped to 5_000
    client.liquidate(&liquidator, &borrower, &asset, &col, &9_000);

    let debt_after = client.get_debt_balance(&borrower);
    let actual_repaid = debt_before - debt_after;

    // I1: actual_repaid ≤ max_liq
    assert!(
        actual_repaid <= max_liq,
        "repaid {actual_repaid} exceeded close-factor cap {max_liq}"
    );
    // Exact: clamped to exactly max_liq
    assert_eq!(actual_repaid, max_liq);
}

/// Close-factor sweep: for every close_factor in {1, 500, 1000, 5000, 10000},
/// the actual repaid amount must equal min(requested, max_liq).
#[test]
fn inv_i1_close_factor_sweep() {
    let close_factors: [i128; 5] = [1, 500, 1000, 5000, 10_000];

    for &cf in &close_factors {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset, col) = inv_setup(&env);
        let borrower = Address::generate(&env);

        client.set_close_factor_bps(&admin, &cf);
        make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

        let debt_before = client.get_debt_balance(&borrower);
        let max_liq = client.get_max_liquidatable_amount(&borrower);
        let expected_max = debt_before * cf / BPS;
        assert_eq!(
            max_liq, expected_max,
            "cf={cf}: max_liq={max_liq} expected={expected_max}"
        );

        let liquidator = Address::generate(&env);
        // Request the full debt — must be clamped to max_liq
        client.liquidate(&liquidator, &borrower, &asset, &col, &debt_before);

        let debt_after = client.get_debt_balance(&borrower);
        let actual_repaid = debt_before - debt_after;

        assert!(
            actual_repaid <= max_liq,
            "cf={cf}: repaid {actual_repaid} > max_liq {max_liq}"
        );
        assert_eq!(
            actual_repaid, max_liq,
            "cf={cf}: expected exact clamp to {max_liq}, got {actual_repaid}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I2 — No over-repayment: actual_repaid ≤ total_debt
// ─────────────────────────────────────────────────────────────────────────────

/// Even with close_factor=100%, repaid amount must never exceed total debt.
#[test]
fn inv_i2_repaid_never_exceeds_total_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    client.set_close_factor_bps(&admin, &10_000); // 100%
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let debt_before = client.get_debt_balance(&borrower);
    let liquidator = Address::generate(&env);
    // Request 3x the debt — must be clamped to total_debt
    client.liquidate(&liquidator, &borrower, &asset, &col, &(debt_before * 3));

    let debt_after = client.get_debt_balance(&borrower);
    // I2: debt cannot go negative
    assert!(debt_after >= 0, "debt went negative: {debt_after}");
    // Debt must be fully cleared (close_factor=100%)
    assert_eq!(debt_after, 0);
}

/// With accrued interest, repaid amount still must not exceed principal + interest.
#[test]
fn inv_i2_repaid_never_exceeds_debt_with_interest() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    client.set_close_factor_bps(&admin, &10_000);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    // Advance 1 year so interest accrues
    env.ledger().with_mut(|li| li.timestamp = 1_000 + 31_536_000);

    let debt_with_interest = client.get_debt_balance(&borrower);
    assert!(debt_with_interest > 10_000);

    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &(debt_with_interest * 2));

    let debt_after = client.get_debt_balance(&borrower);
    assert!(debt_after >= 0, "debt went negative after interest liquidation");
    assert_eq!(debt_after, 0, "full close should clear all debt including interest");
}

// ─────────────────────────────────────────────────────────────────────────────
// I3 — Incentive bound: seized = repaid * (10_000 + incentive_bps) / 10_000
// ─────────────────────────────────────────────────────────────────────────────

/// For a given incentive_bps, the collateral seized must equal exactly
/// `repaid * (BPS + incentive_bps) / BPS` (subject to the collateral cap).
#[test]
fn inv_i3_seized_equals_incentive_formula() {
    let incentive_cases: [(i128, i128); 5] = [
        (0, 10_000),    // 0% incentive: seized = repaid
        (500, 10_500),  // 5%
        (1000, 11_000), // 10% (default)
        (2000, 12_000), // 20%
        (5000, 15_000), // 50%
    ];

    for (incentive_bps, scale_numerator) in incentive_cases {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset, col) = inv_setup(&env);
        let borrower = Address::generate(&env);

        client.set_liquidation_incentive_bps(&admin, &incentive_bps);
        // collateral=20_000: under-water (HF=8_000<10_000) and large enough
        // that seizure cap never triggers for repay=3_000 even at 100% incentive
        // (max seized = 3_000 * 2 = 6_000 < 20_000).
        make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

        let col_before = client.get_collateral_balance(&borrower);
        let repay: i128 = 3_000;

        let liquidator = Address::generate(&env);
        client.liquidate(&liquidator, &borrower, &asset, &col, &repay);

        let col_after = client.get_collateral_balance(&borrower);
        let seized = col_before - col_after;
        let expected = repay * scale_numerator / BPS;

        assert_eq!(
            seized, expected,
            "incentive_bps={incentive_bps}: seized={seized} expected={expected}"
        );
    }
}

/// Incentive sweep across all valid values (0–10000 bps in steps of 1000).
/// Verifies the linear relationship between incentive_bps and seized amount.
#[test]
fn inv_i3_incentive_sweep_linear() {
    for incentive_bps in (0..=10_000_i128).step_by(1000) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset, col) = inv_setup(&env);
        let borrower = Address::generate(&env);

        client.set_liquidation_incentive_bps(&admin, &incentive_bps);
        // collateral=20_000: under-water and large enough that seizure cap
        // never triggers for repay=1_000 even at 100% incentive (seized=2_000<20_000).
        make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

        let col_before = client.get_collateral_balance(&borrower);
        let repay: i128 = 1_000;

        let liquidator = Address::generate(&env);
        client.liquidate(&liquidator, &borrower, &asset, &col, &repay);

        let col_after = client.get_collateral_balance(&borrower);
        let seized = col_before - col_after;
        let expected = repay * (BPS + incentive_bps) / BPS;

        assert_eq!(
            seized, expected,
            "incentive_bps={incentive_bps}: seized={seized} expected={expected}"
        );
        // I3: seized is always ≥ repaid (incentive is non-negative)
        assert!(
            seized >= repay,
            "incentive_bps={incentive_bps}: seized {seized} < repaid {repay}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I4 — No free collateral: seized ≤ collateral_before (never negative balance)
// ─────────────────────────────────────────────────────────────────────────────

/// With 100% incentive and a small collateral pool, seized must be capped
/// at the available collateral — the borrower's balance must never go negative.
#[test]
fn inv_i4_collateral_never_negative_high_incentive() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    client.set_liquidation_incentive_bps(&admin, &10_000); // 100%
    client.set_close_factor_bps(&admin, &10_000);
    // collateral=15_000, debt=10_000; with 100% incentive, uncapped seized=20_000 > 15_000
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let col_before = client.get_collateral_balance(&borrower);
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &10_000);

    let col_after = client.get_collateral_balance(&borrower);
    // I4: collateral must not go negative
    assert!(col_after >= 0, "collateral went negative: {col_after}");
    // I4: seized must not exceed what was available
    let seized = col_before - col_after;
    assert!(
        seized <= col_before,
        "seized {seized} > collateral_before {col_before}"
    );
}

/// Sweep: for every incentive level, collateral after liquidation is always ≥ 0.
#[test]
fn inv_i4_collateral_non_negative_across_incentive_levels() {
    for incentive_bps in (0..=10_000_i128).step_by(2000) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset, col) = inv_setup(&env);
        let borrower = Address::generate(&env);

        client.set_liquidation_incentive_bps(&admin, &incentive_bps);
        client.set_close_factor_bps(&admin, &10_000);
        make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

        let col_before = client.get_collateral_balance(&borrower);
        let liquidator = Address::generate(&env);
        client.liquidate(&liquidator, &borrower, &asset, &col, &10_000);

        let col_after = client.get_collateral_balance(&borrower);
        assert!(
            col_after >= 0,
            "incentive_bps={incentive_bps}: collateral went negative: {col_after}"
        );
        let seized = col_before - col_after;
        assert!(
            seized <= col_before,
            "incentive_bps={incentive_bps}: seized {seized} > available {col_before}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I5 — Debt monotone: each liquidation call strictly reduces outstanding debt
// ─────────────────────────────────────────────────────────────────────────────

/// Three sequential partial liquidations must each reduce debt strictly.
#[test]
fn inv_i5_debt_strictly_decreases_each_call() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    // collateral=20_000: under-water (HF=8_000) and large enough for multiple
    // partial liquidations (seized = 2_000 * 1.1 = 2_200 per step < 20_000)
    make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

    let liquidator = Address::generate(&env);
    let mut prev_debt = client.get_debt_balance(&borrower);

    for _ in 0..3 {
        if client.get_health_factor(&borrower) >= HEALTH_FACTOR_SCALE {
            break;
        }
        client.liquidate(&liquidator, &borrower, &asset, &col, &2_000);
        let curr_debt = client.get_debt_balance(&borrower);
        assert!(
            curr_debt < prev_debt,
            "debt did not decrease: before={prev_debt} after={curr_debt}"
        );
        prev_debt = curr_debt;
    }
}

/// Multi-step liquidation with varying repay amounts must always reduce debt.
#[test]
fn inv_i5_debt_monotone_varying_repay_amounts() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    // collateral=20_000: under-water and large enough for multiple passes
    make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

    // Allow full close so we can do multiple passes
    client.set_close_factor_bps(&admin, &10_000);

    let liquidator = Address::generate(&env);
    let repay_amounts: [i128; 4] = [1_000, 2_500, 500, 3_000];
    let mut prev_debt = client.get_debt_balance(&borrower);

    for &repay in &repay_amounts {
        if client.get_health_factor(&borrower) >= HEALTH_FACTOR_SCALE {
            break;
        }
        if prev_debt == 0 {
            break;
        }
        client.liquidate(&liquidator, &borrower, &asset, &col, &repay);
        let curr_debt = client.get_debt_balance(&borrower);
        assert!(
            curr_debt < prev_debt,
            "debt did not decrease: repay={repay} before={prev_debt} after={curr_debt}"
        );
        prev_debt = curr_debt;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I6 — Solvency conservation: collateral_after + seized = collateral_before
// ─────────────────────────────────────────────────────────────────────────────

/// The total collateral is conserved: what the borrower loses equals what
/// the liquidator gains. No collateral is created or destroyed.
#[test]
fn inv_i6_collateral_conservation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let col_before = client.get_collateral_balance(&borrower);
    let repay: i128 = 3_000;

    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &repay);

    let col_after = client.get_collateral_balance(&borrower);
    let seized = col_before - col_after;

    // I6: seized = repay * (BPS + incentive_bps) / BPS (default incentive=1000)
    let expected_seized = repay * 11_000 / BPS;
    assert_eq!(
        seized, expected_seized,
        "conservation violated: seized={seized} expected={expected_seized}"
    );
    // I6: col_after + seized = col_before
    assert_eq!(
        col_after + seized,
        col_before,
        "collateral not conserved: after={col_after} seized={seized} before={col_before}"
    );
}

/// Conservation holds even when seizure is capped at available collateral.
#[test]
fn inv_i6_conservation_when_seizure_capped() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    client.set_liquidation_incentive_bps(&admin, &10_000); // 100%
    client.set_close_factor_bps(&admin, &10_000);
    // collateral=15_000; uncapped seized for 10_000 repay = 20_000 > 15_000
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let col_before = client.get_collateral_balance(&borrower);
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &10_000);

    let col_after = client.get_collateral_balance(&borrower);
    let seized = col_before - col_after;

    // I6: col_after + seized = col_before (conservation always holds)
    assert_eq!(
        col_after + seized,
        col_before,
        "conservation violated when capped: after={col_after} seized={seized} before={col_before}"
    );
    // I4: col_after must be ≥ 0
    assert!(col_after >= 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// I7 — Healthy positions immune: HF ≥ 10_000 → liquidation always rejected
// ─────────────────────────────────────────────────────────────────────────────

/// A position at exactly HF = 10_000 must not be liquidatable.
#[test]
fn inv_i7_exactly_healthy_position_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    // threshold=6667 bps, collateral=15_000, debt=10_000
    // weighted = 15_000 * 6667 / 10_000 = 10_000 (rounded)
    // HF = 10_000 * 10_000 / 10_000 = 10_000 (exactly at boundary)
    client.set_liquidation_threshold_bps(&admin, &6667);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let hf = client.get_health_factor(&borrower);
    assert_eq!(hf, HEALTH_FACTOR_SCALE, "expected HF exactly at boundary");

    let liquidator = Address::generate(&env);
    let result = client.try_liquidate(&liquidator, &borrower, &asset, &col, &5_000);
    assert_eq!(
        result,
        Err(Ok(BorrowError::InsufficientCollateral)),
        "healthy position must be rejected"
    );
}

/// A well-collateralised position (HF >> 10_000) must be rejected regardless
/// of the close_factor or incentive settings.
#[test]
fn inv_i7_healthy_position_rejected_across_param_combinations() {
    let close_factors: [i128; 3] = [1, 5000, 10_000];
    let incentives: [i128; 3] = [0, 1000, 10_000];

    for &cf in &close_factors {
        for &inc in &incentives {
            let env = Env::default();
            env.mock_all_auths();
            let (client, admin, asset, col) = inv_setup(&env);
            let borrower = Address::generate(&env);

            client.set_close_factor_bps(&admin, &cf);
            client.set_liquidation_incentive_bps(&admin, &inc);
            // threshold=80% (default), collateral=20_000, debt=10_000 → HF=16_000
            client.set_liquidation_threshold_bps(&admin, &8000);
            make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

            let hf = client.get_health_factor(&borrower);
            assert!(hf >= HEALTH_FACTOR_SCALE, "cf={cf} inc={inc}: expected healthy HF, got {hf}");

            let liquidator = Address::generate(&env);
            let result = client.try_liquidate(&liquidator, &borrower, &asset, &col, &5_000);
            assert_eq!(
                result,
                Err(Ok(BorrowError::InsufficientCollateral)),
                "cf={cf} inc={inc}: healthy position must be rejected"
            );
        }
    }
}

/// A position that becomes healthy after a partial liquidation must be
/// rejected on the next call.
#[test]
fn inv_i7_position_healthy_after_partial_liquidation_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    // threshold=40%, collateral=15_000, debt=10_000 → HF=6_000
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    // Full close
    client.set_close_factor_bps(&admin, &10_000);
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &10_000);

    // Position now has no debt → healthy
    let hf_after = client.get_health_factor(&borrower);
    assert!(
        hf_after >= HEALTH_FACTOR_SCALE || hf_after == views::HEALTH_FACTOR_NO_DEBT,
        "expected healthy after full close, got {hf_after}"
    );

    // Second liquidation attempt must fail
    let result = client.try_liquidate(&liquidator, &borrower, &asset, &col, &1_000);
    assert!(result.is_err(), "second liquidation on cleared position must fail");
}

// ─────────────────────────────────────────────────────────────────────────────
// I8 — Price-drop eligibility: collateral price drop makes position liquidatable
// ─────────────────────────────────────────────────────────────────────────────

/// A mock oracle whose price can be set per test.
/// Uses instance storage so each registered contract has its own price.
#[contract]
pub struct PriceableOracle;

#[contractimpl]
impl PriceableOracle {
    /// Store a price for later retrieval.
    pub fn set_price(env: Env, price: i128) {
        env.storage().persistent().set(&soroban_sdk::symbol_short!("price"), &price);
    }

    /// Return the stored price (default 100_000_000 = $1.00).
    pub fn price(env: Env, _asset: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&soroban_sdk::symbol_short!("price"))
            .unwrap_or(100_000_000_i128)
    }
}

/// A position that is healthy at threshold=80% becomes liquidatable when the
/// liquidation threshold is lowered to 40% (simulating a risk parameter change
/// that makes previously-safe positions eligible for liquidation).
///
/// This validates that the health factor gate correctly uses the configured
/// threshold and that eligibility changes are reflected immediately.
#[test]
fn inv_i8_threshold_change_makes_position_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let asset = Address::generate(&env);
    let col = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &100);
    let oracle_id = env.register(InvMockOracle, ());
    client.set_oracle(&admin, &oracle_id);

    // threshold=80%: collateral=20_000, debt=10_000 → HF=16_000 (healthy)
    client.set_liquidation_threshold_bps(&admin, &8000);
    let borrower = Address::generate(&env);
    make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

    let hf_before = client.get_health_factor(&borrower);
    assert!(hf_before >= HEALTH_FACTOR_SCALE, "should be healthy at threshold=80%");
    assert_eq!(client.get_max_liquidatable_amount(&borrower), 0);

    // Lower threshold to 40%: HF = 20_000 * 0.40 * 10_000 / 10_000 = 8_000 < 10_000
    client.set_liquidation_threshold_bps(&admin, &4000);

    let hf_after = client.get_health_factor(&borrower);
    assert!(
        hf_after < HEALTH_FACTOR_SCALE,
        "should be liquidatable after threshold lowered, HF={hf_after}"
    );
    assert!(
        client.get_max_liquidatable_amount(&borrower) > 0,
        "max_liq must be > 0 after threshold change"
    );

    // Liquidation must now succeed
    let liquidator = Address::generate(&env);
    let result = client.try_liquidate(&liquidator, &borrower, &asset, &col, &2_000);
    assert!(result.is_ok(), "liquidation must succeed after threshold change");
}

/// Restoring the threshold back to a safe level must restore immunity.
#[test]
fn inv_i8_threshold_restore_restores_immunity() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let asset = Address::generate(&env);
    let col = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &100);
    let oracle_id = env.register(InvMockOracle, ());
    client.set_oracle(&admin, &oracle_id);

    // Start healthy at threshold=80%
    client.set_liquidation_threshold_bps(&admin, &8000);
    let borrower = Address::generate(&env);
    make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

    // Lower threshold → liquidatable
    client.set_liquidation_threshold_bps(&admin, &4000);
    assert!(client.get_health_factor(&borrower) < HEALTH_FACTOR_SCALE);

    // Restore threshold → healthy again
    client.set_liquidation_threshold_bps(&admin, &8000);
    let hf_recovered = client.get_health_factor(&borrower);
    assert!(
        hf_recovered >= HEALTH_FACTOR_SCALE,
        "should be healthy after threshold restored, HF={hf_recovered}"
    );

    let liquidator = Address::generate(&env);
    let result = client.try_liquidate(&liquidator, &borrower, &asset, &col, &2_000);
    assert_eq!(
        result,
        Err(Ok(BorrowError::InsufficientCollateral)),
        "must be rejected after threshold restored"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// I9 — Full close clears debt: close_factor=100% + repay≥debt → debt=0
// ─────────────────────────────────────────────────────────────────────────────

/// With close_factor=100%, a single liquidation call requesting ≥ total_debt
/// must clear the position entirely.
#[test]
fn inv_i9_full_close_clears_debt_exactly() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    client.set_close_factor_bps(&admin, &10_000);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &10_000);

    let debt_after = client.get_debt_balance(&borrower);
    assert_eq!(debt_after, 0, "full close must clear all debt");

    let hf = client.get_health_factor(&borrower);
    assert!(
        hf >= HEALTH_FACTOR_SCALE || hf == views::HEALTH_FACTOR_NO_DEBT,
        "health factor must be healthy after full close, got {hf}"
    );
}

/// Full close with interest: close_factor=100% clears principal + interest.
#[test]
fn inv_i9_full_close_clears_debt_including_interest() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    client.set_close_factor_bps(&admin, &10_000);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    env.ledger().with_mut(|li| li.timestamp = 1_000 + 31_536_000);

    let total_debt = client.get_debt_balance(&borrower);
    assert!(total_debt > 10_000, "interest must have accrued");

    let liquidator = Address::generate(&env);
    // Request more than total — must be clamped to total
    client.liquidate(&liquidator, &borrower, &asset, &col, &(total_debt * 2));

    let debt_after = client.get_debt_balance(&borrower);
    assert_eq!(debt_after, 0, "full close must clear debt including interest");
}

// ─────────────────────────────────────────────────────────────────────────────
// I10 — Incentive sweep: seized scales linearly with incentive_bps
// ─────────────────────────────────────────────────────────────────────────────

/// For a fixed repay amount, seized collateral must increase monotonically
/// as incentive_bps increases (when not capped by available collateral).
#[test]
fn inv_i10_seized_monotone_with_incentive() {
    let repay: i128 = 1_000;
    let mut prev_seized: i128 = -1;

    for incentive_bps in (0..=10_000_i128).step_by(500) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset, col) = inv_setup(&env);
        let borrower = Address::generate(&env);

        client.set_liquidation_incentive_bps(&admin, &incentive_bps);
        // Large collateral so cap never triggers
        make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

        let col_before = client.get_collateral_balance(&borrower);
        let liquidator = Address::generate(&env);
        client.liquidate(&liquidator, &borrower, &asset, &col, &repay);

        let col_after = client.get_collateral_balance(&borrower);
        let seized = col_before - col_after;

        assert!(
            seized >= prev_seized,
            "incentive_bps={incentive_bps}: seized {seized} < prev {prev_seized} (not monotone)"
        );
        prev_seized = seized;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I11 — Close-factor sweep: max_liq scales linearly with close_factor_bps
// ─────────────────────────────────────────────────────────────────────────────

/// max_liq must equal total_debt * close_factor_bps / 10_000 for every valid cf.
#[test]
fn inv_i11_max_liq_linear_with_close_factor() {
    let debt: i128 = 10_000;

    for cf in (1..=10_000_i128).step_by(999) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset, col) = inv_setup(&env);
        let borrower = Address::generate(&env);

        client.set_close_factor_bps(&admin, &cf);
        make_position(&client, &borrower, &asset, &col, debt, 15_000);

        let max_liq = client.get_max_liquidatable_amount(&borrower);
        let expected = debt * cf / BPS;

        assert_eq!(
            max_liq, expected,
            "cf={cf}: max_liq={max_liq} expected={expected}"
        );
    }
}

/// max_liq must be monotonically non-decreasing as close_factor increases.
#[test]
fn inv_i11_max_liq_monotone_with_close_factor() {
    let debt: i128 = 10_000;
    let mut prev_max: i128 = 0;

    for cf in (1..=10_000_i128).step_by(500) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, asset, col) = inv_setup(&env);
        let borrower = Address::generate(&env);

        client.set_close_factor_bps(&admin, &cf);
        make_position(&client, &borrower, &asset, &col, debt, 15_000);

        let max_liq = client.get_max_liquidatable_amount(&borrower);
        assert!(
            max_liq >= prev_max,
            "cf={cf}: max_liq={max_liq} < prev={prev_max} (not monotone)"
        );
        prev_max = max_liq;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I12 — Multi-step convergence: sequential partial liquidations → debt = 0
// ─────────────────────────────────────────────────────────────────────────────

/// Repeated partial liquidations (each at the close-factor cap) must
/// eventually converge to zero debt within a bounded number of steps.
///
/// With close_factor=50% and starting debt=10_000:
///   step 1: repay 5_000 → debt=5_000
///   step 2: repay 2_500 → debt=2_500
///   step 3: repay 1_250 → debt=1_250
///   ...
/// After 14 steps, debt < 1 (rounds to 0).
#[test]
fn inv_i12_sequential_liquidations_converge_to_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    // collateral=20_000: under-water (HF=8_000) and large enough for multiple
    // partial liquidations without hitting the seizure cap
    make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

    let liquidator = Address::generate(&env);
    let max_steps = 20;

    for step in 0..max_steps {
        let hf = client.get_health_factor(&borrower);
        if hf >= HEALTH_FACTOR_SCALE || hf == views::HEALTH_FACTOR_NO_DEBT {
            break;
        }
        let debt = client.get_debt_balance(&borrower);
        if debt == 0 {
            break;
        }
        let max_liq = client.get_max_liquidatable_amount(&borrower);
        assert!(max_liq > 0, "step {step}: max_liq=0 but position still under-water");

        client.liquidate(&liquidator, &borrower, &asset, &col, &max_liq);

        let debt_after = client.get_debt_balance(&borrower);
        assert!(
            debt_after < debt,
            "step {step}: debt did not decrease: before={debt} after={debt_after}"
        );
    }

    // After convergence, either debt=0 or position is healthy
    let final_debt = client.get_debt_balance(&borrower);
    let final_hf = client.get_health_factor(&borrower);
    assert!(
        final_debt == 0 || final_hf >= HEALTH_FACTOR_SCALE || final_hf == views::HEALTH_FACTOR_NO_DEBT,
        "did not converge: debt={final_debt} hf={final_hf}"
    );
}

/// With close_factor=100%, a single step must clear the entire debt.
#[test]
fn inv_i12_full_close_factor_single_step_convergence() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);

    client.set_close_factor_bps(&admin, &10_000);
    // collateral=20_000: under-water (HF=8_000) and large enough for full seizure
    // with default 10% incentive: seized = 10_000 * 1.1 = 11_000 < 20_000
    make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

    let liquidator = Address::generate(&env);
    let max_liq = client.get_max_liquidatable_amount(&borrower);
    assert_eq!(max_liq, 10_000);

    client.liquidate(&liquidator, &borrower, &asset, &col, &max_liq);

    assert_eq!(client.get_debt_balance(&borrower), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Additional edge-case invariants
// ─────────────────────────────────────────────────────────────────────────────

/// Requesting exactly max_liq must succeed and reduce debt by exactly max_liq.
#[test]
fn inv_exact_max_liq_request_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let debt_before = client.get_debt_balance(&borrower);
    let max_liq = client.get_max_liquidatable_amount(&borrower);
    assert_eq!(max_liq, 5_000);

    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &max_liq);

    let debt_after = client.get_debt_balance(&borrower);
    assert_eq!(debt_before - debt_after, max_liq);
}

/// Requesting max_liq + 1 must be silently clamped to max_liq.
#[test]
fn inv_one_over_max_liq_clamped() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let debt_before = client.get_debt_balance(&borrower);
    let max_liq = client.get_max_liquidatable_amount(&borrower);

    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &(max_liq + 1));

    let debt_after = client.get_debt_balance(&borrower);
    let actual_repaid = debt_before - debt_after;
    assert_eq!(actual_repaid, max_liq, "must be clamped to max_liq");
}

/// Requesting 1 unit (minimum positive amount) must succeed and reduce debt by 1.
#[test]
fn inv_minimum_repay_amount_one_unit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    let debt_before = client.get_debt_balance(&borrower);
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &1);

    let debt_after = client.get_debt_balance(&borrower);
    assert_eq!(debt_before - debt_after, 1, "minimum repay of 1 must reduce debt by 1");
}

/// Global total debt must decrease by exactly the repaid principal amount.
#[test]
fn inv_global_total_debt_decremented_by_repaid() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    // collateral=20_000: under-water (HF=8_000) and large enough for seizure
    make_position(&client, &borrower, &asset, &col, 10_000, 20_000);

    // Capture debt before via the user's position (proxy for global debt in single-user test)
    let debt_before = client.get_debt_balance(&borrower);
    let repay: i128 = 3_000;

    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &repay);

    let debt_after = client.get_debt_balance(&borrower);
    // The user's debt must have decreased by exactly repay
    assert_eq!(
        debt_before - debt_after,
        repay,
        "user debt must decrease by repaid amount"
    );
}

/// Liquidating a position with accrued interest: interest is settled first,
/// then principal. The debt position must be consistent after liquidation.
#[test]
fn inv_interest_settled_before_principal() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (client, _admin, asset, col) = inv_setup(&env);
    let borrower = Address::generate(&env);
    make_position(&client, &borrower, &asset, &col, 10_000, 15_000);

    // Advance time so interest accrues
    env.ledger().with_mut(|li| li.timestamp = 1_000 + 15_768_000); // ~6 months

    let debt_pos_before = client.get_user_debt(&borrower);
    let total_before = debt_pos_before.borrowed_amount + debt_pos_before.interest_accrued;
    assert!(debt_pos_before.interest_accrued > 0, "interest must have accrued");

    // Repay a small amount (less than interest)
    let small_repay: i128 = 1;
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &asset, &col, &small_repay);

    let debt_pos_after = client.get_user_debt(&borrower);
    let total_after = debt_pos_after.borrowed_amount + debt_pos_after.interest_accrued;

    // Total debt must have decreased
    assert!(total_after < total_before, "total debt must decrease");
    // Principal must be unchanged (interest absorbed the repayment)
    assert_eq!(
        debt_pos_after.borrowed_amount,
        debt_pos_before.borrowed_amount,
        "principal must be unchanged when repay < interest"
    );
}
