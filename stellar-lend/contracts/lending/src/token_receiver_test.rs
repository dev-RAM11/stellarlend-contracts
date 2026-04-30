//! # Token Receiver Tests
//!
//! Complete test coverage for `token_receiver.rs` under the secure pull-based
//! token flow.

use crate::{borrow::BorrowError, pause::PauseType, LendingContract, LendingContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env, IntoVal, Symbol, Vec,
};

fn setup() -> (Env, Address, LendingContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1_000);
    (env, contract_id, client, admin)
}

fn action_payload(env: &Env, action: &str) -> soroban_sdk::Vec<soroban_sdk::Val> {
    (Symbol::new(env, action),).into_val(env)
}

fn versioned_payload(env: &Env, version: u32, action: &str) -> soroban_sdk::Vec<soroban_sdk::Val> {
    (version, Symbol::new(env, action)).into_val(env)
}

fn malformed_payload(env: &Env) -> soroban_sdk::Vec<soroban_sdk::Val> {
    // Create a payload with invalid types
    let mut payload = soroban_sdk::Vec::new(env);
    payload.push_back(1u32.into_val(env)); // valid version
    payload.push_back(123456u32.into_val(env)); // invalid action type (should be Symbol)
    payload
}

fn register_token(env: &Env, admin: &Address) -> Address {
    env.register_stellar_asset_contract_v2(admin.clone())
        .address()
}

/// Creates a Stellar asset contract AND registers it in the lending registry.
fn register_token_in_registry(env: &Env, client: &LendingContractClient, admin: &Address) -> Address {
    let asset = register_token(env, admin);
    client.register_asset(admin, &asset);
    asset
}

fn mint(env: &Env, asset: &Address, owner: &Address, amount: i128) {
    let token_admin = token::StellarAssetClient::new(env, asset);
    token_admin.mint(owner, &amount);
}

fn approve(env: &Env, asset: &Address, owner: &Address, spender: &Address, amount: i128) {
    let token_client = token::Client::new(env, asset);
    token_client.approve(owner, spender, &amount, &200);
}

fn mint_and_approve(env: &Env, asset: &Address, owner: &Address, spender: &Address, amount: i128) {
    mint(env, asset, owner, amount);
    approve(env, asset, owner, spender, amount);
}

#[test]
fn test_receive_empty_payload() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    // Register asset so we reach the payload check
    let asset = register_token_in_registry(&env, &client, &admin);
    let payload: soroban_sdk::Vec<soroban_sdk::Val> = Vec::new(&env);

    let result = client.try_receive(&asset, &from, &50_000, &payload);
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

#[test]
fn test_receive_invalid_action() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    // Register asset so we reach the action check
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);
    let token_client = token::Client::new(&env, &asset);

    let result = client.try_receive(&asset, &from, &50_000, &versioned_payload(&env, 1, "withdraw"));
    assert_eq!(result, Err(Ok(BorrowError::AssetNotSupported)));
    assert_eq!(token_client.balance(&from), 50_000);
    assert_eq!(token_client.balance(&contract_id), 0);
}

#[test]
fn test_receive_requires_allowance() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint(&env, &asset, &from, 10_000);

    let result = client.try_receive(&asset, &from, &10_000, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::Unauthorized)));
}

#[test]
fn test_receive_deposit_success() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);
    let token_client = token::Client::new(&env, &asset);

    client.receive(&asset, &from, &50_000, &versioned_payload(&env, 1, "deposit"));

    let collateral = client.get_user_collateral(&from);
    assert_eq!(collateral.amount, 50_000);
    assert_eq!(collateral.asset, asset);
    assert_eq!(token_client.balance(&from), 0);
    assert_eq!(token_client.balance(&contract_id), 50_000);
}

#[test]
fn test_receive_deposit_accumulates_collateral() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let payload = versioned_payload(&env, 1, "deposit");
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);

    client.receive(&asset, &from, &30_000, &payload);
    client.receive(&asset, &from, &20_000, &payload);

    let collateral = client.get_user_collateral(&from);
    assert_eq!(collateral.amount, 50_000);
    assert_eq!(
        token::Client::new(&env, &asset).balance(&contract_id),
        50_000
    );
}

