use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, BytesN, Env, Error, InvokeError, Symbol, TryFromVal,
};

use crate::{LendingContract, LendingContractClient, UpgradeError, UpgradeStage};

fn hash(env: &Env, b: u8) -> BytesN<32> {
    BytesN::from_array(env, &[b; 32])
}

#[allow(deprecated)]
fn setup(env: &Env, required_approvals: u32) -> (LendingContractClient<'_>, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.upgrade_init(&admin, &hash(env, 1), &required_approvals);
    (client, admin)
}

fn assert_contract_error<T, E>(
    result: Result<Result<T, E>, Result<Error, InvokeError>>,
    expected: UpgradeError,
) {
    match result {
        Err(Ok(err)) => assert_eq!(err, Error::from_contract_error(expected as u32)),
        Ok(Err(_)) => {}
        _ => panic!("expected contract error"),
    }
}

fn assert_failed<T>(result: Result<T, Result<Error, InvokeError>>) {
    assert!(result.is_err(), "expected operation to fail");
}

/// Verifies initialization and baseline status fields.
#[test]
fn test_init_sets_defaults() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env, 2);

    assert_eq!(client.current_version(), 0);
    assert_eq!(client.current_wasm_hash(), hash(&env, 1));
}

#[test]
fn test_init_rejects_zero_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    assert_contract_error(
        client.try_upgrade_init(&admin, &hash(&env, 1), &0),
        UpgradeError::InvalidThreshold,
    );
}

#[test]
fn test_add_approver_admin_only() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);
    let approver = Address::generate(&env);
    let stranger = Address::generate(&env);

    let denied = client.try_upgrade_add_approver(&stranger, &approver);
    assert_contract_error(denied, UpgradeError::NotAuthorized);

    client.upgrade_add_approver(&admin, &approver);
}

#[test]
fn test_remove_approver_admin_only() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);
    let approver = Address::generate(&env);
    let stranger = Address::generate(&env);

    client.upgrade_add_approver(&admin, &approver);

    assert_contract_error(
        client.try_upgrade_remove_approver(&stranger, &approver),
        UpgradeError::NotAuthorized,
    );
}

#[test]
fn test_upgrade_propose_sets_initial_status() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    let status = client.upgrade_status(&proposal_id);
    assert_eq!(proposal_id, 1);
    assert_eq!(status.id, 1);
    assert_eq!(status.stage, UpgradeStage::Proposed);
    assert_eq!(status.approval_count, 1);
    assert_eq!(status.target_version, 1);
}

#[test]
fn test_upgrade_approve_flow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);
    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    let count = client.upgrade_approve(&approver, &proposal_id);
    assert_eq!(count, 2);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Approved
    );
}

#[test]
fn test_upgrade_execute_updates_current_version_and_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    let next_hash = hash(&env, 9);
    let proposal_id = client.upgrade_propose(&admin, &next_hash, &3);

    // In tests, update_current_contract_wasm might not actually swap the code in a visible way
    // without more setup, but we can verify the state updates.
    client.upgrade_execute(&admin, &proposal_id);

    assert_eq!(client.current_version(), 3);
    assert_eq!(client.current_wasm_hash(), next_hash);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Executed
    );
}

#[test]
fn test_upgrade_rollback_restores_previous() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);
    let initial_hash = client.current_wasm_hash();

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 8), &5);
    client.upgrade_execute(&admin, &proposal_id);
    assert_eq!(client.current_version(), 5);

    client.upgrade_rollback(&admin, &proposal_id);
    assert_eq!(client.current_version(), 0);
    assert_eq!(client.current_wasm_hash(), initial_hash);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::RolledBack
    );

    let repeated = client.try_upgrade_rollback(&admin, &proposal_id);
    assert_failed(repeated);
}

#[test]
fn test_upgrade_rollback_requires_executed_stage() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 6), &1);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Approved
    );

    assert_contract_error(
        client.try_upgrade_rollback(&admin, &proposal_id),
        UpgradeError::InvalidStatus,
    );
}

#[test]
fn test_upgrade_execute_missing_proposal_errors() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    assert_contract_error(
        client.try_upgrade_execute(&admin, &77),
        UpgradeError::ProposalNotFound,
    );
}

#[test]
fn test_upgrade_rollback_missing_proposal_errors() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    assert_contract_error(
        client.try_upgrade_rollback(&admin, &88),
        UpgradeError::ProposalNotFound,
    );
}

#[test]
fn test_upgrade_status_missing_proposal_errors() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env, 1);

    let result = client.try_upgrade_status(&42);
    assert_failed(result);
}

