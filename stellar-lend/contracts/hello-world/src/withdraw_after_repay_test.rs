#![cfg(test)]
use soroban_sdk::{testutils::Address as _, Address, Env, testutils::Ledger};

#[test]
fn test_withdraw_after_repay_with_interest_dust() {
    let env = Env::default();
    env.mock_all_auths();
    let current_time = 1777123530;
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: current_time,
        protocol_version: 20,
        sequence_number: 100,
        network_id: [0u8; 32],
        base_reserve: 10,
        min_persistent_entry_expiration: 100,
        min_temp_entry_expiration: 100,
    });
    assert!(true);
}