#[test]
fn test_receive_deposit_zero_amount() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);

    let result = client.try_receive(&asset, &from, &0, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

#[test]
fn test_receive_deposit_negative_amount() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);

    let result = client.try_receive(&asset, &from, &-1, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

#[test]
fn test_receive_deposit_asset_mismatch() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset_a = register_token_in_registry(&env, &client, &admin);
    let asset_b = register_token_in_registry(&env, &client, &admin);
    let payload = versioned_payload(&env, 1, "deposit");
    mint_and_approve(&env, &asset_a, &from, &contract_id, 10_000);
    mint_and_approve(&env, &asset_b, &from, &contract_id, 10_000);

    client.receive(&asset_a, &from, &10_000, &payload);

    let result = client.try_receive(&asset_b, &from, &10_000, &payload);
    assert_eq!(result, Err(Ok(BorrowError::AssetNotSupported)));
    assert_eq!(token::Client::new(&env, &asset_b).balance(&contract_id), 0);
}

#[test]
fn test_receive_deposit_overflow() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);

    client.deposit_collateral(&from, &asset, &i128::MAX);
    mint_and_approve(&env, &asset, &from, &contract_id, 1);

    let result = client.try_receive(&asset, &from, &1, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::Overflow)));
    assert_eq!(token::Client::new(&env, &asset).balance(&contract_id), 0);
}

#[test]
fn test_receive_deposit_respects_deposit_pause() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);

    client.set_pause(&admin, &PauseType::Deposit, &true);

    let result = client.try_receive(&asset, &from, &50_000, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::ProtocolPaused)));
    assert_eq!(token::Client::new(&env, &asset).balance(&from), 50_000);
}

#[test]
fn test_receive_deposit_respects_global_pause() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);

    client.set_pause(&admin, &PauseType::All, &true);

    let result = client.try_receive(&asset, &from, &50_000, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::ProtocolPaused)));
    assert_eq!(token::Client::new(&env, &asset).balance(&contract_id), 0);
}

#[test]
fn test_receive_repay_success() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 5_000);

    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);
    client.receive(&asset, &from, &5_000, &versioned_payload(&env, 1, "repay"));

    let debt = client.get_user_debt(&from);
    assert_eq!(debt.borrowed_amount, 5_000);
    assert_eq!(debt.interest_accrued, 0);
    assert_eq!(
        token::Client::new(&env, &asset).balance(&contract_id),
        5_000
    );
}

#[test]
fn test_receive_repay_full_debt() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 10_000);

    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);
    client.receive(&asset, &from, &10_000, &versioned_payload(&env, 1, "repay"));

    let debt = client.get_user_debt(&from);
    assert_eq!(debt.borrowed_amount, 0);
    assert_eq!(debt.interest_accrued, 0);
}

#[test]
fn test_receive_repay_interest_repaid_first() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 500);

    env.ledger().with_mut(|li| li.timestamp = 0);
    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);

    const SECONDS_PER_YEAR: u64 = 31_536_000;
    env.ledger().with_mut(|li| li.timestamp = SECONDS_PER_YEAR);

    client.receive(&asset, &from, &500, &versioned_payload(&env, 1, "repay"));

    let debt = client.get_user_debt(&from);
    assert_eq!(debt.borrowed_amount, 10_000);
    assert_eq!(debt.interest_accrued, 0);
}

#[test]
fn test_receive_repay_zero_amount() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);

    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);

    let result = client.try_receive(&asset, &from, &0, &versioned_payload(&env, 1, "repay"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

#[test]
fn test_receive_repay_negative_amount() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);

    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);

    let result = client.try_receive(&asset, &from, &-500, &versioned_payload(&env, 1, "repay"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
}

#[test]
fn test_receive_repay_no_debt() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 5_000);

    let result = client.try_receive(&asset, &from, &5_000, &versioned_payload(&env, 1, "repay"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidAmount)));
    assert_eq!(token::Client::new(&env, &asset).balance(&contract_id), 0);
}

#[test]
fn test_receive_repay_wrong_asset() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let borrow_asset = register_token_in_registry(&env, &client, &admin);
    let wrong_asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &wrong_asset, &from, &contract_id, 5_000);

    client.borrow(&from, &borrow_asset, &10_000, &collateral_asset, &20_000);

    let result = client.try_receive(&wrong_asset, &from, &5_000, &versioned_payload(&env, 1, "repay"));
    assert_eq!(result, Err(Ok(BorrowError::AssetNotSupported)));
    assert_eq!(
        token::Client::new(&env, &wrong_asset).balance(&contract_id),
        0
    );
}

#[test]
fn test_receive_repay_overpayment() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 10_001);

    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);

    let result = client.try_receive(&asset, &from, &10_001, &versioned_payload(&env, 1, "repay"));
    assert_eq!(result, Err(Ok(BorrowError::RepayAmountTooHigh)));
    assert_eq!(token::Client::new(&env, &asset).balance(&contract_id), 0);
}

