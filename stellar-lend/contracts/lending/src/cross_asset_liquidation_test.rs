//! # Cross-Asset Liquidation Scenario Tests
//!
//! Validates liquidation correctness when the debt asset and collateral asset
//! are **different tokens with different oracle prices**.
//!
//! ## Why Cross-Asset Pricing Matters
//!
//! Liquidation health factor, seizure amount, and close-factor enforcement all
//! depend on price values derived from two independent oracle reads:
//!
//! ```text
//! collateral_value = collateral_amount * price(collateral) / PRICE_SCALE
//! debt_value       = total_debt        * price(debt_asset) / PRICE_SCALE
//! health_factor    = (collateral_value * liq_threshold_bps / 10_000)
//!                    * HF_SCALE / debt_value
//! seized           = min(repay * (10_000 + incentive) / 10_000, collateral_balance)
//! ```
//!
//! When the two prices differ the dollar-denominated health factor diverges from
//! the raw-unit ratio, which is the exact regime missing from existing tests.
//!
//! ## Oracle Requirements for Cross-Asset Liquidations
//!
//! - **Both** the debt asset and the collateral asset must have fresh oracle prices.
//! - Staleness in either feed returns HF = 0, blocking liquidation.
//! - The collateral seizure formula operates in raw units (not dollars), so the
//!   collateral:debt price ratio does **not** directly scale the seized amount;
//!   it only affects the HF eligibility check.
//!
//! ## Scenarios Covered
//!
//! | # | Scenario |
//! |---|----------|
//! | 1 | Collateral (ETH) more expensive than debt (USDC) — healthy initially, liquidatable after price drop |
//! | 2 | Debt more expensive than collateral — position immediately liquidatable |
//! | 3 | Collateral price crashes to near zero — seizure capped at balance |
//! | 4 | Partial liquidation with cross prices — close factor enforced |
//! | 5 | Debt price spikes — position flips from healthy to liquidatable |
//! | 6 | Full liquidation across assets — HF → NO_DEBT after full repay |
//! | 7 | Missing oracle for debt asset — liquidation safely rejected |
//! | 8 | Missing oracle for collateral asset — liquidation safely rejected |
//! | 9 | Sequential cross-asset partial liquidations converge |
//! | 10| Post-liquidation HF is monotonically non-decreasing for partial repay |

use super::*;
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, Address, Env,
};
use views::HEALTH_FACTOR_SCALE;

// ─────────────────────────────────────────────────────────────────────────────
// Mock oracle — returns per-asset price stored in instance storage
// ─────────────────────────────────────────────────────────────────────────────

#[contract]
pub struct CrossPriceOracle;

#[contractimpl]
impl CrossPriceOracle {
    /// Return per-asset price (8-decimal). Returns 0 if not set.
    pub fn price(env: Env, asset: Address) -> i128 {
        env.storage()
            .instance()
            .get::<Address, i128>(&asset)
            .unwrap_or(0)
    }
}

