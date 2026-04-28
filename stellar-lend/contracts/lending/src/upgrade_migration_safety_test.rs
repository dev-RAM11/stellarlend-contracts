// Upgrade and Storage Migration Safety Test Suite
//
// This test suite validates contract upgrade scenarios with focus on:
// - Storage layout compatibility across versions
// - User state preservation (balances, positions, configs)
// - Failed upgrade handling and rollback scenarios
// - Multi-step upgrade paths
// - Concurrent state modifications during upgrade proposals

extern crate alloc;
use alloc::{format, vec::Vec};

use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, String as SorobanString};

use crate::{LendingContract, LendingContractClient, UpgradeStage};

// ═══════════════════════════════════════════════════════
// Test Helpers
// ═══════════════════════════════════════════════════════

fn hash(env: &Env, b: u8) -> BytesN<32> {
    BytesN::from_array(env, &[b; 32])
}

fn setup_contract(env: &Env) -> (LendingContractClient<'_>, Address) {
    let contract_id = env.register_contract(None, LendingContract);
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    (client, admin)
}

fn setup_with_upgrade_init(
    env: &Env,
    required_approvals: u32,
) -> (LendingContractClient<'_>, Address) {
    let (client, admin) = setup_contract(env);
    client.upgrade_init(&admin, &hash(env, 1), &required_approvals);
    (client, admin)
}

// Seed user state with data store entries
fn seed_user_state(env: &Env, client: &LendingContractClient, admin: &Address, users: &[Address]) {
    // Initialize data store for user metadata
    client.data_store_init(admin);

    for (idx, _user) in users.iter().enumerate() {
        let key = SorobanString::from_str(env, &format!("user_{idx}"));
        let value = soroban_sdk::Bytes::from_slice(env, &[idx as u8; 32]);
        client.data_save(admin, &key, &value);
    }
}

// ═══════════════════════════════════════════════════════
// 1. Basic Upgrade with State Preservation
// ═══════════════════════════════════════════════════════

#[test]
fn test_upgrade_preserves_admin_and_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    assert_eq!(client.current_version(), 0);
    assert_eq!(client.current_wasm_hash(), hash(&env, 1));

    // Propose and execute upgrade
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    // Verify version and hash updated
    assert_eq!(client.current_version(), 1);
    assert_eq!(client.current_wasm_hash(), hash(&env, 2));

    // Verify admin still has control
    let new_approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &new_approver);
}

#[test]
fn test_upgrade_preserves_data_store_entries() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Initialize data store and save entries
    client.data_store_init(&admin);
    let key1 = SorobanString::from_str(&env, "balance_user1");
    let val1 = soroban_sdk::Bytes::from_slice(&env, &[1, 2, 3, 4]);
    client.data_save(&admin, &key1, &val1);

    let key2 = SorobanString::from_str(&env, "position_user2");
    let val2 = soroban_sdk::Bytes::from_slice(&env, &[5, 6, 7, 8]);
    client.data_save(&admin, &key2, &val2);

    assert_eq!(client.data_entry_count(), 2);

    // Execute upgrade
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    // Verify all data preserved
    assert_eq!(client.data_load(&key1), val1);
    assert_eq!(client.data_load(&key2), val2);
    assert_eq!(client.data_entry_count(), 2);
}

#[test]
fn test_upgrade_preserves_multiple_user_states() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Create multiple users with state
    let users: Vec<Address> = (0..5).map(|_| Address::generate(&env)).collect();
    seed_user_state(&env, &client, &admin, &users);

    let pre_upgrade_count = client.data_entry_count();
    assert_eq!(pre_upgrade_count, 5);

    // Execute upgrade
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &2);
    client.upgrade_execute(&admin, &proposal_id);

    // Verify all user states preserved
    assert_eq!(client.data_entry_count(), pre_upgrade_count);

    for (idx, _user) in users.iter().enumerate() {
        let key = SorobanString::from_str(&env, &format!("user_{idx}"));
        let expected = soroban_sdk::Bytes::from_slice(&env, &[idx as u8; 32]);
        assert_eq!(client.data_load(&key), expected);
    }
}

