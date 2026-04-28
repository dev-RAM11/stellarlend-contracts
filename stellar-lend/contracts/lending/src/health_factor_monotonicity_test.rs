//! Tests for health factor monotonicity.
//! Ensures that repaying debt or adding collateral always improves or maintains the health factor.
//! Covers multiple assets, price changes, and rounding edge cases.

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use views::{HEALTH_FACTOR_NO_DEBT, HEALTH_FACTOR_SCALE};

/// Setup environment with lending contract, admin, user, and two assets.
fn setup(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address,
    Address,
    Address,
    Address,
) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    let collateral_asset = Address::generate(env);
    
    // Initialize with standard settings
    client.initialize(&admin, &1_000_000_000, &10);
    client.initialize_deposit_settings(&1_000_000_000, &10);
    client.initialize_withdraw_settings(&10);
    
    (client, admin, user, asset, collateral_asset)
}

#[test]
fn test_health_factor_monotonicity_repay() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup(&env);
    
    // Set prices (1.0 with 8 decimals)
    client.update_price_feed(&admin, &asset, &100_000_000);
    client.update_price_feed(&admin, &collateral_asset, &100_000_000);
    client.set_liquidation_threshold_bps(&admin, &8000);

    // Initial position: $100 collateral, $50 debt
    client.borrow(&user, &asset, &50_000, &collateral_asset, &100_000);
    let hf_initial = client.get_health_factor(&user);
    // Expected: (100k * 0.8) / 50k * 10000 = 16000
    assert_eq!(hf_initial, 16000);

    // Repay some debt -> HF must increase
    client.repay(&user, &asset, &10_000);
    let hf_after_repay = client.get_health_factor(&user);
    // Expected: (100k * 0.8) / 40k * 10000 = 20000
    assert!(hf_after_repay > hf_initial);
    assert_eq!(hf_after_repay, 20000);

    // Repay almost all
    client.repay(&user, &asset, &39_000);
    let hf_near_zero = client.get_health_factor(&user);
    assert!(hf_near_zero > hf_after_repay);

    // Full repay -> Returns sentinel HEALTH_FACTOR_NO_DEBT
    client.repay(&user, &asset, &1_000);
    let hf_final = client.get_health_factor(&user);
    assert_eq!(hf_final, HEALTH_FACTOR_NO_DEBT);
}

#[test]
fn test_health_factor_monotonicity_deposit_collateral() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup(&env);
    
    client.update_price_feed(&admin, &asset, &100_000_000);
    client.update_price_feed(&admin, &collateral_asset, &100_000_000);
    client.set_liquidation_threshold_bps(&admin, &8000);

    // Initial position
    client.borrow(&user, &asset, &50_000, &collateral_asset, &100_000);
    let hf_initial = client.get_health_factor(&user);

    // Add collateral -> HF must increase
    client.deposit_collateral(&user, &collateral_asset, &50_000);
    let hf_after_deposit = client.get_health_factor(&user);
    // Expected: (150k * 0.8) / 50k * 10000 = 24000
    assert!(hf_after_deposit > hf_initial);
    assert_eq!(hf_after_deposit, 24000);
}

#[test]
fn test_health_factor_monotonicity_multiple_assets_price_change() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset_debt, asset_collateral) = setup(&env);
    
    // Initial: debt asset $2.0, collateral asset $1.0
    client.update_price_feed(&admin, &asset_debt, &200_000_000);
    client.update_price_feed(&admin, &asset_collateral, &100_000_000);
    client.set_liquidation_threshold_bps(&admin, &8000);

    // Debt 100 units ($200), collateral 400 units ($400)
    client.borrow(&user, &asset_debt, &100, &asset_collateral, &400);
    let hf_initial = client.get_health_factor(&user);
    // Expected: (400 * 1.0 * 0.8) / (100 * 2.0) * 10000 = 16000
    assert_eq!(hf_initial, 16000);

    // Collateral price goes up -> HF must increase
    client.update_price_feed(&admin, &asset_collateral, &150_000_000); // $1.5
    let hf_collat_up = client.get_health_factor(&user);
    // Expected: (400 * 1.5 * 0.8) / 200 * 10000 = 24000
    assert!(hf_collat_up > hf_initial);
    assert_eq!(hf_collat_up, 24000);

    // Debt price goes down -> HF must increase
    client.update_price_feed(&admin, &asset_debt, &100_000_000); // $1.0
    let hf_debt_down = client.get_health_factor(&user);
    // Expected: (400 * 1.5 * 0.8) / (100 * 1.0) * 10000 = 48000
    assert!(hf_debt_down > hf_collat_up);
    assert_eq!(hf_debt_down, 48000);
}

#[test]
fn test_health_factor_rounding_stability() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset) = setup(&env);
    
    // Set prices with minor deviations to test rounding sensitivity
    client.update_price_feed(&admin, &asset, &100_000_001); 
    client.update_price_feed(&admin, &collateral_asset, &99_999_999);
    client.set_liquidation_threshold_bps(&admin, &8000);

    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    let hf1 = client.get_health_factor(&user);

    // Repay small amount (min 10 in setup)
    client.repay(&user, &asset, &10);
    let hf2 = client.get_health_factor(&user);
    // Non-decreasing: hf2 >= hf1
    assert!(hf2 >= hf1, "HF decreased after repay: {} < {}", hf2, hf1);

    // Add small collateral (min 10 in setup)
    client.deposit_collateral(&user, &collateral_asset, &10);
    let hf3 = client.get_health_factor(&user);
    // Non-decreasing: hf3 >= hf2
    assert!(hf3 >= hf2, "HF decreased after collateral add: {} < {}", hf3, hf2);
}

// Documentation on rounding behavior:
// - `collateral_value` rounds down (amount * price / 1e8).
// - `debt_value` rounds down (debt * price / 1e8).
// - `compute_health_factor` rounds down (weighted_collateral * 10000 / debt_value).
//
// While rounding debt down slightly inflates the health factor, 
// the logic ensures monotonicity because adding collateral or repaying debt 
// always moves the numerator up or denominator down in the HF formula.
// The tests above verify that even with rounding, the HF never degrades 
// during these positive user actions.
