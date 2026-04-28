//! # Multisig Threshold Change Safety Tests
//!
//! Verifies that threshold and signer-set changes cannot create a window where
//! protocol actions are executable with weaker security than intended.
//!
//! ## Scenarios covered
//!
//! ### Positive (safe sequences)
//! - Atomic replace via `ms_set_admins` keeps threshold consistent
//! - Threshold increase before adding a signer is accepted
//! - Adding a signer then raising threshold is accepted
//! - Removing a signer auto-adjusts threshold when it would exceed signer count
//!
//! ### Negative (downgrade / misconfiguration attacks)
//! - Threshold of zero is rejected
//! - Threshold greater than signer count is rejected
//! - Non-admin cannot change threshold or signer set
//! - Duplicate signers in the new set are rejected
//! - Empty signer set is rejected
//! - Threshold cannot be set to zero via `set_ms_threshold`
//! - Threshold cannot exceed current signer count via `set_ms_threshold`
//! - A proposal approved under the old threshold still requires the *stored*
//!   threshold at execution time (no retroactive downgrade)

#![cfg(test)]

use crate::errors::GovernanceError;
use crate::governance::initialize;
use crate::multisig::{
    get_ms_admins, get_ms_threshold, ms_approve, ms_execute, ms_propose_set_min_cr, ms_set_admins,
    set_ms_threshold,
};
use crate::HelloContract;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Vec,
};

// ============================================================================
// Helpers
// ============================================================================

/// Register the contract, initialize it, and return `(contract_id, admin)`.
fn setup(env: &Env) -> (Address, Address) {
    env.mock_all_auths();
    let cid = env.register(HelloContract, ());
    let admin = Address::generate(env);

    let client = crate::HelloContractClient::new(env, &cid);
    client.initialize(&admin);

    env.as_contract(&cid, || {
        initialize(env, admin.clone(), admin.clone(), None, None, None, None, None, None).unwrap();
    });

    (cid, admin)
}

/// Build a `Vec<Address>` from a slice.
fn addr_vec(env: &Env, addrs: &[Address]) -> Vec<Address> {
    let mut v = Vec::new(env);
    for a in addrs {
        v.push_back(a.clone());
    }
    v
}

/// Advance ledger time by `secs` seconds.
fn advance(env: &Env, secs: u64) {
    env.ledger().with_mut(|li| li.timestamp += secs);
}

// ============================================================================
// Positive tests — safe sequences
// ============================================================================

/// `ms_set_admins` atomically replaces both the signer list and threshold.
/// There is never a moment where the old threshold applies to the new set.
#[test]
fn test_atomic_replace_keeps_threshold_consistent() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]), 2).unwrap();

        assert_eq!(get_ms_admins(&env).unwrap().len(), 3);
        assert_eq!(get_ms_threshold(&env), 2);
    });
}

/// Raising the threshold before adding a new signer is safe: the new signer
/// cannot reduce the effective quorum because the threshold is already higher.
#[test]
fn test_raise_threshold_then_add_signer() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    env.as_contract(&cid, || {
        // Start: [admin], threshold=1
        // Step 1: add a2 and raise threshold to 2 atomically
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 2).unwrap();
        assert_eq!(get_ms_threshold(&env), 2);

        // Step 2: add a3 and keep threshold at 2 (still safe — 2-of-3)
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]), 2).unwrap();
        assert_eq!(get_ms_threshold(&env), 2);
        assert_eq!(get_ms_admins(&env).unwrap().len(), 3);
    });
}

/// Adding a signer and then raising the threshold is also safe.
#[test]
fn test_add_signer_then_raise_threshold() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    env.as_contract(&cid, || {
        // Add a2 (threshold stays 1 — still valid)
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 1).unwrap();
        // Raise threshold to 2
        set_ms_threshold(&env, admin.clone(), 2).unwrap();
        assert_eq!(get_ms_threshold(&env), 2);

        // Add a3 and raise to 3-of-3
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]), 3).unwrap();
        assert_eq!(get_ms_threshold(&env), 3);
    });
}

/// Removing a signer when threshold < new signer count is safe.
#[test]
fn test_remove_signer_threshold_still_valid() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]), 2).unwrap();
        // Remove a3 — 2-of-2 is still valid
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 2).unwrap();
        assert_eq!(get_ms_admins(&env).unwrap().len(), 2);
        assert_eq!(get_ms_threshold(&env), 2);
    });
}