// ═══════════════════════════════════════════════════════
// 2. Multi-Step Upgrade Path
// ═══════════════════════════════════════════════════════

#[test]
fn test_sequential_upgrades_preserve_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);
    let key = SorobanString::from_str(&env, "persistent_data");
    let val = soroban_sdk::Bytes::from_slice(&env, &[0xAA; 16]);
    client.data_save(&admin, &key, &val);

    // Upgrade v0 -> v1
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);
    assert_eq!(client.current_version(), 1);
    assert_eq!(client.data_load(&key), val);

    // Upgrade v1 -> v2
    let p2 = client.upgrade_propose(&admin, &hash(&env, 3), &2);
    client.upgrade_execute(&admin, &p2);
    assert_eq!(client.current_version(), 2);
    assert_eq!(client.data_load(&key), val);

    // Upgrade v2 -> v5 (skip versions)
    let p3 = client.upgrade_propose(&admin, &hash(&env, 4), &5);
    client.upgrade_execute(&admin, &p3);
    assert_eq!(client.current_version(), 5);
    assert_eq!(client.data_load(&key), val);
}

#[test]
fn test_upgrade_with_state_modifications_between_versions() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);

    // Version 0: Initial state
    let k1 = SorobanString::from_str(&env, "k1");
    let v1 = soroban_sdk::Bytes::from_slice(&env, &[1]);
    client.data_save(&admin, &k1, &v1);

    // Upgrade to v1
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);

    // Modify state in v1
    let k2 = SorobanString::from_str(&env, "k2");
    let v2 = soroban_sdk::Bytes::from_slice(&env, &[2]);
    client.data_save(&admin, &k2, &v2);

    // Upgrade to v2
    let p2 = client.upgrade_propose(&admin, &hash(&env, 3), &2);
    client.upgrade_execute(&admin, &p2);

    // Verify both states preserved
    assert_eq!(client.data_load(&k1), v1);
    assert_eq!(client.data_load(&k2), v2);
    assert_eq!(client.data_entry_count(), 2);
}

// ═══════════════════════════════════════════════════════
// 3. Rollback Scenarios
// ═══════════════════════════════════════════════════════

#[test]
fn test_rollback_restores_previous_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Execute upgrade v0 -> v1
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);
    assert_eq!(client.current_version(), 1);
    assert_eq!(client.current_wasm_hash(), hash(&env, 2));

    // Rollback to v0
    client.upgrade_rollback(&admin, &proposal_id);
    assert_eq!(client.current_version(), 0);
    assert_eq!(client.current_wasm_hash(), hash(&env, 1));

    // Verify proposal marked as rolled back
    let status = client.upgrade_status(&proposal_id);
    assert_eq!(status.stage, UpgradeStage::RolledBack);
}

#[test]
fn test_rollback_preserves_user_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Setup state before upgrade
    client.data_store_init(&admin);
    let key = SorobanString::from_str(&env, "critical_data");
    let val = soroban_sdk::Bytes::from_slice(&env, &[0xFF; 32]);
    client.data_save(&admin, &key, &val);

    // Upgrade
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    // Modify state after upgrade
    let key2 = SorobanString::from_str(&env, "new_data");
    let val2 = soroban_sdk::Bytes::from_slice(&env, &[0xAA; 16]);
    client.data_save(&admin, &key2, &val2);

    // Rollback
    client.upgrade_rollback(&admin, &proposal_id);

    // Verify all state still accessible (storage is persistent)
    assert_eq!(client.data_load(&key), val);
    assert_eq!(client.data_load(&key2), val2);
}

#[test]
#[should_panic]
fn test_rollback_cannot_be_repeated() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);
    client.upgrade_rollback(&admin, &proposal_id);

    // Second rollback should fail
    client.upgrade_rollback(&admin, &proposal_id);
}

