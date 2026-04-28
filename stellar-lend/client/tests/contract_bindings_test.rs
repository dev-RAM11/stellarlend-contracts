#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};
// Adjust the import path if your generated bindings are exposed differently in the client crate.
use stellarlend_lending::{LendingContract, LendingContractClient};

/// E2E Harness to prevent interface drift between the generated client and the contract.
#[test]
fn test_client_deposit_borrow_repay_withdraw_flow() {
    let env = Env::default();
    env.mock_all_auths();

    // 1. Deploy Contract
    let contract_id = env.register_contract(None, LendingContract);
    let client = LendingContractClient::new(&env, &contract_id);

    // Setup mock accounts and assets
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let debt_asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    let debt_ceiling = 1_000_000_000i128;
    let min_borrow_amount = 100i128;

    // Initialize the protocol
    client.initialize(&admin, &debt_ceiling, &min_borrow_amount);

    let deposit_amount = 10_000_000i128;
    let borrow_amount = 5_000_000i128;
    // The borrow function requires the collateral asset and amount to be specified
    let collateral_amount_for_borrow = 8_000_000i128;

    // 2. Deposit Flow
    let deposited = client.deposit(&user, &collateral_asset, &deposit_amount);
    assert_eq!(deposited, deposit_amount);

    // Check view function
    let initial_collateral = client.get_collateral_balance(&user);
    assert!(initial_collateral >= deposit_amount);

    // 3. Borrow Flow
    client.borrow(
        &user,
        &debt_asset,
        &borrow_amount,
        &collateral_asset,
        &collateral_amount_for_borrow,
    );

    let debt = client.get_debt_balance(&user);
    assert_eq!(debt, borrow_amount); // Assuming no interest accrued immediately at block 0

    // 4. Repay Flow
    client.repay(&user, &debt_asset, &borrow_amount);
    
    let post_repay_debt = client.get_debt_balance(&user);
    assert_eq!(post_repay_debt, 0);

    // 5. Withdraw Flow
    let withdrawn = client.withdraw(&user, &collateral_asset, &deposit_amount);
    assert_eq!(withdrawn, deposit_amount);
}

/// Harness to ensure access control and constraint errors translate to panics in the generated client.
#[test]
#[should_panic]
fn test_client_unauthorized_admin_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, LendingContract);
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let fake_admin = Address::generate(&env);

    // Initialize successfully with the real admin
    client.initialize(&admin, &1_000_000_000i128, &100i128);

    // Attempting to modify protocol settings with the wrong admin should trigger an unauthorized error/panic
    client.set_liquidation_threshold_bps(&fake_admin, &8000i128);
}