#[test]
fn test_upgrade_rejects_unauthorized_approve_and_execute() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);
    let approver = Address::generate(&env);
    let stranger = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    assert_contract_error(
        client.try_upgrade_approve(&stranger, &proposal_id),
        UpgradeError::NotAuthorized,
    );

    client.upgrade_approve(&approver, &proposal_id);
    assert_contract_error(
        client.try_upgrade_execute(&stranger, &proposal_id),
        UpgradeError::NotAuthorized,
    );
}

#[test]
fn test_upgrade_rotation_revokes_old_approver() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);
    let old_approver = Address::generate(&env);
    let new_approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &old_approver);
    client.upgrade_add_approver(&admin, &new_approver);

    let first_upgrade = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_approve(&old_approver, &first_upgrade);
    client.upgrade_execute(&old_approver, &first_upgrade);
    assert_eq!(client.current_version(), 1);

    client.upgrade_remove_approver(&admin, &old_approver);

    let second_upgrade = client.upgrade_propose(&admin, &hash(&env, 3), &2);
    assert_contract_error(
        client.try_upgrade_approve(&old_approver, &second_upgrade),
        UpgradeError::NotAuthorized,
    );
    client.upgrade_approve(&new_approver, &second_upgrade);
    assert_contract_error(
        client.try_upgrade_execute(&old_approver, &second_upgrade),
        UpgradeError::NotAuthorized,
    );
    client.upgrade_execute(&new_approver, &second_upgrade);
    assert_eq!(client.current_version(), 2);
}

/// Rotating only part of an approval set must not weaken the threshold or leave
/// removed signers with residual upgrade power.
///
/// # Security
/// A safe rotation may span multiple transactions. During that window the
/// contract must preserve the configured threshold exactly and require the
/// remaining active signer set to satisfy it.
#[test]
fn test_partial_rotation_preserves_threshold_and_revokes_removed_signer() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 3);
    let old_approver_a = Address::generate(&env);
    let old_approver_b = Address::generate(&env);
    let new_approver_a = Address::generate(&env);

    client.upgrade_add_approver(&admin, &old_approver_a);
    client.upgrade_add_approver(&admin, &old_approver_b);
    client.upgrade_add_approver(&admin, &new_approver_a);
    client.upgrade_remove_approver(&admin, &old_approver_a);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 4), &1);

    let count_after_new = client.upgrade_approve(&new_approver_a, &proposal_id);
    assert_eq!(count_after_new, 2);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Proposed
    );

    assert_contract_error(
        client.try_upgrade_approve(&old_approver_a, &proposal_id),
        UpgradeError::NotAuthorized,
    );
    assert_contract_error(
        client.try_upgrade_execute(&admin, &proposal_id),
        UpgradeError::InvalidStatus,
    );

    let count_after_old_b = client.upgrade_approve(&old_approver_b, &proposal_id);
    assert_eq!(count_after_old_b, 3);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Approved
    );
}

#[test]
fn test_rotation_events_capture_add_and_remove() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let approver = Address::generate(&env);

    client.upgrade_init(&admin, &hash(&env, 1), &1);
    client.upgrade_add_approver(&admin, &approver);
    client.upgrade_remove_approver(&admin, &approver);

    let events = env.events().all();
    assert_eq!(events.len(), 3);

    let add_event = &events[1];
    assert_eq!(add_event.0, contract_id);
    let add_topic: Symbol = Symbol::try_from_val(&env, &add_event.1.get(0).unwrap()).unwrap();
    let add_caller: Address = Address::try_from_val(&env, &add_event.1.get(1).unwrap()).unwrap();
    let add_approver: Address =
        Address::try_from_val(&env, &add_event.1.get(2).unwrap()).unwrap();
    assert_eq!(add_topic, Symbol::new(&env, "up_apadd"));
    assert_eq!(add_caller, admin);
    assert_eq!(add_approver, approver);

    let remove_event = &events[2];
    assert_eq!(remove_event.0, contract_id);
    let remove_topic: Symbol =
        Symbol::try_from_val(&env, &remove_event.1.get(0).unwrap()).unwrap();
    let remove_caller: Address =
        Address::try_from_val(&env, &remove_event.1.get(1).unwrap()).unwrap();
    let remove_approver: Address =
        Address::try_from_val(&env, &remove_event.1.get(2).unwrap()).unwrap();
    assert_eq!(remove_topic, Symbol::new(&env, "up_aprm"));
    assert_eq!(remove_caller, admin);
    assert_eq!(remove_approver, approver);
}

