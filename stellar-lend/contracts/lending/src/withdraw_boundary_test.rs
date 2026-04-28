//! # Withdraw Boundary and Collateral Ratio Tests
//!
//! This test suite validates withdrawal constraints at the edge of collateralization
//! boundaries, ensuring the protocol remains solvent and prevents undercollateralized
//! positions under various market conditions (price moves, multi-asset portfolios).

extern crate std;
use soroban_sdk::{testutils::Address as _, Address, Env};
use crate::cross_asset::AssetParams;
use crate::oracle::OracleConfig;
use crate::constants::HEALTH_FACTOR_SCALE;
use crate::{LendingContract, LendingContractClient};

/// Helper to set up a test environment with a contract and admin.
fn setup_env() -> (Env, LendingContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    
    // Initialize modules with high limits
    client.initialize(&admin, &1_000_000_000_000, &100);
    client.initialize_admin(&admin);
    
    // Configure oracle
    client.configure_oracle(&admin, &OracleConfig {
        max_staleness_seconds: 3600,
    });
    
    (env, client, admin)
}

/// Helper to configure an asset and set its price.
fn setup_asset(env: &Env, client: &LendingContractClient, admin: &Address, asset: &Address, ltv: i128, price: i128) {
    client.set_asset_params(&asset, &AssetParams {
        ltv,
        liquidation_threshold: ltv + 500, // 5% buffer
        price_feed: Address::generate(env), // Dummy feed addr
        debt_ceiling: 1_000_000_000_000, // High debt ceiling ($100k)
        is_active: true,
    });
    
    // Submit price to oracle
    client.update_price_feed(admin, asset, &price);
}

// 7 decimals for prices and USD values
const USD_DECIMALS: i128 = 10_000_000;

#[test]
fn test_withdraw_boundary_single_asset() {
    let (env, client, admin) = setup_env();
    let user = Address::generate(&env);
    let usdc = env.register_stellar_asset_contract(admin.clone());
    
    // $1.00 USDC, 80% LTV
    setup_asset(&env, &client, &admin, &usdc, 8000, USD_DECIMALS);
    
    // Deposit $100 USDC (100 units)
    client.deposit_collateral_asset(&user, &usdc, &(100 * USD_DECIMALS));
    
    // Borrow $80 USDC (80 units)
    client.borrow_asset(&user, &usdc, &(80 * USD_DECIMALS));
    
    // Current state: Weighted Collateral = $80, Debt = $80. HF = 1.0 (10000).
    let summary = client.get_cross_position_summary(&user);
    assert_eq!(summary.health_factor, HEALTH_FACTOR_SCALE);
    
    // 1. Withdrawal fails
    assert!(client.try_withdraw_asset(&user, &usdc, &1).is_err());
    
    // 2. Repay $1.
    client.repay_asset(&user, &usdc, &USD_DECIMALS);
    
    // Fund contract
    let usdc_token = soroban_sdk::token::StellarAssetClient::new(&env, &usdc);
    usdc_token.mint(&client.address, &(100 * USD_DECIMALS));
    
    client.withdraw_asset(&user, &usdc, &USD_DECIMALS); // Withdraw 1 unit ($1)
    
    let summary2 = client.get_cross_position_summary(&user);
    assert!(summary2.health_factor >= HEALTH_FACTOR_SCALE);
}

#[test]
fn test_withdraw_boundary_price_drop() {
    let (env, client, admin) = setup_env();
    let user = Address::generate(&env);
    let eth = env.register_stellar_asset_contract(admin.clone());
    let usdc = env.register_stellar_asset_contract(admin.clone());

    // $2000 ETH, 80% LTV
    setup_asset(&env, &client, &admin, &eth, 8000, 2000 * USD_DECIMALS);
    setup_asset(&env, &client, &admin, &usdc, 9000, 1 * USD_DECIMALS);
    
    // Deposit 1 ETH ($2000). Weighted = $1600.
    client.deposit_collateral_asset(&user, &eth, &USD_DECIMALS);
    
    // Borrow $1500 USD (1500 units of USDC)
    client.borrow_asset(&user, &usdc, &(1500 * USD_DECIMALS));
    
    // HF = 1600 / 1500 = 1.0666...
    let summary = client.get_cross_position_summary(&user);
    assert!(summary.health_factor > HEALTH_FACTOR_SCALE);
    
    // Price drops to $1875. Weighted = 1875 * 0.8 = $1500.
    client.update_price_feed(&admin, &eth, &(1875 * USD_DECIMALS));
    
    let summary2 = client.get_cross_position_summary(&user);
    assert_eq!(summary2.health_factor, HEALTH_FACTOR_SCALE);
    
    // Withdrawal fails
    assert!(client.try_withdraw_asset(&user, &eth, &1).is_err());
}