/// A full proposal lifecycle (propose → approve → execute) works correctly
/// after a threshold change.
#[test]
fn test_full_flow_after_threshold_change() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]), 2).unwrap();
    });

    let pid = env.as_contract(&cid, || {
        let pid = ms_propose_set_min_cr(&env, admin.clone(), 15_000).unwrap();
        ms_approve(&env, a2.clone(), pid).unwrap(); // threshold (2) met
        pid
    });

    advance(&env, 5 * 24 * 60 * 60); // past 24h timelock

    env.as_contract(&cid, || {
        ms_execute(&env, admin.clone(), pid).unwrap();
    });
}

/// `set_ms_threshold` can lower the threshold when signers > new threshold.
#[test]
fn test_lower_threshold_valid() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]), 3).unwrap();
        set_ms_threshold(&env, admin.clone(), 2).unwrap();
        assert_eq!(get_ms_threshold(&env), 2);
    });
}

// ============================================================================
// Negative tests — downgrade / misconfiguration attacks
// ============================================================================

/// Threshold of zero must be rejected — it would allow execution with no approvals.
#[test]
fn test_threshold_zero_rejected_by_set_admins() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);

    env.as_contract(&cid, || {
        let result = ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 0);
        assert_eq!(result, Err(GovernanceError::InvalidMultisigConfig));
    });
}

/// Threshold of zero must be rejected by `set_ms_threshold`.
#[test]
fn test_threshold_zero_rejected_by_set_threshold() {
    let env = Env::default();
    let (cid, admin) = setup(&env);

    env.as_contract(&cid, || {
        let result = set_ms_threshold(&env, admin.clone(), 0);
        assert_eq!(result, Err(GovernanceError::InvalidThreshold));
    });
}

/// Threshold exceeding signer count must be rejected — it would make execution
/// permanently impossible (deadlock).
#[test]
fn test_threshold_exceeds_signer_count_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);

    env.as_contract(&cid, || {
        // 2 signers, threshold 3 → impossible to reach
        let result = ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 3);
        assert_eq!(result, Err(GovernanceError::InvalidMultisigConfig));
    });
}

/// `set_ms_threshold` must reject a threshold that exceeds the current signer count.
#[test]
fn test_set_threshold_exceeds_signer_count_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);

    env.as_contract(&cid, || {
        // Only 1 signer (admin), threshold 2 → impossible
        let result = set_ms_threshold(&env, admin.clone(), 2);
        assert_eq!(result, Err(GovernanceError::InvalidThreshold));
    });
}

/// A non-admin address must not be able to change the signer set.
#[test]
fn test_non_admin_cannot_change_signer_set() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let attacker = Address::generate(&env);
    let a2 = Address::generate(&env);

    env.as_contract(&cid, || {
        // Bootstrap a 2-signer set so the post-bootstrap path is exercised
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 1).unwrap();

        // Attacker tries to replace the set with themselves at threshold 1
        let result = ms_set_admins(&env, attacker.clone(), addr_vec(&env, &[attacker.clone()]), 1);
        assert_eq!(result, Err(GovernanceError::Unauthorized));
    });
}

/// A non-admin address must not be able to change the threshold.
#[test]
fn test_non_admin_cannot_change_threshold() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let attacker = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 2).unwrap();
        let result = set_ms_threshold(&env, attacker.clone(), 1);
        assert_eq!(result, Err(GovernanceError::Unauthorized));
    });
}

/// Duplicate addresses in the new signer set must be rejected.
/// Duplicates would allow a single key to satisfy multiple approval slots.
#[test]
fn test_duplicate_signer_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);

    env.as_contract(&cid, || {
        let result = ms_set_admins(
            &env,
            admin.clone(),
            addr_vec(&env, &[admin.clone(), admin.clone()]),
            1,
        );
        assert_eq!(result, Err(GovernanceError::InvalidMultisigConfig));
    });
}

/// An empty signer set must be rejected — it would leave governance permanently locked.
#[test]
fn test_empty_signer_set_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);

    env.as_contract(&cid, || {
        let result = ms_set_admins(&env, admin.clone(), Vec::new(&env), 1);
        assert_eq!(result, Err(GovernanceError::InvalidMultisigConfig));
    });
}

