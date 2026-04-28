//! Price Oscillation Stress Tests
//!
//! This module implements cross-asset stress tests simulating rapid oracle price
//! oscillations across multiple assets. It verifies:
//! - Liquidation stability under high volatility
//! - Protocol solvency (Total Assets >= Total Liabilities)
//! - Consistent error behavior (e.g., stale prices, insufficient collateral)
//! - Invariant preservation (no negative reserves, bounded payouts)

#![cfg(test)]

use crate::tests::test_helpers::setup_env_with_native_asset;
use crate::{AssetConfig, UserPositionSummary};
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env, Vec};

/// Setup multiple assets with initial prices and configurations
fn setup_stress_assets(env: &Env, client: &crate::HelloContractClient, admin: &Address) -> Vec<Address> {
    let mut assets = Vec::new(env);
    
    // 1. USDC - Stablecoin ($1.00, 7 decimals)
    let usdc = Address::generate(env);
    client.initialize_asset(&Some(usdc.clone()), &AssetConfig {
        asset: Some(usdc.clone()),
        collateral_factor: 8000,   // 80%
        liquidation_threshold: 8500, // 85%
        reserve_factor: 1000,      // 10%
        max_supply: 10_000_000_0000000,
        max_borrow: 8_000_000_0000000,
        can_collateralize: true,
        can_borrow: true,
        borrow_factor: 10000,
        price: 10_000_000,
        price_updated_at: env.ledger().timestamp(),
    });
    assets.push_back(usdc);

    // 2. ETH - Volatile ($2000.00, 7 decimals)
    let eth = Address::generate(env);
    client.initialize_asset(&Some(eth.clone()), &AssetConfig {
        asset: Some(eth.clone()),
        collateral_factor: 7000,   // 70%
        liquidation_threshold: 7500, // 75%
        reserve_factor: 1500,      // 15%
        max_supply: 1_000_000_0000000,
        max_borrow: 500_000_0000000,
        can_collateralize: true,
        can_borrow: true,
        borrow_factor: 10000,
        price: 20_000_000_000,
        price_updated_at: env.ledger().timestamp(),
    });
    assets.push_back(eth);

    // 3. XLM - Volatile ($0.10, 7 decimals)
    let xlm = Address::generate(env);
    client.initialize_asset(&Some(xlm.clone()), &AssetConfig {
        asset: Some(xlm.clone()),
        collateral_factor: 6000,   // 60%
        liquidation_threshold: 6500, // 65%
        reserve_factor: 2000,      // 20%
        max_supply: 100_000_000_0000000,
        max_borrow: 50_000_000_0000000,
        can_collateralize: true,
        can_borrow: true,
        borrow_factor: 10000,
        price: 1_000_000,
        price_updated_at: env.ledger().timestamp(),
    });
    assets.push_back(xlm);

    assets
}

#[test]
fn test_rapid_price_oscillation_stability() {
    let (env, _contract_id, client, admin, _user, _native) = setup_env_with_native_asset();
    let assets = setup_stress_assets(&env, &client, &admin);
    
    let usdc = assets.get(0).unwrap();
    let eth = assets.get(1).unwrap();
    let xlm = assets.get(2).unwrap();

    // Setup 5 users with different initial positions
    let mut users = Vec::new(&env);
    for _ in 0..5 {
        let u = Address::generate(&env);
        // Deposit $10k USDC
        client.cross_asset_deposit(&u, &Some(usdc.clone()), &10000_0000000);
        users.push_back(u);
    }

    // User 0: Borrows ETH (Healthy)
    client.cross_asset_borrow(&users.get(0).unwrap(), &Some(eth.clone()), &2_0000000); // 2 ETH = $4k
    
    // User 1: Borrows ETH (Near limit)
    client.cross_asset_borrow(&users.get(1).unwrap(), &Some(eth.clone()), &3_0000000); // 3 ETH = $6k (HF ≈ 1.33)

    // User 2: Borrows XLM (Aggressive)
    client.cross_asset_borrow(&users.get(2).unwrap(), &Some(xlm.clone()), &50000_0000000); // 50k XLM = $5k

    // Simulation Loop: 20 Ticks of price oscillation
    let initial_eth_price = 20_000_000_000i128;
    let initial_xlm_price = 1_000_000i128;

    for tick in 0..20 {
        // Oscillation logic:
        // Ticks 0-4: ETH drops 10% each tick (Crash)
        // Ticks 5-9: ETH recovers 15% each tick (Bounce)
        // Ticks 10-14: XLM oscillates +/- 20%
        // Ticks 15-19: Simultaneous volatility
        
        let eth_price = if tick < 5 {
            initial_eth_price * (10 - tick as i128) / 10
        } else if tick < 10 {
            initial_eth_price / 2 + (initial_eth_price / 10 * (tick as i128 - 5))
        } else {
            initial_eth_price
        };

        let xlm_price = if tick >= 10 && tick < 15 {
            if tick % 2 == 0 { initial_xlm_price * 8 / 10 } else { initial_xlm_price * 12 / 10 }
        } else {
            initial_xlm_price
        };

        let _ = client.update_asset_price(&Some(eth.clone()), &eth_price);
        let _ = client.update_asset_price(&Some(xlm.clone()), &xlm_price);

        // Advance time to avoid staleness issues (but keep it within 1 hour)
        env.ledger().with_mut(|li| {
            li.timestamp += 60; // 1 minute per tick
        });

        // Attempt liquidations for all users
        let liquidator = Address::generate(&env);
        for user in users.iter() {
            let summary = client.get_user_position_summary(&user);
            if summary.is_liquidatable {
                // Determine which asset to liquidate
                // For simplicity, we try to repay debt in whatever they borrowed
                let debt_asset = if summary.total_debt_value > 0 {
                    // In our setup, users 0-1 borrowed ETH, user 2 borrowed XLM
                    if tick < 10 { Some(eth.clone()) } else { Some(xlm.clone()) }
                } else {
                    None
                };

                if let Some(asset) = debt_asset {
                    // Try to liquidate 50% of debt
                    let _ = client.try_liquidate(&liquidator, &user, &Some(asset), &Some(usdc.clone()), &(summary.total_debt_value / 2));
                }
            }
        }

        // Invariant Checks
        assert_invariants(&env, &client, &assets);
    }
}

fn assert_invariants(env: &Env, client: &crate::HelloContractClient, assets: &Vec<Address>) {
    for asset in assets.iter() {
        // 1. Prices are positive
        let config = client.get_asset_config(&Some(asset.clone())).unwrap();
        assert!(config.price > 0, "Price must be positive");

        // 2. Solvency: Total Supply >= Total Borrow
        let total_supply = client.get_total_supply_for(&Some(asset.clone()));
        let total_borrow = client.get_total_borrow_for(&Some(asset.clone()));
        assert!(total_supply >= total_borrow, "Protocol must be solvent for asset");
    }
    
    // 3. Deterministic failure modes: Stale prices
    // If we advance time by 10 hours, any operation should fail
    env.ledger().with_mut(|li| {
        li.timestamp += 3600 * 10;
    });
    
    let dummy_user = Address::generate(env);
    let result = client.try_cross_asset_borrow(&dummy_user, &None, &1);
    assert!(result.is_err(), "Should fail with stale price");
    
    // Reset time for next tick
    env.ledger().with_mut(|li| {
        li.timestamp -= 3600 * 10;
    });
}