#[test]
fn test_receive_repay_respects_repay_pause() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 5_000);

    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);
    client.set_pause(&admin, &PauseType::Repay, &true);

    let result = client.try_receive(&asset, &from, &5_000, &versioned_payload(&env, 1, "repay"));
    assert_eq!(result, Err(Ok(BorrowError::ProtocolPaused)));
    assert_eq!(token::Client::new(&env, &asset).balance(&from), 5_000);
}

#[test]
fn test_receive_repay_respects_global_pause() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 5_000);

    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);
    client.set_pause(&admin, &PauseType::All, &true);

    let result = client.try_receive(&asset, &from, &5_000, &versioned_payload(&env, 1, "repay"));
    assert_eq!(result, Err(Ok(BorrowError::ProtocolPaused)));
    assert_eq!(token::Client::new(&env, &asset).balance(&contract_id), 0);
}

#[test]
fn test_direct_deposit_repay() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    let collateral_asset = register_token_in_registry(&env, &client, &admin);

    client.deposit_collateral(&from, &collateral_asset, &20_000);
    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);
    client.repay(&from, &asset, &5_000);

    assert_eq!(client.get_user_collateral(&from).amount, 40_000);
    assert_eq!(client.get_user_debt(&from).borrowed_amount, 5_000);
}

#[test]
fn test_receive_deposit_exceeds_cap() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token(&env, &admin);
    let payload = versioned_payload(&env, 1, "deposit");

    // Set cap to 50k
    client.initialize_deposit_settings(&50_000, &100);

    // Try to deposit 50,001
    mint_and_approve(&env, &asset, &from, &contract_id, 50_001);
    let result = client.try_receive(&asset, &from, &50_001, &payload);

    assert_eq!(result, Err(Ok(BorrowError::ExceedsDepositCap)));

    // Verify atomicity: tokens should NOT have been pulled from the user
    let token_client = token::Client::new(&env, &asset);
    assert_eq!(token_client.balance(&from), 50_001);
    assert_eq!(token_client.balance(&contract_id), 0);

    // Verify state: collateral should be 0
    assert_eq!(client.get_user_collateral(&from).amount, 0);
}

#[test]
fn test_receive_deposit_at_cap_boundary() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token(&env, &admin);
    let payload = versioned_payload(&env, 1, "deposit");

    // Set cap to 50k
    client.initialize_deposit_settings(&50_000, &100);

    // Deposit exactly 50k
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);
    client.receive(&asset, &from, &50_000, &payload);

    assert_eq!(client.get_user_collateral(&from).amount, 50_000);

    // Next 1 unit should fail
    mint_and_approve(&env, &asset, &from, &contract_id, 1);
    let result = client.try_receive(&asset, &from, &1, &payload);
    assert_eq!(result, Err(Ok(BorrowError::ExceedsDepositCap)));
}

// ===== SECURITY TESTS =====

#[test]
fn test_receive_payload_version_invalid() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &env.current_contract_address(), 10_000);

    // Test with wrong version (version 0)
    let result = client.try_receive(&asset, &from, &10_000, &versioned_payload(&env, 0, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidPayloadVersion)));

    // Test with wrong version (version 2)
    let result = client.try_receive(&asset, &from, &10_000, &versioned_payload(&env, 2, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidPayloadVersion)));

    // Test with future version (version 999)
    let result = client.try_receive(&asset, &from, &10_000, &versioned_payload(&env, 999, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::InvalidPayloadVersion)));
}

#[test]
fn test_receive_payload_malformed_structure() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &env.current_contract_address(), 10_000);

    // Test with empty payload
    let empty_payload: soroban_sdk::Vec<soroban_sdk::Val> = soroban_sdk::Vec::new(&env);
    let result = client.try_receive(&asset, &from, &10_000, &empty_payload);
    assert_eq!(result, Err(Ok(BorrowError::MalformedPayload)));

    // Test with single element payload (missing action)
    let mut single_payload = soroban_sdk::Vec::new(&env);
    single_payload.push_back(1u32.into_val(&env));
    let result = client.try_receive(&asset, &from, &10_000, &single_payload);
    assert_eq!(result, Err(Ok(BorrowError::MalformedPayload)));

    // Test with malformed payload (wrong action type)
    let result = client.try_receive(&asset, &from, &10_000, &malformed_payload(&env));
    assert_eq!(result, Err(Ok(BorrowError::MalformedPayload)));
}

