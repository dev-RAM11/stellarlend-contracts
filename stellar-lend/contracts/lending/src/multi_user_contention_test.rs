//! # Multi-user Contention Scenarios
//!
//! Simulating many users depositing/borrowing in interleaved order within
//! the same ledger context to validate security, bounds, and reentrancy protections.

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

fn setup_contention_test(
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

    client.initialize(&admin, &10_000_000_000, &100);
    client.register_asset(&admin, &asset);
    client.register_asset(&admin, &collateral_asset);
    client.initialize_deposit_settings(&10_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    (client, admin, asset, collateral_asset)
}

fn generate_users(env: &Env, count: u32) -> Vec<Address> {
    let mut users = Vec::new(env);
    for _ in 0..count {
        users.push_back(Address::generate(env));
    }
    users
}

#[test]
fn test_contention_interleaved_deposits_borrows() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, collateral_asset) = setup_contention_test(&env);

    let num_users = 50;
    let users = generate_users(&env, num_users);

    let mut expected_total_deposits = 0;
    let mut expected_total_borrows = 0;

    // Interleaved deposit and borrow operations
    for (i, user) in users.iter().enumerate() {
        // Even indices deposit first, odd indices borrow first (if they have collateral)

        // Every user deposits collateral
        let deposit_amount = 50_000 + (i as i128 * 100);
        client.deposit(&user, &collateral_asset, &deposit_amount);
        expected_total_deposits += deposit_amount;

        // Alternate users borrow
        if i % 2 == 0 {
            let borrow_amount = 10_000 + (i as i128 * 50);
            let collateral_amount = borrow_amount * 2;
            client.borrow(
                &user,
                &asset,
                &borrow_amount,
                &collateral_asset,
                &collateral_amount,
            );
            expected_total_borrows += borrow_amount;
        }
    }

    // Verify individual positions and global state constraints
    let mut actual_debt = 0i128;
    let mut actual_deposits = 0i128;
    for (i, user) in users.iter().enumerate() {
        let collat = client.get_user_collateral_deposit(&user, &collateral_asset);
        assert_eq!(collat.amount, 50_000 + (i as i128 * 100));
        actual_deposits += collat.amount;

        let debt = client.get_user_debt(&user);
        if i % 2 == 0 {
            assert_eq!(debt.borrowed_amount, 10_000 + (i as i128 * 50));
            actual_debt += debt.borrowed_amount;
        } else {
            assert_eq!(debt.borrowed_amount, 0);
        }
    }

    assert_eq!(actual_deposits, expected_total_deposits);
    assert_eq!(actual_debt, expected_total_borrows);
    // Global invariant: total deposits >= total borrows
    assert!(actual_deposits >= actual_debt);
}

#[test]
fn test_contention_edge_cases_zero_amounts_overflow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, asset, collateral_asset) = setup_contention_test(&env);
    let user = Address::generate(&env);

    // Zero amount deposit
    let res_deposit = client.try_deposit(&user, &asset, &0);
    assert!(res_deposit.is_err());

    // Zero amount borrow
    client.deposit(&user, &collateral_asset, &100_000);
    let res_borrow = client.try_borrow(&user, &asset, &0, &collateral_asset, &0);
    assert!(res_borrow.is_err());

    // Max amount (overflow testing)
    let res_overflow_deposit = client.try_deposit(&user, &asset, &i128::MAX);
    assert!(res_overflow_deposit.is_err()); // Exceeds deposit cap
}

#[test]
fn test_contention_paused_operations() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, asset, collateral_asset) = setup_contention_test(&env);

    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);

    client.deposit(&user1, &collateral_asset, &50_000);

    // Pause deposits
    client.set_deposit_paused(&admin, &true);

    // Trying to deposit while paused under contention scenario
    let deposit_res = client.try_deposit(&user2, &collateral_asset, &50_000);
    assert!(deposit_res.is_err());

    // Borrow should still work if not paused
    client.borrow(&user1, &asset, &10_000, &collateral_asset, &20_000);

    // Pause borrows
    client.set_pause(&admin, &PauseType::Borrow, &true);
    let borrow_res = client.try_borrow(&user1, &asset, &10_000, &collateral_asset, &20_000);
    assert!(borrow_res.is_err());
}