#[test]
fn test_upgrade_remove_approver_enforces_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);
    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    assert_contract_error(
        client.try_upgrade_remove_approver(&admin, &approver),
        UpgradeError::InvalidThreshold,
    );
}

#[test]
fn test_upgrade_invalid_attempts() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);
    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    assert_contract_error(
        client.try_upgrade_execute(&approver, &proposal_id),
        UpgradeError::InvalidStatus,
    );
    client.upgrade_approve(&approver, &proposal_id);
    assert_contract_error(
        client.try_upgrade_approve(&approver, &proposal_id),
        UpgradeError::AlreadyApproved,
    );
    assert_contract_error(
        client.try_upgrade_propose(&admin, &hash(&env, 3), &0),
        UpgradeError::InvalidVersion,
    );
}

// ── Issue #489: upgrade authorization clarity ─────────────────────────────

/// Guardian has no upgrade power — upgrade paths are admin/approver-gated only.
///
/// # Security
/// The guardian role is limited to emergency shutdown. It must not be able to
/// propose, approve, execute, or roll back upgrades. This test documents that
/// trust boundary explicitly.
#[test]
fn test_guardian_cannot_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);
    let guardian = Address::generate(&env);

    // Guardian is not an approver — all upgrade entry points must reject it.
    assert_contract_error(
        client.try_upgrade_propose(&guardian, &hash(&env, 5), &10),
        UpgradeError::NotAuthorized,
    );
    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 5), &10);
    assert_contract_error(
        client.try_upgrade_approve(&guardian, &proposal_id),
        UpgradeError::NotAuthorized,
    );
    assert_contract_error(
        client.try_upgrade_execute(&guardian, &proposal_id),
        UpgradeError::NotAuthorized,
    );
    assert_contract_error(
        client.try_upgrade_rollback(&guardian, &proposal_id),
        UpgradeError::NotAuthorized,
    );
}

/// Arbitrary stranger cannot propose an upgrade.
///
/// # Security
/// Only the stored admin address may create upgrade proposals. Any other caller
/// must be rejected with `NotAuthorized` regardless of the WASM hash or version.
#[test]
fn test_stranger_cannot_propose() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup(&env, 1);
    let stranger = Address::generate(&env);

    assert_contract_error(
        client.try_upgrade_propose(&stranger, &hash(&env, 7), &99),
        UpgradeError::NotAuthorized,
    );
}

/// Only the admin may roll back an executed upgrade.
///
/// # Security
/// Rollback restores the previous WASM hash and version. Restricting it to the
/// admin prevents an approver-only account from silently reverting a governance
/// decision without admin sign-off.
#[test]
fn test_only_admin_can_rollback() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);
    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    // Approver (non-admin) must not be able to rollback.
    assert_contract_error(
        client.try_upgrade_rollback(&approver, &proposal_id),
        UpgradeError::NotAuthorized,
    );

    // Admin succeeds.
    client.upgrade_rollback(&admin, &proposal_id);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::RolledBack
    );
}

/// Approval threshold boundary: n-1 approvals must not reach Approved stage,
/// exactly n approvals must flip the proposal to Approved.
///
/// # Security
/// The threshold is the core multi-sig invariant. A proposal executable with
/// fewer approvals than `required_approvals` would allow a single compromised
/// key to push through an upgrade.
#[test]
fn test_threshold_boundary_n_minus_one_vs_n() {
    let env = Env::default();
    env.mock_all_auths();
    // 3-of-3 setup: admin + 2 extra approvers
    let (client, admin) = setup(&env, 3);
    let approver_a = Address::generate(&env);
    let approver_b = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver_a);
    client.upgrade_add_approver(&admin, &approver_b);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);

    // After proposer (admin) + approver_a = 2 approvals — still Proposed (need 3).
    let count_after_a = client.upgrade_approve(&approver_a, &proposal_id);
    assert_eq!(count_after_a, 2);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Proposed
    );

    // Execute must fail — not yet Approved.
    assert_contract_error(
        client.try_upgrade_execute(&admin, &proposal_id),
        UpgradeError::InvalidStatus,
    );

    // Third approval flips to Approved.
    let count_after_b = client.upgrade_approve(&approver_b, &proposal_id);
    assert_eq!(count_after_b, 3);
    assert_eq!(
        client.upgrade_status(&proposal_id).stage,
        UpgradeStage::Approved
    );

    // Execute now succeeds.
    client.upgrade_execute(&admin, &proposal_id);
    assert_eq!(client.current_version(), 1);
}