/// Write a per-asset price directly into the oracle's instance storage.
fn set_price(env: &Env, oracle: &Address, asset: &Address, price: i128) {
    env.as_contract(oracle, || {
        env.storage().instance().set(asset, &price);
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Price constants (8-decimal format, 1e8 = $1.00)
// ─────────────────────────────────────────────────────────────────────────────

const PRICE_1_USD: i128 = 100_000_000; // $1.00
const PRICE_2_USD: i128 = 200_000_000; // $2.00
const PRICE_5_USD: i128 = 500_000_000; // $5.00
const PRICE_10_USD: i128 = 1_000_000_000; // $10.00
const PRICE_05_USD: i128 = 50_000_000;  // $0.50
const PRICE_001_USD: i128 = 100_000;    // $0.001 (near-zero crash)

// ─────────────────────────────────────────────────────────────────────────────
// Setup helper
// ─────────────────────────────────────────────────────────────────────────────

/// Initialises the lending contract with:
/// - Admin + oracle configured
/// - Liquidation threshold = 8000 bps (80%)
/// - Close factor = 5000 bps (50% — default)
/// - Liquidation incentive = 1000 bps (10% — default)
///
/// Returns `(client, admin, borrower, oracle_id, debt_asset, collateral_asset)`.
fn setup_cross(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address, // admin
    Address, // borrower
    Address, // oracle
    Address, // debt_asset
    Address, // collateral_asset
) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let oracle_id = env.register(CrossPriceOracle, ());
    let debt_asset = Address::generate(env);
    let collateral_asset = Address::generate(env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_oracle(&admin, &oracle_id);
    // 80% threshold — a 2:1 collateral:debt ratio is healthy; 1.2:1 is not.
    client.set_liquidation_threshold_bps(&admin, &8000);

    (client, admin, borrower, oracle_id, debt_asset, collateral_asset)
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: Collateral more expensive than debt — healthy initially
// ─────────────────────────────────────────────────────────────────────────────

/// ETH ($10) as collateral, USDC ($1) as debt.
/// 2000 USDC debt ($2_000) vs 3000 ETH collateral ($30_000).
/// Raw 3000 >= 2000 * 1.5 = 3000 → passes collateral ratio check.
/// HF = (30_000 * 0.80) * 10_000 / 2_000 = 120_000 → healthy.
/// After ETH crashes to $0.50, dollar CV = $1_500 < debt $2_000 → liquidatable.
#[test]
fn test_cross_collateral_expensive_price_drop_triggers_liquidation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // ETH = $10, USDC = $1
    set_price(&env, &oracle, &debt_asset, PRICE_1_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_10_USD);

    // 2_000 USDC debt, 3_000 ETH raw collateral (raw 3000 >= 2000*1.5=3000 ✓)
    client.borrow(&borrower, &debt_asset, &2_000, &collateral_asset, &3_000);

    let hf_before = client.get_health_factor(&borrower);
    assert!(
        hf_before >= HEALTH_FACTOR_SCALE,
        "should be healthy: ETH col value $30_000 >> debt $2_000; got hf={hf_before}"
    );

    // ETH crashes to $0.50 → col value = 3_000 * 0.50 = $1_500; debt = $2_000
    set_price(&env, &oracle, &collateral_asset, PRICE_05_USD);

    let hf_after = client.get_health_factor(&borrower);
    assert!(
        hf_after < HEALTH_FACTOR_SCALE,
        "should be liquidatable after ETH price crash: col $1_500 * 0.80 < debt $2_000; got hf={hf_after}"
    );
    assert!(hf_after > 0, "HF must be computable with valid prices");

    // Liquidation should now succeed (50% close factor cap: max 1_000)
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &1_000);

    let remaining = client.get_user_debt(&borrower);
    assert!(
        remaining.borrowed_amount < 2_000,
        "debt must reduce after cross-asset liquidation"
    );
    let _ = admin;
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: Debt more expensive than collateral — position is immediately liquidatable
// ─────────────────────────────────────────────────────────────────────────────

/// Collateral ($1) is cheaper than debt ($5).
/// 3_000 USDC collateral ($3_000 value) vs 1_000 debt-token ($5_000 value)
/// Raw: 3_000 >= 1_000 * 1.5 = 1_500 ✓. Min borrow: 1_000 ✓.
/// HF = (3_000 * 0.80) * 10_000 / 5_000 = 4_800 < 10_000 → immediately liquidatable.
#[test]
fn test_cross_debt_expensive_position_immediately_liquidatable() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Debt at $5, collateral at $1
    set_price(&env, &oracle, &debt_asset, PRICE_5_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_1_USD);

    // 1_000 debt-token ($5_000 value), 3_000 collateral ($3_000 value)
    // Raw 3_000 >= 1_000 * 1.5 = 1_500 ✓
    // HF = (3_000 * 0.80) * 10_000 / 5_000 = 4_800 < 10_000 → immediately liquidatable
    client.borrow(&borrower, &debt_asset, &1_000, &collateral_asset, &3_000);

    let hf = client.get_health_factor(&borrower);
    assert!(
        hf < HEALTH_FACTOR_SCALE,
        "should be under-collateralised: $3_000 col at 80% vs $5_000 debt => HF=4_800; got {hf}"
    );
    assert!(hf > 0, "HF must be computable with valid oracle prices");

    // Partial liquidation — 50% close factor caps at 500
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &500);

    let pos = client.get_user_position(&borrower);
    assert!(pos.debt_balance < 1_000, "debt must be reduced");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: Collateral price crash to near-zero — seizure capped at balance
// ─────────────────────────────────────────────────────────────────────────────

/// Collateral crashes to $0.001. Incentive-scaled seized amount would exceed
/// collateral balance. Protocol must cap at actual balance.
/// Uses raw amounts: debt=5_000, collateral=10_000 (raw 10_000 >= 5_000*1.5=7_500 ✓).
#[test]
fn test_cross_collateral_crash_seizure_capped_at_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Both at $2 to start — healthy
    set_price(&env, &oracle, &debt_asset, PRICE_1_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_2_USD);

    // 5_000 debt, 10_000 collateral: raw 10_000 >= 5_000*1.5=7_500 ✓
    // Dollar value: col=$20_000 vs debt=$5_000 → HF=(20_000*0.80)*10_000/5_000 = 32_000 → healthy
    client.borrow(&borrower, &debt_asset, &5_000, &collateral_asset, &10_000);

    // Collateral crashes to near-zero: $0.001
    set_price(&env, &oracle, &collateral_asset, PRICE_001_USD);

    let hf = client.get_health_factor(&borrower);
    assert!(hf < HEALTH_FACTOR_SCALE, "should be liquidatable after crash");
    assert!(hf > 0);

    // 100% incentive + 100% close factor → uncapped seize = 10_000, but only 10_000 available
    client.set_liquidation_incentive_bps(&admin, &10_000); // 100%
    client.set_close_factor_bps(&admin, &10_000);          // 100%

    let collateral_before = client.get_collateral_balance(&borrower);
    let liquidator = Address::generate(&env);
    client.liquidate(
        &liquidator,
        &borrower,
        &debt_asset,
        &collateral_asset,
        &6_000, // request more than owed — clamped by close factor to 5_000
    );

    let collateral_after = client.get_collateral_balance(&borrower);
    let seized = collateral_before - collateral_after;

    // Seizure must not exceed what was available
    assert!(seized <= collateral_before, "seized must never exceed available collateral");
    assert!(collateral_after >= 0, "collateral must not go negative");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: Close factor enforced with cross-asset prices
// ─────────────────────────────────────────────────────────────────────────────

/// With a 2× price difference, the close factor (50%) must cap the repayable
/// amount to 50% of total_debt regardless of asset denomination.
#[test]
fn test_cross_close_factor_enforced_with_price_difference() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Debt at $2, collateral at $1 — 10_000 debt units ($20_000) vs 30_000 col units ($30_000)
    // HF = (30_000 * 0.80) * 10_000 / 20_000 = 12_000 → healthy
    set_price(&env, &oracle, &debt_asset, PRICE_2_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_1_USD);
    client.borrow(&borrower, &debt_asset, &10_000, &collateral_asset, &30_000);

    let hf_before = client.get_health_factor(&borrower);
    assert!(hf_before >= HEALTH_FACTOR_SCALE, "should start healthy");

    // Debt price spikes to $5 — HF = (30_000*0.80)*10_000 / 50_000 = 4_800 → liquidatable
    set_price(&env, &oracle, &debt_asset, PRICE_5_USD);
    let hf_after = client.get_health_factor(&borrower);
    assert!(hf_after < HEALTH_FACTOR_SCALE);
    assert!(hf_after > 0);

    // Default 50% close factor: max_liq = 10_000 * 5000 / 10_000 = 5_000
    let max_liq = client.get_max_liquidatable_amount(&borrower);
    assert_eq!(max_liq, 5_000, "close factor should cap at 5_000");

    let liquidator = Address::generate(&env);
    // Request 8_000 — must be clamped to 5_000
    client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &8_000);

    let debt_after = client.get_user_debt(&borrower);
    assert_eq!(
        debt_after.borrowed_amount, 5_000,
        "debt reduced by exactly the close-factor-capped amount"
    );

    // Set liquidation threshold for the admin reference
    let _ = admin;
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 5: Debt price spike flips healthy position to liquidatable
// ─────────────────────────────────────────────────────────────────────────────

/// Debt oracle price doubles mid-lifecycle. Position that was healthy becomes
/// liquidatable. Tests that price-driven HF flip is detected correctly.
#[test]
fn test_cross_debt_price_spike_triggers_liquidation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Both at $1 — open with 20_000 collateral vs 10_000 debt → HF = 1.60
    set_price(&env, &oracle, &debt_asset, PRICE_1_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_1_USD);
    client.borrow(&borrower, &debt_asset, &10_000, &collateral_asset, &20_000);

    let hf_before = client.get_health_factor(&borrower);
    assert!(hf_before >= HEALTH_FACTOR_SCALE, "should be healthy at 1:1 prices");

    // Debt price doubles to $2 → HF = (20_000 * 0.80) * 10_000 / 20_000 = 8_000
    set_price(&env, &oracle, &debt_asset, PRICE_2_USD);
    let hf_mid = client.get_health_factor(&borrower);
    assert!(
        hf_mid < HEALTH_FACTOR_SCALE,
        "should be liquidatable after debt price spike: HF≈8_000"
    );

    // Confirm liquidation proceeds
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &5_000);

    let hf_final = client.get_health_factor(&borrower);
    assert!(hf_final >= hf_mid || hf_final == 0, "HF must not worsen after partial liquidation");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 6: Full liquidation across assets — HF → NO_DEBT