#[test]
fn test_rollback_then_new_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Upgrade and rollback
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);
    client.upgrade_rollback(&admin, &p1);

    assert_eq!(client.current_version(), 0);

    // New upgrade should work
    let p2 = client.upgrade_propose(&admin, &hash(&env, 3), &1);
    client.upgrade_execute(&admin, &p2);
    assert_eq!(client.current_version(), 1);
}

// ═══════════════════════════════════════════════════════
// 4. Failed Upgrade Scenarios
// ═══════════════════════════════════════════════════════

#[test]
#[should_panic]
fn test_execute_without_approval_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 2);

    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    // Propose but don't get enough approvals
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);

    // Try to execute without threshold - should fail
    client.upgrade_execute(&admin, &proposal_id);
}

#[test]
#[should_panic]
fn test_execute_already_executed_proposal_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    // Second execution should fail
    client.upgrade_execute(&admin, &proposal_id);
}

#[test]
#[should_panic]
fn test_propose_same_version_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Upgrade to v1
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);

    // Try to propose v1 again - should fail
    client.upgrade_propose(&admin, &hash(&env, 3), &1);
}

#[test]
#[should_panic]
fn test_propose_lower_version_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Upgrade to v5
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &5);
    client.upgrade_execute(&admin, &p1);

    // Try to propose v3 - should fail
    client.upgrade_propose(&admin, &hash(&env, 3), &3);
}

// ═══════════════════════════════════════════════════════
// 5. Concurrent Operations During Upgrade
// ═══════════════════════════════════════════════════════

#[test]
fn test_state_modifications_during_proposal_phase() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 2);

    client.data_store_init(&admin);

    // Create proposal
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Proposed
    );

    // Modify state while proposal is pending
    let key = SorobanString::from_str(&env, "during_proposal");
    let val = soroban_sdk::Bytes::from_slice(&env, &[0xBB; 8]);
    client.data_save(&admin, &key, &val);

    // Complete upgrade
    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);
    client.upgrade_approve(&approver, &proposal_id);
    client.upgrade_execute(&admin, &proposal_id);

    // Verify state preserved
    assert_eq!(client.data_load(&key), val);
}

#[test]
fn test_multiple_pending_proposals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 3);

    let approver1 = Address::generate(&env);
    let approver2 = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver1);
    client.upgrade_add_approver(&admin, &approver2);

    // Create multiple proposals
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    let p2 = client.upgrade_propose(&admin, &hash(&env, 3), &2);

    // Approve and execute first proposal
    client.upgrade_approve(&approver1, &p1);
    client.upgrade_approve(&approver2, &p1);
    client.upgrade_execute(&admin, &p1);

    assert_eq!(client.current_version(), 1);

    // Second proposal should now be invalid (version too low)
    let result = client.try_upgrade_execute(&admin, &p2);
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════
// 6. Storage Schema Migration
// ═══════════════════════════════════════════════════════

#[test]
fn test_schema_version_bump_during_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);
    assert_eq!(client.data_schema_version(), 0);

    // Upgrade contract
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);

    // Bump schema version to match new contract
    let memo = SorobanString::from_str(&env, "v1_schema_migration");
    client.data_migrate_bump_version(&admin, &1, &memo);

    assert_eq!(client.data_schema_version(), 1);
    assert_eq!(client.current_version(), 1);
}

#[test]
fn test_backup_restore_across_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);

    // Create and backup state
    let k1 = SorobanString::from_str(&env, "k1");
    let v1 = soroban_sdk::Bytes::from_slice(&env, &[1, 2, 3]);
    client.data_save(&admin, &k1, &v1);

    let backup_name = SorobanString::from_str(&env, "pre_upgrade_backup");
    client.data_backup(&admin, &backup_name);

    // Upgrade
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);

    // Modify state after upgrade
    let k2 = SorobanString::from_str(&env, "k2");
    let v2 = soroban_sdk::Bytes::from_slice(&env, &[4, 5, 6]);
    client.data_save(&admin, &k2, &v2);

    // Restore pre-upgrade backup
    client.data_restore(&admin, &backup_name);

    // Should have only pre-upgrade data
    assert_eq!(client.data_load(&k1), v1);
    assert_eq!(client.data_entry_count(), 1);
}