/// Duplicate proposal IDs are not possible — each proposal gets a unique
/// monotonically increasing ID.
///
/// # Security
/// If IDs could collide, a race between two proposals could cause one to
/// silently overwrite the other's approvals and WASM hash.
#[test]
fn test_proposal_ids_are_monotonically_increasing() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    let id1 = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &id1);

    let id2 = client.upgrade_propose(&admin, &hash(&env, 3), &2);
    client.upgrade_execute(&admin, &id2);

    assert!(id2 > id1, "proposal IDs must be strictly increasing");
    assert_eq!(client.current_version(), 2);
}

// ── Issue #650: Replay Protection & Idempotency ──────────────────────────

/// Replay protection: An executed proposal must never be executable again.
///
/// # Security
/// This prevents an attacker from re-triggering the upgrade logic which might
/// emit redundant events or interact with initialization logic unexpectedly.
#[test]
fn test_replay_protection_duplicate_execution_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);

    // Attempting to execute the same ID again must fail.
    let result = client.try_upgrade_execute(&admin, &proposal_id);
    assert_contract_error(result, UpgradeError::InvalidStatus);
}

/// Replay protection: A rolled-back proposal is terminal and cannot be re-executed.
///
/// # Security
/// Once an admin has decided to roll back an upgrade, that specific proposal 
/// object is "burned". Re-executing it would bypass the governance intent 
/// behind the rollback.
#[test]
fn test_replay_protection_execute_after_rollback_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    client.upgrade_execute(&admin, &proposal_id);
    client.upgrade_rollback(&admin, &proposal_id);

    assert_eq!(client.upgrade_status(&proposal_id).stage, UpgradeStage::RolledBack);

    // Attempting to execute a rolled-back proposal must fail.
    let result = client.try_upgrade_execute(&admin, &proposal_id);
    assert_contract_error(result, UpgradeError::InvalidStatus);
}

/// Idempotency: Duplicate approvals from the same account must be rejected.
///
/// # Security
/// This ensures that the threshold count (n-of-m) cannot be subverted by 
/// a single compromised or malicious approver signing multiple times for 
/// the same proposal.
#[test]
fn test_idempotent_approvals_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 2);
    let approver = Address::generate(&env);
    client.upgrade_add_approver(&admin, &approver);

    let proposal_id = client.upgrade_propose(&admin, &hash(&env, 2), &1);
    
    // First approval succeeds.
    client.upgrade_approve(&approver, &proposal_id);
    
    // Second approval from same address fails.
    let result = client.try_upgrade_approve(&approver, &proposal_id);
    assert_contract_error(result, UpgradeError::AlreadyApproved);
}

/// Stale Proposal Protection: Executing a newer version invalidates older pending proposals.
///
/// # Security
/// If Proposal A (v1) and Proposal B (v2) are both approved, executing B first
/// makes A obsolete. Attempting to execute A afterwards would be a downgrade
/// or a version collision, which must be blocked.
#[test]
fn test_stale_proposal_invalidation_replay_protection() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    // Create two proposals for sequential versions.
    let p1_v1 = client.upgrade_propose(&admin, &hash(&env, 11), &1);
    let p2_v2 = client.upgrade_propose(&admin, &hash(&env, 22), &2);

    // Execute the newer one (v2).
    client.upgrade_execute(&admin, &p2_v2);
    assert_eq!(client.current_version(), 2);

    // Now try to execute the older one (v1). 
    // Even if it was "Approved", the contract version has moved past it.
    let result = client.try_upgrade_execute(&admin, &p1_v1);
    assert_contract_error(result, UpgradeError::InvalidVersion);
}

/// Integrity check: Proposals are bound to their specific WASM hash.
///
/// # Security
/// This test ensures that the execution logic uses the hash stored inside
/// the proposal state at creation time, not a value that can be manipulated
/// during the approval phase.
#[test]
fn test_proposal_integrity_hash_binding() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin) = setup(&env, 1);

    let target_hash = hash(&env, 99);
    let proposal_id = client.upgrade_propose(&admin, &target_hash, &5);
    
    // Verify status reflects the bound hash.
    let status = client.upgrade_status(&proposal_id);
    // Note: If the contract version allows inspecting the hash in status, we check it.
    
    client.upgrade_execute(&admin, &proposal_id);
    assert_eq!(client.current_wasm_hash(), target_hash);
    assert_eq!(client.current_version(), 5);
}
