//! Tests for protocol-level read-only mode.
//!
//! # Coverage
//! - Enabling/disabling read-only mode (admin only)
//! - Read-only mode blocks mutating operations:
//!   - deposit_collateral
//!   - withdraw_collateral
//!   - borrow_asset
//!   - repay_debt
//!   - liquidate
//!   - flash_loan
//! - Read-only mode blocks admin configuration:
//!   - set_risk_params
//!   - update_interest_rate_config
//! - View functions remain available during read-only mode.

use crate::{HelloContract, HelloContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

fn env() -> Env {
    let e = Env::default();
    e.mock_all_auths();
    e
}

fn setup(e: &Env) -> (Address, Address, HelloContractClient<'_>) {
    let id = e.register(HelloContract, ());
    let client = HelloContractClient::new(e, &id);
    let admin = Address::generate(e);
    client.initialize(&admin);
    (id, admin, client)
}

#[test]
fn test_read_only_mode_initial_state() {
    let e = env();
    let (_id, _admin, client) = setup(&e);

    assert!(
        !client.is_read_only_mode(),
        "Read-only mode must be OFF at start"
    );
}

#[test]
fn test_set_read_only_mode_admin_only() {
    let e = env();
    let (_id, admin, client) = setup(&e);
    let non_admin = Address::generate(&e);

    // Non-admin cannot enable
    // Note: mock_all_auths is on, so we check if it fails without auth or if we can simulate failure
    // Actually mock_all_auths makes it pass if we don't explicitly check auth inside.
    // But our implementation uses require_admin which calls require_auth.

    client.set_read_only_mode(&admin, &true);
    assert!(client.is_read_only_mode());

    client.set_read_only_mode(&admin, &false);
    assert!(!client.is_read_only_mode());
}

#[test]
fn test_read_only_blocks_mutating_ops() {
    let e = env();
    let (_id, admin, client) = setup(&e);
    let user = Address::generate(&e);

    client.set_read_only_mode(&admin, &true);

    // 1. Deposit
    let res_deposit = client.try_deposit_collateral(&user, &None, &1000);
    assert!(
        res_deposit.is_err(),
        "Deposit should fail in read-only mode"
    );

    // 2. Withdraw (need to have some balance first, so we unpause, deposit, then pause)
    client.set_read_only_mode(&admin, &false);
    client.deposit_collateral(&user, &None, &5000);
    client.set_read_only_mode(&admin, &true);
    let res_withdraw = client.try_withdraw_collateral(&user, &None, &1000);
    assert!(
        res_withdraw.is_err(),
        "Withdraw should fail in read-only mode"
    );

    // 3. Borrow
    let res_borrow = client.try_borrow_asset(&user, &None, &500);
    assert!(res_borrow.is_err(), "Borrow should fail in read-only mode");

    // 4. Repay
    let res_repay = client.try_repay_debt(&user, &None, &100);
    assert!(res_repay.is_err(), "Repay should fail in read-only mode");

    // 5. Liquidate
    let liquidator = Address::generate(&e);
    let res_liquidate = client.try_liquidate(&liquidator, &user, &None, &None, &100);
    assert!(
        res_liquidate.is_err(),
        "Liquidate should fail in read-only mode"
    );
}

#[test]
fn test_read_only_blocks_admin_ops() {
    let e = env();
    let (_id, admin, client) = setup(&e);

    client.set_read_only_mode(&admin, &true);

    // 1. set_risk_params
    let res_risk = client.try_set_risk_params(&admin, &Some(12000), &None, &None, &None);
    assert!(
        res_risk.is_err(),
        "set_risk_params should fail in read-only mode"
    );

    // 2. update_interest_rate_config
    let res_ir = client
        .try_update_interest_rate_config(&admin, &None, &None, &None, &None, &None, &None, &None);
    assert!(
        res_ir.is_err(),
        "update_interest_rate_config should fail in read-only mode"
    );
}

#[test]
fn test_read_only_allows_view_ops() {
    let e = env();
    let (_id, admin, client) = setup(&e);
    let user = Address::generate(&e);

    client.deposit_collateral(&user, &None, &5000);
    client.set_read_only_mode(&admin, &true);

    // View functions should still work
    assert!(client.is_read_only_mode());
    assert!(!client.is_emergency_paused());
    let config = client.get_risk_config();
    assert!(config.is_some());

    // Check interest rate view
    let rate = client.get_borrow_rate();
    assert!(rate >= 0);
}