#[test]
fn test_migration_with_large_dataset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);

    // Create large dataset
    for i in 0..50 {
        let key = SorobanString::from_str(&env, &format!("key_{i}"));
        let val = soroban_sdk::Bytes::from_slice(&env, &[i as u8; 64]);
        client.data_save(&admin, &key, &val);
    }

    assert_eq!(client.data_entry_count(), 50);

    // Upgrade
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);

    // Verify all data intact
    assert_eq!(client.data_entry_count(), 50);

    for i in 0..50 {
        let key = SorobanString::from_str(&env, &format!("key_{i}"));
        let expected = soroban_sdk::Bytes::from_slice(&env, &[i as u8; 64]);
        assert_eq!(client.data_load(&key), expected);
    }
}

// ═══════════════════════════════════════════════════════
// 7. Authorization and Security
// ═══════════════════════════════════════════════════════

#[test]
#[should_panic]
fn test_non_admin_cannot_rollback() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    let stranger = Address::generate(&env);
    client.upgrade_rollback(&stranger, &proposal_id);
}

#[test]
#[should_panic]
fn test_non_approver_cannot_execute() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);

    let stranger = Address::generate(&env);
    client.upgrade_execute(&stranger, &proposal_id);
}

#[test]
fn test_approver_permissions_preserved_across_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 2);

    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    // Upgrade
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_approve(&approver, &p1);
    client.upgrade_execute(&admin, &p1);

    // Approver should still be able to approve new proposals
    let p2 = client.upgrade_propose(&admin, &hash(&env, 3), &2);
    let count = client.upgrade_approve(&approver, &p2);
    assert_eq!(count, 2);
}

// ═══════════════════════════════════════════════════════
// 8. Edge Cases
// ═══════════════════════════════════════════════════════

#[test]
fn test_upgrade_with_empty_data_store() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);
    assert_eq!(client.data_entry_count(), 0);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    assert_eq!(client.data_entry_count(), 0);
    assert_eq!(client.current_version(), 1);
}

#[test]
fn test_upgrade_with_max_approvers() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 10);

    // Add 9 more approvers (admin is already one)
    let approvers: Vec<Address> = (0..9).map(|_| Address::generate(&env)).collect();
    for approver in &approvers {
        client.upgrade_add_approver(&admin, approver);
    }

    // Create proposal
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);

    // Get all approvals
    for approver in &approvers {
        client.upgrade_approve(approver, &proposal_id);
    }

    // Execute
    client.upgrade_execute(&admin, &proposal_id);
    assert_eq!(client.current_version(), 1);
}

#[test]
fn test_rapid_version_increments() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Rapidly upgrade through versions
    for version in 1..=10 {
        let hash_byte = (version + 1) as u8;
        let proposal_id = client.upgrade_propose(&admin, &hash(&env, hash_byte), &version);
        client.upgrade_execute(&admin, &proposal_id);
        assert_eq!(client.current_version(), version);
    }
}

#[test]
fn test_upgrade_preserves_writer_permissions() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);
    let writer = Address::generate(&env);
    client.data_grant_writer(&admin, &writer);

    // Writer can save before upgrade
    let k1 = SorobanString::from_str(&env, "k1");
    let v1 = soroban_sdk::Bytes::from_slice(&env, &[1]);
    client.data_save(&writer, &k1, &v1);

    // Upgrade
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    // Writer can still save after upgrade
    let k2 = SorobanString::from_str(&env, "k2");
    let v2 = soroban_sdk::Bytes::from_slice(&env, &[2]);
    client.data_save(&writer, &k2, &v2);

    assert_eq!(client.data_entry_count(), 2);
}