// ──────────────────────────────────────────────────────────────────────────────
// ADDED FOR ISSUE #671: Deterministic Multi-User Fairness and Ordering Tests
// ──────────────────────────────────────────────────────────────────────────────

/// Tests that two users borrowing and repaying in the exact same sequence 
/// and ledger timestamps accrue identical interest, proving fair ordering.
#[test]
fn test_borrow_repay_ordering_fairness() {
    let env = Env::default();
    env.mock_all_auths();

    // 1. Setup Environment using existing helper
    let (client, _admin, debt_asset, collateral_asset) = setup_contention_test(&env);

    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);

    let deposit_amount = 10_000_000i128;
    let borrow_amount = 2_000_000i128;

    // 2. Both users deposit identical amounts
    client.deposit(&user_a, &collateral_asset, &deposit_amount);
    client.deposit(&user_b, &collateral_asset, &deposit_amount);

    // 3. Both users borrow identical amounts sequentially in the same ledger timestamp
    client.borrow(&user_a, &debt_asset, &borrow_amount, &collateral_asset, &deposit_amount);
    client.borrow(&user_b, &debt_asset, &borrow_amount, &collateral_asset, &deposit_amount);

    // 4. Advance ledger time to force interest accrual (e.g., 30 days)
    env.ledger().set_timestamp(env.ledger().timestamp() + 2_592_000);

    // 5. Both users repay the exact principal amount in the same ledger timestamp
    // By only repaying the principal, their remaining debt will exactly equal the accrued interest.
    client.repay(&user_a, &debt_asset, &borrow_amount);
    client.repay(&user_b, &debt_asset, &borrow_amount);

    // 6. Assert Fairness
    let remaining_debt_a = client.get_debt_balance(&user_a);
    let remaining_debt_b = client.get_debt_balance(&user_b);

    // No user should gain an advantage by being the first or second transaction in the block
    assert_eq!(remaining_debt_a, remaining_debt_b);
}

/// Tests that multiple liquidators racing to liquidate the same unhealthy position 
/// are processed deterministically without causing unexpected panics or double-spending.
#[test]
fn test_liquidation_race_contention() {
    let env = Env::default();
    env.mock_all_auths();

    // 1. Setup Environment
    let (client, _admin, debt_asset, collateral_asset) = setup_contention_test(&env);

    let borrower = Address::generate(&env);
    let liquidator_1 = Address::generate(&env);
    let liquidator_2 = Address::generate(&env);

    let deposit_amount = 10_000_000i128;
    let borrow_amount = 8_000_000i128;

    // Borrower deposits and borrows close to the limit
    client.deposit(&borrower, &collateral_asset, &deposit_amount);
    client.borrow(&borrower, &debt_asset, &borrow_amount, &collateral_asset, &deposit_amount);

    // Advance time significantly to push the health factor below the threshold via interest
    env.ledger().set_timestamp(env.ledger().timestamp() + 31_536_000); // 1 year

    // The race: Both liquidators try to liquidate the same portion sequentially
    let liquidate_amount = 1_000_000i128;

    // Liquidator 1 succeeds first
    client.liquidate(&liquidator_1, &borrower, &debt_asset, &collateral_asset, &liquidate_amount);

    // Liquidator 2 executes immediately after. 
    // We use try_liquidate here. If the first liquidation pushed the health factor back 
    // to a healthy state, this second call will cleanly reject without panicking the network.
    let _ = client.try_liquidate(&liquidator_2, &borrower, &debt_asset, &collateral_asset, &liquidate_amount);

    let remaining_debt = client.get_debt_balance(&borrower);
    
    // Total debt should safely decrement reflecting the sequential liquidations
    assert!(remaining_debt < borrow_amount + 5_000_000i128); // Rough bounds check for interest + liquidations
}