#[test]
fn test_withdraw_boundary_multi_asset_portfolio() {
    let (env, client, admin) = setup_env();
    let user = Address::generate(&env);
    let btc = env.register_stellar_asset_contract(admin.clone());
    let eth = env.register_stellar_asset_contract(admin.clone());
    let usdc = env.register_stellar_asset_contract(admin.clone());
    
    // BTC: $50,000 (80% LTV), ETH: $2,000 (70% LTV)
    setup_asset(&env, &client, &admin, &btc, 8000, 50000 * USD_DECIMALS);
    setup_asset(&env, &client, &admin, &eth, 7000, 2000 * USD_DECIMALS);
    setup_asset(&env, &client, &admin, &usdc, 9000, 1 * USD_DECIMALS);
    
    // Deposit 0.1 BTC ($5000) + 5 ETH ($10,000)
    // Weighted = (5000 * 0.8) + (10000 * 0.7) = 4000 + 7000 = $11,000
    client.deposit_collateral_asset(&user, &btc, &(USD_DECIMALS / 10)); // 0.1 units
    client.deposit_collateral_asset(&user, &eth, &(5 * USD_DECIMALS)); // 5 units
    
    // Borrow $11,000
    client.borrow_asset(&user, &usdc, &(11000 * USD_DECIMALS));
    
    // HF = 1.0
    assert_eq!(client.get_cross_position_summary(&user).health_factor, HEALTH_FACTOR_SCALE);
    
    // Withdrawal fails
    assert!(client.try_withdraw_asset(&user, &btc, &1).is_err());
    
    // Repay $1000. Debt = $10,000.
    client.repay_asset(&user, &usdc, &(1000 * USD_DECIMALS));
    
    // Fund contract
    let eth_token = soroban_sdk::token::StellarAssetClient::new(&env, &eth);
    eth_token.mint(&client.address, &(5 * USD_DECIMALS));

    // Max ETH withdrawal 'w' such that: (11000 - 0.7*w*2000) >= 10000 
    // Weighted reduction = 1000. 0.7 * w * 2000 = 1000 => 1400w = 1000 => w = 1000/1400 = 0.714... units.
    client.withdraw_asset(&user, &eth, &(USD_DECIMALS / 2)); // Withdraw 0.5 units ($1000). Weighted reduction = $700.
    
    assert!(client.get_cross_position_summary(&user).health_factor >= HEALTH_FACTOR_SCALE);
}

#[test]
fn test_withdraw_boundary_rounding_precision() {
    let (env, client, admin) = setup_env();
    let user = Address::generate(&env);
    let usdc = env.register_stellar_asset_contract(admin.clone());
    
    setup_asset(&env, &client, &admin, &usdc, 8000, USD_DECIMALS);
    
    // Small deposit 1.0000001 units
    client.deposit_collateral_asset(&user, &usdc, &(USD_DECIMALS + 1));
    
    // Max borrow = (USD_DECIMALS + 1) * 0.8 = 8,000,000.8 -> rounds down to 8,000,000
    client.borrow_asset(&user, &usdc, &(8 * 1_000_000));
    
    // Fund contract
    let usdc_token = soroban_sdk::token::StellarAssetClient::new(&env, &usdc);
    usdc_token.mint(&client.address, &(USD_DECIMALS + 1));

    // Try to withdraw the "rounding dust" (1 unit)
    client.withdraw_asset(&user, &usdc, &1); // Should succeed at the exact boundary
    
    assert_eq!(client.get_cross_position_summary(&user).health_factor, HEALTH_FACTOR_SCALE);
    
    // Now any further withdrawal fails
    assert!(client.try_withdraw_asset(&user, &usdc, &1).is_err());
}
