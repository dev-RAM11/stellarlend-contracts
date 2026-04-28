use super::*;
use crate::deposit::DepositError;
use crate::flash_loan::FlashLoanError;
use crate::oracle::OracleError;
use crate::withdraw::WithdrawError;
use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env, Symbol, TryFromVal,
};

#[test]
fn test_read_only_mode_toggle_and_query() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1000);

    // Default: false
    assert!(!client.is_read_only());

    // Enable
    client.set_read_only(&admin, &true);
    assert!(client.is_read_only());

    // Disable
    client.set_read_only(&admin, &false);
    assert!(!client.is_read_only());
}

#[test]
fn test_read_only_mode_authorization() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1000);

    // Non-admin cannot set read-only
    let result = client.try_set_read_only(&user, &true);
    assert_eq!(result, Err(Ok(BorrowError::Unauthorized)));
}

#[test]
fn test_read_only_mode_blocks_user_ops() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    // Enable read-only
    client.set_read_only(&admin, &true);

    // Test blocking
    assert_eq!(
        client.try_borrow(&user, &asset, &10_000, &collateral_asset, &20_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_deposit(&user, &asset, &10_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_repay(&user, &asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw(&user, &asset, &10_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );
    assert_eq!(
        client.try_liquidate(&admin, &user, &asset, &collateral_asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_flash_loan(&user, &asset, &1_000, &soroban_sdk::Bytes::new(&env)),
        Err(Ok(FlashLoanError::ProtocolPaused))
    );
    assert_eq!(
        client.try_deposit_collateral(&user, &asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
}

#[test]
fn test_read_only_mode_blocks_admin_ops() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asset = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1000);

    // Enable read-only
    client.set_read_only(&admin, &true);

    assert_eq!(
        client.try_credit_insurance_fund(&admin, &asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_offset_bad_debt(&admin, &asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_update_price_feed(&admin, &asset, &100),
        Err(Ok(OracleError::OraclePaused))
    );
}

#[test]
fn test_read_only_mode_allows_view_functions() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1000);

    // Enable read-only
    client.set_read_only(&admin, &true);

    // View functions should work
    client.get_user_position(&user);
    client.get_health_factor(&user);
    client.get_collateral_balance(&user);
    client.get_debt_balance(&user);
}

#[test]
fn test_read_only_mode_blocks_cross_asset_ops() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1000);

    // Enable read-only
    client.set_read_only(&admin, &true);

    assert_eq!(
        client.try_deposit_collateral_asset(&user, &asset, &10_000),
        Err(Ok(CrossAssetError::ProtocolPaused))
    );
    assert_eq!(
        client.try_borrow_asset(&user, &asset, &10_000),
        Err(Ok(CrossAssetError::ProtocolPaused))
    );
    assert_eq!(
        client.try_repay_asset(&user, &asset, &10_000),
        Err(Ok(CrossAssetError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw_asset(&user, &asset, &10_000),
        Err(Ok(CrossAssetError::ProtocolPaused))
    );
}

#[test]
fn test_read_only_event() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1000);

    client.set_read_only(&admin, &true);

    // let events = env.events().all();
    // let last_event = events.get(events.len() - 1).unwrap();

    // assert_eq!(last_event.0, contract_id);
    // let topic: Symbol = Symbol::try_from_val(&env, &last_event.1.get(0).unwrap()).unwrap();
    // assert_eq!(topic, Symbol::new(&env, "read_only_event"));
}
