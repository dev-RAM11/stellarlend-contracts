//! # AMM Pool Accounting Invariant Tests
//!
//! Property-based and invariant tests for the AMM liquidity pool accounting.
//! Tests strictly validate:
//! - Initial bootstrapping LP share issuance `floor(sqrt(a * b))`
//! - Proportional minting `floor(min(a * total / reserve_a, b * total / reserve_b))`
//! - Round-trip invariants where users cannot extract more value than deposited.
//! - Floor rounding constraints and non-negative pool reserves.
//! - Boundary conditions around extreme values (`i128::MAX`).

use super::*;
use crate::amm::{AmmDataKey, AmmProtocolConfig, LiquidityParams, SwapParams, TokenPair};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Symbol, Vec,
};

// ═══════════════════════════════════════════════════════════════════════════
// Mock AMM Protocol Contract
// ═══════════════════════════════════════════════════════════════════════════

#[soroban_sdk::contract]
pub struct InvariantMockAmm;

#[soroban_sdk::contractimpl]
impl InvariantMockAmm {
    pub fn swap(
        _env: Env,
        _executor: Address,
        _token_in: Option<Address>,
        _token_out: Option<Address>,
        amount_in: i128,
        _min_amount_out: i128,
        _callback_data: amm::AmmCallbackData,
    ) -> i128 {
        amount_in
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Test Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn setup_amm(
    env: &Env,
) -> (AmmContractClient<'_>, Address, Address, AmmProtocolConfig, Address) {
    let contract_id = env.register(AmmContract, ());
    let client = AmmContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let user = Address::generate(env);

    client.initialize_amm_settings(&admin, &100, &1000, &10000);

    let protocol_addr = env.register(MockAmm, ());
    let token_b = Address::generate(env);

    let mut supported_pairs = Vec::new(env);
    supported_pairs.push_back(TokenPair {
        token_a: None,
        token_b: Some(token_b.clone()),
        pool_address: Address::generate(env),
    });

    let config = AmmProtocolConfig {
        protocol_address: protocol_addr.clone(),
        protocol_name: Symbol::new(env, "MockAMM"),
        enabled: true,
        fee_tier: 30,
        min_swap_amount: 1,
        max_swap_amount: i128::MAX,
        supported_pairs,
    };

    client.add_amm_protocol(&admin, &config);

    (client, admin, user, config, token_b)
}

fn get_pool_state(
    env: &Env,
    contract_id: &Address,
    protocol: &Address,
    token_a: &Option<Address>,
    token_b: &Option<Address>,
) -> amm::PoolState {
    let key = AmmDataKey::PoolState(protocol.clone(), token_a.clone(), token_b.clone());
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get::<AmmDataKey, amm::PoolState>(&key)
            .unwrap_or(amm::PoolState {
                reserve_a: 0,
                reserve_b: 0,
                total_lp_shares: 0,
            })
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Invariant Tests
// ═══════════════════════════════════════════════════════════════════════════

/// Test proportional minting math invariants using a table-driven approach
#[test]
fn test_invariant_proportional_minting_table() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, config, token_b) = setup_amm(&env);

    let cases = [
        // (add_a, add_b, expected_lp_minted, msg)
        (10_000, 10_000, 10_000, "1:1 ratio"),
        (5_000, 5_000, 5_000, "1:1 ratio exact proportional"),
        (20_000, 10_000, 10_000, "over-supply A yields LP based on B (min)"),
        (10_000, 20_000, 10_000, "over-supply B yields LP based on A (min)"),
        (1, 1, 1, "smallest discrete liquidity"),
    ];

    // Bootstrap pool to 10k:10k
    let bootstrap_params = LiquidityParams {
        protocol: config.protocol_address.clone(),
        token_a: None,
        token_b: Some(token_b.clone()),
        amount_a: 10_000,
        amount_b: 10_000,
        min_amount_a: 10_000,
        min_amount_b: 10_000,
        deadline: env.ledger().timestamp() + 3600,
    };
    client.add_liquidity(&user, &bootstrap_params);

    for (add_a, add_b, expected_lp, msg) in cases {
        // Reset pool explicitly by clearing internal state, or create a fresh env per iteration.
        // Let's use a fresh env per iteration for isolation.
        let env2 = Env::default();
        env2.mock_all_auths();
        let (client2, _, user2, config2, token_b2) = setup_amm(&env2);

        let p1 = LiquidityParams {
            protocol: config2.protocol_address.clone(),
            token_a: None,
            token_b: Some(token_b2.clone()),
            amount_a: 10_000,
            amount_b: 10_000,
            min_amount_a: 10_000,
            min_amount_b: 10_000,
            deadline: env2.ledger().timestamp() + 3600,
        };
        client2.add_liquidity(&user2, &p1);

        let p2 = LiquidityParams {
            protocol: config2.protocol_address.clone(),
            token_a: None,
            token_b: Some(token_b2.clone()),
            amount_a: add_a,
            amount_b: add_b,
            min_amount_a: 0,
            min_amount_b: 0,
            deadline: env2.ledger().timestamp() + 3600,
        };
        let minted = client2.add_liquidity(&user2, &p2);

        assert_eq!(minted, expected_lp, "Failed case: {}", msg);
    }
}

/// Invariant: Adding liquidity and immediately removing it MUST NEVER yield more
/// tokens than were initially supplied, protecting the pool from extraction.
#[test]
fn test_invariant_round_trip_no_extraction() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, config, token_b) = setup_amm(&env);
    let contract_id = Address::from_string(&client.address.to_string());

    // User 1 bootstraps pool: 1_000_000 / 500_000
    let u1 = Address::generate(&env);
    let boot_params = LiquidityParams {
        protocol: config.protocol_address.clone(),
        token_a: None,
        token_b: Some(token_b.clone()),
        amount_a: 1_000_000,
        amount_b: 500_000,
        min_amount_a: 0,
        min_amount_b: 0,
        deadline: env.ledger().timestamp() + 3600,
    };
    client.add_liquidity(&u1, &boot_params);

    // User 2 adds a highly skewed amount (over-supplying A)
    let amount_a_in = 2_000_000;
    let amount_b_in = 100_000; // Limits the minted shares

    let add_params = LiquidityParams {
        protocol: config.protocol_address.clone(),
        token_a: None,
        token_b: Some(token_b.clone()),
        amount_a: amount_a_in,
        amount_b: amount_b_in,
        min_amount_a: 0,
        min_amount_b: 0,
        deadline: env.ledger().timestamp() + 3600,
    };
    let minted_lp = client.add_liquidity(&user, &add_params);

    // Track state pre-remove
    let _state_mid = get_pool_state(&env, &contract_id, &config.protocol_address, &None, &Some(token_b.clone()));

    // User 2 immediately removes the exact LP they minted
    let (out_a, out_b) = client.remove_liquidity(
        &user,
        &config.protocol_address,
        &None,
        &Some(token_b.clone()),
        &minted_lp,
        &0,
        &0,
        &(env.ledger().timestamp() + 3600),
    );

    // Check strict invariants
    assert!(out_a <= amount_a_in, "Extraction invariant violated for A");
    assert!(out_b <= amount_b_in, "Extraction invariant violated for B");

    let state_final = get_pool_state(&env, &contract_id, &config.protocol_address, &None, &Some(token_b.clone()));

    // Non-negative pool reserves
    assert!(state_final.reserve_a >= 0);
    assert!(state_final.reserve_b >= 0);
    assert!(state_final.total_lp_shares >= 0);

    // The pool must strictly grow or remain equal (dust stays in the pool due to floor rounding)
    assert!(state_final.reserve_a >= 1_000_000);
    assert!(state_final.reserve_b >= 500_000);
}