#[test]
fn test_receive_payload_too_long() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &env.current_contract_address(), 10_000);

    // Create payload that exceeds MAX_PAYLOAD_LENGTH
    let mut oversized_payload = soroban_sdk::Vec::new(&env);
    oversized_payload.push_back(1u32.into_val(&env)); // version
    oversized_payload.push_back(Symbol::new(&env, "deposit").into_val(&env)); // action
    
    // Add 9 more elements to exceed the limit (total 11 > MAX_PAYLOAD_LENGTH 10)
    for i in 0..9 {
        oversized_payload.push_back(i.into_val(&env));
    }

    let result = client.try_receive(&asset, &from, &10_000, &oversized_payload);
    assert_eq!(result, Err(Ok(BorrowError::MalformedPayload)));
}

#[test]
fn test_receive_unauthorized_sender_token_contract() {
    let (env, _contract_id, client, admin) = setup();
    let asset = register_token_in_registry(&env, &client, &admin);
    mint(&env, &asset, &asset, 10_000); // Mint tokens to the token contract itself
    
    // Try to have the token contract call receive directly
    let result = client.try_receive(&asset, &asset, &10_000, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::UnauthorizedSender)));
}

#[test]
fn test_receive_unauthorized_sender_lending_contract() {
    let (env, contract_id, client, admin) = setup();
    let asset = register_token_in_registry(&env, &client, &admin);
    mint(&env, &asset, &contract_id, 10_000); // Mint tokens to the lending contract
    
    // Try to have the lending contract call receive (self-call attack)
    let result = client.try_receive(&asset, &contract_id, &10_000, &versioned_payload(&env, 1, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::UnauthorizedSender)));
}

#[test]
fn test_receive_invalid_action_symbol() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &env.current_contract_address(), 10_000);

    // Test with invalid action symbols
    let invalid_actions = vec!["withdraw", "transfer", "mint", "burn", "hack", "exploit"];
    
    for action in invalid_actions {
        let result = client.try_receive(&asset, &from, &10_000, &versioned_payload(&env, 1, action));
        assert_eq!(result, Err(Ok(BorrowError::AssetNotSupported)));
    }
}

#[test]
fn test_receive_legacy_payload_format_rejected() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &env.current_contract_address(), 10_000);

    // Test with legacy payload format (no version, just action)
    let result = client.try_receive(&asset, &from, &10_000, &action_payload(&env, "deposit"));
    assert_eq!(result, Err(Ok(BorrowError::MalformedPayload)));
}

#[test]
fn test_receive_payload_version_boundary_values() {
    let (env, _contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &env.current_contract_address(), 10_000);

    // Test with boundary version values
    let boundary_versions = vec![u32::MIN, u32::MAX, PAYLOAD_VERSION - 1, PAYLOAD_VERSION + 1];
    
    for version in boundary_versions {
        if version != PAYLOAD_VERSION {
            let result = client.try_receive(&asset, &from, &10_000, &versioned_payload(&env, version, "deposit"));
            assert_eq!(result, Err(Ok(BorrowError::InvalidPayloadVersion)));
        }
    }
}

#[test]
fn test_receive_successful_with_versioned_payload() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);
    let token_client = token::Client::new(&env, &asset);

    // Test successful deposit with versioned payload
    client.receive(&asset, &from, &50_000, &versioned_payload(&env, 1, "deposit"));

    let collateral = client.get_user_collateral(&from);
    assert_eq!(collateral.amount, 50_000);
    assert_eq!(collateral.asset, asset);
    assert_eq!(token_client.balance(&from), 0);
    assert_eq!(token_client.balance(&contract_id), 50_000);

    // Test successful repay with versioned payload
    let collateral_asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 5_000);
    client.borrow(&from, &asset, &10_000, &collateral_asset, &20_000);
    
    client.receive(&asset, &from, &5_000, &versioned_payload(&env, 1, "repay"));

    let debt = client.get_user_debt(&from);
    assert_eq!(debt.borrowed_amount, 5_000);
    assert_eq!(debt.interest_accrued, 0);
}

#[test]
fn test_receive_payload_with_extra_data_ignored() {
    let (env, contract_id, client, admin) = setup();
    let from = Address::generate(&env);
    let asset = register_token_in_registry(&env, &client, &admin);
    mint_and_approve(&env, &asset, &from, &contract_id, 50_000);

    // Create payload with extra data (should be ignored but allowed)
    let mut payload_with_extra = versioned_payload(&env, 1, "deposit");
    payload_with_extra.push_back("extra_data".into_val(&env));
    payload_with_extra.push_back(42u32.into_val(&env));

    client.receive(&asset, &from, &50_000, &payload_with_extra);

    let collateral = client.get_user_collateral(&from);
    assert_eq!(collateral.amount, 50_000);
    assert_eq!(collateral.asset, asset);
}