// ═══════════════════════════════════════════════════════
// 9. Authorization error paths
// ═══════════════════════════════════════════════════════

#[test]
#[should_panic]
fn test_duplicate_approval_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 2);

    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_approve(&approver, &proposal_id);

    // Second approval from the same approver must be rejected
    client.upgrade_approve(&approver, &proposal_id);
}

#[test]
#[should_panic]
fn test_rollback_proposed_proposal_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 2);

    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    // Proposal never reaches Approved stage
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Proposed
    );

    // Rolling back a non-executed proposal must fail
    client.upgrade_rollback(&admin, &proposal_id);
}

#[test]
#[should_panic]
fn test_status_nonexistent_proposal_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup_with_upgrade_init(&env, 1);

    // Proposal id 999 was never created
    client.upgrade_status(&999u64);
}

// ═══════════════════════════════════════════════════════
// 10. Version skipping and multi-step with rollback chain
// ═══════════════════════════════════════════════════════

#[test]
fn test_version_skip_then_rollback() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Skip from v0 directly to v10
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &10);
    client.upgrade_execute(&admin, &p1);
    assert_eq!(client.current_version(), 10);

    // Rollback should restore v0
    client.upgrade_rollback(&admin, &p1);
    assert_eq!(client.current_version(), 0);
    assert_eq!(client.current_wasm_hash(), hash(&env, 1));
}

#[test]
fn test_schema_version_independent_of_upgrade_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    client.data_store_init(&admin);

    // Bump schema without upgrading contract
    let memo = SorobanString::from_str(&env, "standalone_schema_bump");
    client.data_migrate_bump_version(&admin, &5, &memo);
    assert_eq!(client.data_schema_version(), 5);
    assert_eq!(client.current_version(), 0); // contract version unchanged

    // Upgrade contract without bumping schema
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);
    assert_eq!(client.current_version(), 1);
    assert_eq!(client.data_schema_version(), 5); // schema version unchanged
}

#[test]
fn test_upgrade_with_single_approver_required() {
    let env = Env::default();
    env.mock_all_auths();
    // Admin is the only approver and threshold is 1
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // Admin proposes — auto-approved (admin is in approvers set)
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    assert_eq!(client.upgrade_status(&p1).stage, UpgradeStage::Approved);
    client.upgrade_execute(&admin, &p1);
    assert_eq!(client.current_version(), 1);
}

// ═══════════════════════════════════════════════════════
// 11. User position preservation across storage layout additions (#681)
//
// These tests model the realistic upgrade scenario where a new contract
// version introduces additional storage keys/fields (e.g. a new per-user
// rate field, an extra timestamp, or a brand-new namespaced map). The
// existing user state — collateral, debt, rates, timestamps — must be
// preserved byte-for-byte while the new fields layer on top without
// touching legacy entries.
//
// Approach
// --------
// Soroban tests cannot literally swap struct definitions mid-run, so we
// emulate a layout addition by:
//   1. Seeding rich, multi-field per-user records under the existing
//      data_store namespace (the encoded "old layout").
//   2. Performing the upgrade.
//   3. After upgrade, *adding* new storage keys for the same users
//      under fresh, non-overlapping namespaces (the "new layout").
//   4. Asserting that every legacy field is preserved verbatim and that
//      the freshly added entries coexist with — never overwrite — the
//      old ones.
//
// Security note
// -------------
// State integrity under upgrades is a load-bearing protocol invariant.
// A migration that silently mutates or drops user positions would let
// an attacker (or buggy migration) socialise losses across borrowers.
// These tests pin the contract's behaviour: legacy entries survive
// upgrades unchanged, and new keys never alias old ones.
// ═══════════════════════════════════════════════════════