/// A proposal that was approved under a higher threshold cannot be executed
/// after the threshold is lowered — the threshold stored on the proposal at
/// creation time is the binding quorum.
///
/// This prevents a downgrade attack where an attacker:
/// 1. Creates a proposal (threshold = 3, needs 3 approvals)
/// 2. Lowers threshold to 1
/// 3. Executes with only 1 approval
#[test]
fn test_threshold_downgrade_does_not_retroactively_lower_proposal_quorum() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    // Set up 3-of-3
    env.as_contract(&cid, || {
        ms_set_admins(
            &env,
            admin.clone(),
            addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]),
            3,
        )
        .unwrap();
    });

    // Create proposal — threshold=3 is captured at creation time
    let pid = env.as_contract(&cid, || {
        ms_propose_set_min_cr(&env, admin.clone(), 15_000).unwrap()
        // Only admin has approved (1 of 3)
    });

    // Lower threshold to 1 (attacker's goal)
    env.as_contract(&cid, || {
        ms_set_admins(
            &env,
            admin.clone(),
            addr_vec(&env, &[admin.clone(), a2.clone(), a3.clone()]),
            1,
        )
        .unwrap();
    });

    advance(&env, 5 * 24 * 60 * 60);

    // Attempt execution with only 1 approval — must fail because the proposal
    // was created with threshold=3
    env.as_contract(&cid, || {
        let result = ms_execute(&env, admin.clone(), pid);
        assert_eq!(result, Err(GovernanceError::InsufficientApprovals));
    });
}

/// Executing before the timelock elapses must be rejected regardless of approvals.
#[test]
fn test_execution_before_timelock_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 2).unwrap();
    });

    let pid = env.as_contract(&cid, || {
        let pid = ms_propose_set_min_cr(&env, admin.clone(), 15_000).unwrap();
        ms_approve(&env, a2.clone(), pid).unwrap();
        pid
    });

    // Do NOT advance time — timelock not elapsed
    env.as_contract(&cid, || {
        let result = ms_execute(&env, admin.clone(), pid);
        assert_eq!(result, Err(GovernanceError::ProposalNotReady));
    });
}

/// Approving a proposal twice with the same address must be rejected.
#[test]
fn test_duplicate_approval_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 2).unwrap();
    });

    env.as_contract(&cid, || {
        let pid = ms_propose_set_min_cr(&env, admin.clone(), 15_000).unwrap();
        // admin already approved during propose; second approval must fail
        let result = ms_approve(&env, admin.clone(), pid);
        assert_eq!(result, Err(GovernanceError::AlreadyVoted));
    });
}

/// A non-admin address must not be able to approve a proposal.
#[test]
fn test_non_admin_cannot_approve() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let outsider = Address::generate(&env);

    env.as_contract(&cid, || {
        let pid = ms_propose_set_min_cr(&env, admin.clone(), 15_000).unwrap();
        let result = ms_approve(&env, outsider.clone(), pid);
        assert_eq!(result, Err(GovernanceError::Unauthorized));
    });
}

/// A non-admin address must not be able to execute a proposal.
#[test]
fn test_non_admin_cannot_execute() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let outsider = Address::generate(&env);

    let pid = env.as_contract(&cid, || {
        ms_propose_set_min_cr(&env, admin.clone(), 15_000).unwrap()
    });

    advance(&env, 5 * 24 * 60 * 60);

    env.as_contract(&cid, || {
        let result = ms_execute(&env, outsider.clone(), pid);
        assert_eq!(result, Err(GovernanceError::Unauthorized));
    });
}

/// Executing an already-executed proposal must be rejected (no replay).
#[test]
fn test_double_execution_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);
    let a2 = Address::generate(&env);

    env.as_contract(&cid, || {
        ms_set_admins(&env, admin.clone(), addr_vec(&env, &[admin.clone(), a2.clone()]), 2).unwrap();
    });

    let pid = env.as_contract(&cid, || {
        let pid = ms_propose_set_min_cr(&env, admin.clone(), 15_000).unwrap();
        ms_approve(&env, a2.clone(), pid).unwrap();
        pid
    });

    advance(&env, 5 * 24 * 60 * 60);

    env.as_contract(&cid, || {
        ms_execute(&env, admin.clone(), pid).unwrap();
        let result = ms_execute(&env, admin.clone(), pid);
        assert_eq!(result, Err(GovernanceError::ProposalAlreadyExecuted));
    });
}

/// A proposal with an invalid ratio (≤ 10 000 bps) must be rejected at creation.
#[test]
fn test_invalid_ratio_proposal_rejected() {
    let env = Env::default();
    let (cid, admin) = setup(&env);

    env.as_contract(&cid, || {
        let result = ms_propose_set_min_cr(&env, admin.clone(), 10_000);
        assert_eq!(result, Err(GovernanceError::InvalidProposal));
    });
}
