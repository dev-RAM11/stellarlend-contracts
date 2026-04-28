//! AMM Liquidation Logic Tests
//!
//! This module validates the liquidation flow when integrated with an AMM.
//! It ensures that collateral is correctly seized, swapped via AMM, and 
//! the resulting debt asset is transferred back to the liquidator.

#![cfg(test)]

use crate::deposit::{DepositDataKey, Position};
use crate::{HelloContract, HelloContractClient, AmmProtocolConfig, TokenPair, SwapParams};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Symbol, Vec,
};

/// Creates a test environment with all auths mocked
fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    // Set a non-zero timestamp
    env.ledger().with_mut(|li| li.timestamp = 1000);
    env
}

/// Sets up admin and initializes the contract
fn setup_contract(env: &Env) -> (Address, Address, HelloContractClient<'_>) {
    let contract_id = env.register(HelloContract, ());
    let client = HelloContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (contract_id, admin, client)
}

fn setup_amm(env: &Env, client: &HelloContractClient<'_>, admin: &Address) -> Address {
    let protocol_addr = Address::generate(env);
    client.initialize_amm(admin, &100, &1000, &1000000);
    
    let mut supported_pairs = Vec::new(env);
    // We'll use this AMM for the liquidation swap
    supported_pairs.push_back(TokenPair {
        token_a: None, // Will be collateral
        token_b: None, // Will be debt
        pool_address: Address::generate(env),
    });

    let protocol_config = AmmProtocolConfig {
        protocol_address: protocol_addr.clone(),
        protocol_name: Symbol::new(env, "TestAMM"),
        enabled: true,
        fee_tier: 30,
        min_swap_amount: 1,
        max_swap_amount: 1_000_000_000,
        supported_pairs,
    };
    client.set_amm_pool(admin, &protocol_config);
    protocol_addr
}

#[test]
fn test_liquidate_with_amm_success() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract(&env);
    let amm_protocol = setup_amm(&env, &client, &admin);

    let borrower = Address::generate(&env);
    let liquidator = Address::generate(&env);
    
    // Using None for assets to use native XLM (simplified)
    let debt_asset = None;
    let collateral_asset = None;

    // Setup prices: 1:1
    // Native XLM address is usually mock-handled or set
    let native_addr = Address::generate(&env);
    client.set_native_asset_address(&admin, &native_addr);
    client.update_price_feed(&admin, &native_addr, &100_000_000, &7, &admin);

    // Setup risk params: 50% close factor, 10% incentive
    client.set_risk_params(&admin, &Some(5000), &None, &None, &Some(1000));

    // Create undercollateralized position
    // Debt: 100, Collateral: 100 (Ratio 100%, liquidatable)
    env.as_contract(&contract_id, || {
        let pos_key = DepositDataKey::Position(borrower.clone());
        env.storage().persistent().set(&pos_key, &Position {
            collateral: 100,
            debt: 100,
            borrow_interest: 0,
            last_accrual_time: env.ledger().timestamp(),
        });
        let col_key = DepositDataKey::CollateralBalance(borrower.clone());
        env.storage().persistent().set(&col_key, &100i128);
    });

    // ACTION: Liquidate 50 units of debt with AMM swap
    // EXPECTATION:
    // 1. Debt liquidated: 50
    // 2. Collateral seized: 50 * 1.1 = 55
    // 3. AMM Swap: 55 units in -> ~54 units out (1% mock slippage)
    // 4. Liquidator receives ~54 units of debt asset
    
    let repaid = client.liquidate_with_amm(
        &liquidator,
        &borrower,
        &debt_asset,
        &collateral_asset,
        &50,
        &amm_protocol,
        &50, // min_amount_out - set to 50, but mock returns 1% less than 55 = 54.45
        &100, // 1% slippage
        &(env.ledger().timestamp() + 3600)
    );

    assert_eq!(repaid, 50);

    // Verify storage
    env.as_contract(&contract_id, || {
        let pos_key = DepositDataKey::Position(borrower.clone());
        let pos: Position = env.storage().persistent().get(&pos_key).unwrap();
        assert_eq!(pos.debt, 50);
        assert_eq!(pos.collateral, 45); // 100 - 55
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")] // AmmSwapFailed
fn test_liquidate_with_amm_slippage_fails() {
    let env = create_test_env();
    let (contract_id, admin, client) = setup_contract(&env);
    let amm_protocol = setup_amm(&env, &client, &admin);

    let borrower = Address::generate(&env);
    let liquidator = Address::generate(&env);
    
    let debt_asset = None;
    let collateral_asset = None;

    let native_addr = Address::generate(&env);
    client.set_native_asset_address(&admin, &native_addr);
    client.update_price_feed(&admin, &native_addr, &100_000_000, &7, &admin);
    client.set_risk_params(&admin, &Some(5000), &None, &None, &Some(1000));

    env.as_contract(&contract_id, || {
        let pos_key = DepositDataKey::Position(borrower.clone());
        env.storage().persistent().set(&pos_key, &Position {
            collateral: 100,
            debt: 100,
            borrow_interest: 0,
            last_accrual_time: env.ledger().timestamp(),
        });
        let col_key = DepositDataKey::CollateralBalance(borrower.clone());
        env.storage().persistent().set(&col_key, &100i128);
    });

    // ACTION: Liquidate with high min_amount_out
    // Seized: 55. Mock Swap out: 54.45 (rounded to 54)
    // If we request min_amount_out = 55, it should fail.
    client.liquidate_with_amm(
        &liquidator,
        &borrower,
        &debt_asset,
        &collateral_asset,
        &50,
        &amm_protocol,
        &55, // Impossible min_amount_out
        &100,
        &(env.ledger().timestamp() + 3600)
    );
}