/// Encode a synthetic per-user position (collateral, debt, rate, timestamp)
/// into a deterministic byte payload. We use an explicit layout so that any
/// re-ordering or truncation during a migration would be detectable.
fn encode_position(env: &Env, collateral: i128, debt: i128, rate_bps: u32, ts: u64) -> soroban_sdk::Bytes {
    let mut buf = [0u8; 40];
    buf[0..16].copy_from_slice(&collateral.to_be_bytes());
    buf[16..32].copy_from_slice(&debt.to_be_bytes());
    buf[32..36].copy_from_slice(&rate_bps.to_be_bytes());
    buf[36..40].copy_from_slice(&(ts as u32).to_be_bytes());
    soroban_sdk::Bytes::from_slice(env, &buf)
}

/// Seed a portfolio of positions for `n` users across `m` synthetic assets.
/// Each (user, asset) record is keyed as `pos_v1::{user_idx}::{asset_idx}`
/// so the layout is unambiguous when later compared against the
/// post-upgrade snapshot.
fn seed_multi_asset_positions(
    env: &Env,
    client: &LendingContractClient,
    admin: &Address,
    n_users: usize,
    n_assets: usize,
) -> Vec<(SorobanString, soroban_sdk::Bytes)> {
    client.data_store_init(admin);
    let mut entries: Vec<(SorobanString, soroban_sdk::Bytes)> = Vec::new();
    for u in 0..n_users {
        for a in 0..n_assets {
            // Deterministic but distinct values per (user, asset)
            let collateral = ((u + 1) as i128) * 10_000 + (a as i128) * 17;
            let debt = ((u + 1) as i128) * 4_000 + (a as i128) * 11;
            let rate_bps = 250u32 + (a as u32) * 25;
            let ts = 1_700_000_000u64 + (u as u64) * 3600 + (a as u64) * 60;
            let key = SorobanString::from_str(env, &format!("pos_v1_{u}_{a}"));
            let val = encode_position(env, collateral, debt, rate_bps, ts);
            client.data_save(admin, &key, &val);
            entries.push((key, val));
        }
    }
    entries
}

#[test]
fn test_positions_preserved_across_upgrade_layout_addition() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    // 1. Seed multi-asset positions in the "old" layout
    let entries = seed_multi_asset_positions(&env, &client, &admin, 4, 3);
    let pre_count = client.data_entry_count();
    assert_eq!(pre_count, 12);

    // 2. Snapshot every value before the upgrade
    let pre_snapshot: Vec<soroban_sdk::Bytes> =
        entries.iter().map(|(k, _)| client.data_load(k)).collect();

    // 3. Execute upgrade and bump schema version (simulates new layout)
    let proposal = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal);
    let memo = SorobanString::from_str(&env, "add_per_asset_rate_field");
    client.data_migrate_bump_version(&admin, &1, &memo);

    // 4. Every legacy entry must round-trip exactly
    for ((key, expected), pre) in entries.iter().zip(pre_snapshot.iter()) {
        let post = client.data_load(key);
        assert_eq!(&post, expected, "post-upgrade value differs from seeded value");
        assert_eq!(&post, pre, "post-upgrade value differs from pre-upgrade snapshot");
    }
    assert_eq!(client.data_entry_count(), pre_count);
    assert_eq!(client.data_schema_version(), 1);
}

#[test]
fn test_new_storage_fields_coexist_with_preserved_positions() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let entries = seed_multi_asset_positions(&env, &client, &admin, 3, 2);
    let pre_count = client.data_entry_count();

    // Upgrade then bump schema (v0 → v1 layout)
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);
    let memo = SorobanString::from_str(&env, "v1_layout_add_health_score");
    client.data_migrate_bump_version(&admin, &1, &memo);

    // Add brand-new keys under a fresh namespace ("v1_meta_") for every legacy
    // (user, asset) pair. These represent the newly added storage fields.
    for u in 0..3usize {
        for a in 0..2usize {
            let new_key = SorobanString::from_str(&env, &format!("v1_meta_{u}_{a}"));
            let new_val =
                soroban_sdk::Bytes::from_slice(&env, &[(u * 2 + a + 1) as u8; 24]);
            client.data_save(&admin, &new_key, &new_val);
        }
    }

    // Legacy entries must remain untouched
    for (key, expected) in entries.iter() {
        assert_eq!(&client.data_load(key), expected);
    }

    // New entries must be readable and distinct from legacy ones
    for u in 0..3usize {
        for a in 0..2usize {
            let new_key = SorobanString::from_str(&env, &format!("v1_meta_{u}_{a}"));
            let expected =
                soroban_sdk::Bytes::from_slice(&env, &[(u * 2 + a + 1) as u8; 24]);
            assert_eq!(client.data_load(&new_key), expected);
        }
    }

    assert_eq!(client.data_entry_count(), pre_count + 6);
}