// ─────────────────────────────────────────────────────────────────────────────

/// Full repay across assets (100% close factor) clears debt completely and
/// returns HEALTH_FACTOR_NO_DEBT.
#[test]
fn test_cross_full_liquidation_clears_debt_hf_no_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    set_price(&env, &oracle, &debt_asset, PRICE_2_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_1_USD);

    // 5_000 debt ($10_000 value) vs 20_000 collateral ($20_000 value) → HF = 1.6 healthy
    client.borrow(&borrower, &debt_asset, &5_000, &collateral_asset, &20_000);
    assert!(client.get_health_factor(&borrower) >= HEALTH_FACTOR_SCALE);

    // Debt price spikes to $10 → $50_000 debt value vs $20_000 collateral → HF = 0.32
    set_price(&env, &oracle, &debt_asset, PRICE_10_USD);
    assert!(client.get_health_factor(&borrower) < HEALTH_FACTOR_SCALE);

    // Allow full liquidation
    client.set_close_factor_bps(&admin, &10_000);

    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &5_001);

    let debt_after = client.get_user_debt(&borrower);
    assert_eq!(debt_after.borrowed_amount, 0, "all debt should be cleared");
    assert_eq!(debt_after.interest_accrued, 0);

    let hf_final = client.get_health_factor(&borrower);
    assert_eq!(
        hf_final,
        views::HEALTH_FACTOR_NO_DEBT,
        "HF should be NO_DEBT sentinel after full repay"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 7: Missing oracle for debt asset — liquidation rejected
// ─────────────────────────────────────────────────────────────────────────────

/// If no price is available for the debt asset, the health factor cannot be
/// computed and liquidation must be safely rejected.
#[test]
fn test_cross_no_oracle_debt_asset_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Only set collateral price — debt price is missing
    set_price(&env, &oracle, &collateral_asset, PRICE_10_USD);

    // Borrow without debt oracle — state is accepted (no oracle check at borrow)
    client.borrow(&borrower, &debt_asset, &10_000, &collateral_asset, &15_000);

    // HF = 0 (debt price unavailable)
    assert_eq!(client.get_health_factor(&borrower), 0);

    // Liquidation must be rejected (HF==0 → treated as healthy/unknown)
    let liquidator = Address::generate(&env);
    let result = client.try_liquidate(
        &liquidator,
        &borrower,
        &debt_asset,
        &collateral_asset,
        &5_000,
    );
    assert_eq!(
        result,
        Err(Ok(BorrowError::InsufficientCollateral)),
        "must reject when debt oracle is absent"
    );
    let _ = admin;
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 8: Missing oracle for collateral asset — liquidation rejected
// ─────────────────────────────────────────────────────────────────────────────

/// If collateral price is absent, HF = 0 → liquidation rejected.
#[test]
fn test_cross_no_oracle_collateral_asset_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Only set debt price — collateral price is missing
    set_price(&env, &oracle, &debt_asset, PRICE_1_USD);

    client.borrow(&borrower, &debt_asset, &10_000, &collateral_asset, &15_000);

    // HF = 0 (collateral price unavailable)
    assert_eq!(client.get_health_factor(&borrower), 0);

    let liquidator = Address::generate(&env);
    let result = client.try_liquidate(
        &liquidator,
        &borrower,
        &debt_asset,
        &collateral_asset,
        &5_000,
    );
    assert_eq!(
        result,
        Err(Ok(BorrowError::InsufficientCollateral)),
        "must reject when collateral oracle is absent"
    );
    let _ = admin;
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 9: Sequential cross-asset partial liquidations converge
// ─────────────────────────────────────────────────────────────────────────────

/// Two partial liquidations in sequence must each reduce debt, with the second
/// call either further reducing debt or being rejected (position recovered).
#[test]
fn test_cross_sequential_partial_liquidations_converge() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Debt at $2, collateral at $1 — 10_000 debt ($20_000) vs 20_000 col ($20_000)
    // HF = 20_000 * 0.80 * 10_000 / 20_000 = 8_000 → immediately liquidatable
    set_price(&env, &oracle, &debt_asset, PRICE_2_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_1_USD);
    client.borrow(&borrower, &debt_asset, &10_000, &collateral_asset, &20_000);

    let hf_initial = client.get_health_factor(&borrower);
    assert!(hf_initial < HEALTH_FACTOR_SCALE && hf_initial > 0);

    let liquidator = Address::generate(&env);

    // First partial liquidation (50% close factor capped at 5_000)
    client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &6_000);
    let debt1 = client.get_user_debt(&borrower);
    let remaining1 = debt1.borrowed_amount + debt1.interest_accrued;
    assert!(remaining1 < 10_000, "first liquidation must reduce debt");

    let hf_mid = client.get_health_factor(&borrower);

    // If still liquidatable, a second call should further reduce debt
    if hf_mid < HEALTH_FACTOR_SCALE && hf_mid > 0 {
        client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &6_000);
        let debt2 = client.get_user_debt(&borrower);
        let remaining2 = debt2.borrowed_amount + debt2.interest_accrued;
        assert!(remaining2 < remaining1, "second liquidation must further reduce debt");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 10: Post-liquidation HF monotonically non-decreasing for partial repay