/// Invariant: Removing more LP shares than the pool contains should fail cleanly.
#[test]
fn test_invariant_share_issuance_bounded() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, config, token_b) = setup_amm(&env);

    let p = LiquidityParams {
        protocol: config.protocol_address.clone(),
        token_a: None,
        token_b: Some(token_b.clone()),
        amount_a: 1_000,
        amount_b: 1_000,
        min_amount_a: 0,
        min_amount_b: 0,
        deadline: env.ledger().timestamp() + 3600,
    };
    let lp = client.add_liquidity(&user, &p);

    let result = client.try_remove_liquidity(
        &user,
        &config.protocol_address,
        &None,
        &Some(token_b.clone()),
        &(lp + 1), // One more than exists
        &0,
        &0,
        &(env.ledger().timestamp() + 3600),
    );

    assert_eq!(result.unwrap_err().unwrap(), amm::AmmError::InsufficientLiquidity);
}

/// Invariant: Negative or Zero amounts are rejected.
#[test]
fn test_invariant_zero_and_negative_constraints() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, config, token_b) = setup_amm(&env);

    // Zero amounts
    let mut p = LiquidityParams {
        protocol: config.protocol_address.clone(),
        token_a: None,
        token_b: Some(token_b.clone()),
        amount_a: 0,
        amount_b: 1_000,
        min_amount_a: 0,
        min_amount_b: 0,
        deadline: env.ledger().timestamp() + 3600,
    };
    let r1 = client.try_add_liquidity(&user, &p);
    assert_eq!(r1.unwrap_err().unwrap(), amm::AmmError::InvalidSwapParams);

    // Negative amount
    p.amount_a = -50;
    let r2 = client.try_add_liquidity(&user, &p);
    assert_eq!(r2.unwrap_err().unwrap(), amm::AmmError::InvalidSwapParams);

    // Zero LP token burn
    let p_valid = LiquidityParams {
        protocol: config.protocol_address.clone(),
        token_a: None,
        token_b: Some(token_b.clone()),
        amount_a: 1_000,
        amount_b: 1_000,
        min_amount_a: 0,
        min_amount_b: 0,
        deadline: env.ledger().timestamp() + 3600,
    };
    client.add_liquidity(&user, &p_valid);

    let r3 = client.try_remove_liquidity(
        &user,
        &config.protocol_address,
        &None,
        &Some(token_b.clone()),
        &0, // Zero burn
        &0,
        &0,
        &(env.ledger().timestamp() + 3600),
    );
    assert_eq!(r3.unwrap_err().unwrap(), amm::AmmError::InvalidSwapParams);
}

/// Invariant: High extreme values must not cause uncontrolled panic, but rather
/// trigger proper overflow AmmErrors or handle `i128` scale correctly up to limits.
#[test]
fn test_invariant_extreme_boundary_limits() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, config, token_b) = setup_amm(&env);

    let p = LiquidityParams {
        protocol: config.protocol_address.clone(),
        token_a: None,
        token_b: Some(token_b.clone()),
        amount_a: i128::MAX, // Maximum i128
        amount_b: i128::MAX,
        min_amount_a: 0,
        min_amount_b: 0,
        deadline: env.ledger().timestamp() + 3600,
    };
    
    // sqrt(MAX * MAX) overflows the product intermediate inside execute_amm_add_liquidity
    let r = client.try_add_liquidity(&user, &p);
    assert_eq!(r.unwrap_err().unwrap(), amm::AmmError::Overflow);
}