#[test]
fn test_position_decoding_after_upgrade_round_trip() {
    // Asserts that every encoded field (collateral, debt, rate, timestamp)
    // survives the upgrade with bit-identical fidelity. This catches
    // off-by-one truncation, byte-order flips, or accidental zeroing
    // during a migration.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);
    client.data_store_init(&admin);

    // A handful of explicit, edge-case positions
    let cases: [(i128, i128, u32, u64); 5] = [
        (1, 1, 1, 1),
        (i128::MAX / 2, i128::MAX / 4, 9_999, 1_700_000_000),
        (10_000_000_000_000, 5_000_000_000_000, 0, 0),
        (123_456_789, 987_654_321, 7_500, 4_294_967_290),
        (0, 0, 10_000, 1),
    ];

    let mut keys: Vec<SorobanString> = Vec::new();
    for (i, (c, d, r, t)) in cases.iter().enumerate() {
        let k = SorobanString::from_str(&env, &format!("edge_pos_{i}"));
        client.data_save(&admin, &k, &encode_position(&env, *c, *d, *r, *t));
        keys.push(k);
    }

    // Upgrade and bump schema
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);
    client.data_migrate_bump_version(
        &admin,
        &1,
        &SorobanString::from_str(&env, "v1_decode_check"),
    );

    // Decode each entry post-upgrade and assert exact field-level equality
    for (k, (c, d, r, t)) in keys.iter().zip(cases.iter()) {
        let bytes = client.data_load(k);
        let mut buf = [0u8; 40];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = bytes.get(i as u32).expect("byte present");
        }
        let collateral = i128::from_be_bytes(buf[0..16].try_into().unwrap());
        let debt = i128::from_be_bytes(buf[16..32].try_into().unwrap());
        let rate = u32::from_be_bytes(buf[32..36].try_into().unwrap());
        let ts = u32::from_be_bytes(buf[36..40].try_into().unwrap()) as u64;
        assert_eq!(collateral, *c, "collateral mismatch after upgrade");
        assert_eq!(debt, *d, "debt mismatch after upgrade");
        assert_eq!(rate, *r, "rate_bps mismatch after upgrade");
        assert_eq!(ts, *t, "timestamp mismatch after upgrade");
    }
}