// ─────────────────────────────────────────────────────────────────────────────

/// After a partial cross-asset liquidation, the health factor must either
/// improve or stay the same (never worsen). This validates that the combined
/// effect of debt reduction and collateral seizure preserves or improves health.
#[test]
fn test_cross_partial_liquidation_hf_monotonically_nondecreasing() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, borrower, oracle, debt_asset, collateral_asset) = setup_cross(&env);

    // Collateral at $0.50, debt at $1 — position liquidatable from start
    // 20_000 collateral ($10_000) vs 10_000 debt ($10_000)
    // HF = 10_000 * 0.80 * 10_000 / 10_000 = 8_000 → liquidatable
    set_price(&env, &oracle, &debt_asset, PRICE_1_USD);
    set_price(&env, &oracle, &collateral_asset, PRICE_05_USD);
    client.borrow(&borrower, &debt_asset, &10_000, &collateral_asset, &20_000);

    let hf_before = client.get_health_factor(&borrower);
    assert!(
        hf_before < HEALTH_FACTOR_SCALE && hf_before > 0,
        "HF must be liquidatable: {hf_before}"
    );

    // Small partial repay well within close factor
    let liquidator = Address::generate(&env);
    client.liquidate(&liquidator, &borrower, &debt_asset, &collateral_asset, &2_000);

    let hf_after = client.get_health_factor(&borrower);

    // After partial liquidation the HF must not worsen
    assert!(
        hf_after >= hf_before || hf_after == 0,
        "HF must not worsen after partial cross-asset liquidation: before={hf_before}, after={hf_after}"
    );
}