#[test]
fn test_positions_preserved_across_sequential_layout_additions() {
    // Three sequential upgrades, each simulating a new storage field
    // layered on top. The original seeded positions must remain intact
    // after every upgrade step, never overwritten by the additive
    // migration writes.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let entries = seed_multi_asset_positions(&env, &client, &admin, 2, 4);
    let baseline_count = client.data_entry_count();

    // v0 → v1: add per-position health score
    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);
    client.data_migrate_bump_version(
        &admin,
        &1,
        &SorobanString::from_str(&env, "v1_health_score"),
    );
    for (i, (key, expected)) in entries.iter().enumerate() {
        assert_eq!(&client.data_load(key), expected);
        let new_k = SorobanString::from_str(&env, &format!("v1_hs_{i}"));
        client.data_save(&admin, &new_k, &soroban_sdk::Bytes::from_slice(&env, &[1u8; 4]));
    }

    // v1 → v2: add per-position last-accrual timestamp
    let p2 = client.upgrade_propose(&admin, &hash(&env, 3), &2);
    client.upgrade_execute(&admin, &p2);
    client.data_migrate_bump_version(
        &admin,
        &2,
        &SorobanString::from_str(&env, "v2_last_accrual"),
    );
    for (i, (key, expected)) in entries.iter().enumerate() {
        assert_eq!(&client.data_load(key), expected);
        let new_k = SorobanString::from_str(&env, &format!("v2_acc_{i}"));
        client.data_save(&admin, &new_k, &soroban_sdk::Bytes::from_slice(&env, &[2u8; 8]));
    }

    // v2 → v3: add per-position liquidation flag
    let p3 = client.upgrade_propose(&admin, &hash(&env, 4), &3);
    client.upgrade_execute(&admin, &p3);
    client.data_migrate_bump_version(
        &admin,
        &3,
        &SorobanString::from_str(&env, "v3_liq_flag"),
    );
    for (i, (key, expected)) in entries.iter().enumerate() {
        assert_eq!(&client.data_load(key), expected);
        let new_k = SorobanString::from_str(&env, &format!("v3_flag_{i}"));
        client.data_save(&admin, &new_k, &soroban_sdk::Bytes::from_slice(&env, &[0xFFu8; 1]));
    }

    // Original entries are still present and unchanged
    for (key, expected) in entries.iter() {
        assert_eq!(&client.data_load(key), expected);
    }

    // Final layout: 8 originals + 8 (v1) + 8 (v2) + 8 (v3) = 32 entries
    assert_eq!(client.data_entry_count(), baseline_count + 24);
    assert_eq!(client.data_schema_version(), 3);
    assert_eq!(client.current_version(), 3);
}

#[test]
fn test_migration_preserves_positions_under_rollback() {
    // Even when an upgrade is rolled back after a migration write was
    // already performed, both the legacy positions AND the migration's
    // new entries persist (Soroban storage is not transactional with
    // upgrade state). This pins behaviour so a future migration author
    // does not inadvertently rely on rollback to "undo" writes.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let entries = seed_multi_asset_positions(&env, &client, &admin, 2, 2);

    let proposal = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal);

    // Simulate the migration writing some new fields
    let migration_key = SorobanString::from_str(&env, "v1_migration_flag");
    let migration_val = soroban_sdk::Bytes::from_slice(&env, &[0xAB; 4]);
    client.data_save(&admin, &migration_key, &migration_val);

    // Rollback the upgrade
    client.upgrade_rollback(&admin, &proposal);
    assert_eq!(client.current_version(), 0);

    // Legacy positions still intact
    for (key, expected) in entries.iter() {
        assert_eq!(&client.data_load(key), expected);
    }
    // Migration write also persists across rollback (documents real behaviour)
    assert_eq!(client.data_load(&migration_key), migration_val);
}

#[test]
fn test_view_consistency_after_upgrade() {
    // The total entry count and per-key reads must match what we wrote.
    // This serves as a "view consistency" check at the data_store level:
    // the public read API agrees with what the upgrade preserved.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup_with_upgrade_init(&env, 1);

    let entries = seed_multi_asset_positions(&env, &client, &admin, 5, 2);
    let pre_count = client.data_entry_count();

    // Backup before upgrade so we can compare restore semantics
    let backup_name = SorobanString::from_str(&env, "pre_layout_change");
    client.data_backup(&admin, &backup_name);

    let p1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &p1);
    client.data_migrate_bump_version(
        &admin,
        &1,
        &SorobanString::from_str(&env, "v1_view_consistency"),
    );

    // Per-entry view consistency
    for (key, expected) in entries.iter() {
        assert_eq!(&client.data_load(key), expected);
    }
    // Aggregate view consistency
    assert_eq!(client.data_entry_count(), pre_count);

    // Restoring the pre-upgrade backup must yield the exact same set
    client.data_restore(&admin, &backup_name);
    for (key, expected) in entries.iter() {
        assert_eq!(&client.data_load(key), expected);
    }
    assert_eq!(client.data_entry_count(), pre_count);
}
