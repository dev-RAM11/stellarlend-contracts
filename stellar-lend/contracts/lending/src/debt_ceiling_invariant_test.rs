//! # Debt Ceiling Invariant Tests
//!
//! This module contains two phases of tests for the five debt ceiling defects:
//!
//! ## Phase 1 — Bug Condition Exploration Tests (Task 1)
//! These tests encode the EXPECTED (fixed) behavior. They MUST FAIL on unfixed code
//! to confirm each bug exists, and PASS after the fix is applied.
//!
//! ## Phase 2 — Preservation Tests (Task 2)
//! These tests capture baseline behavior that must remain unchanged after the fix.
//! They MUST PASS on both unfixed and fixed code.
//!
//! ## Security Notes
//! Debt ceilings are a systemic risk control. Any bypass allows unbounded debt
//! issuance, which can lead to protocol insolvency. All five defects below
//! represent real attack surfaces or operational failure modes.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, BytesN, Env,
};

use crate::{
    borrow::BorrowError,
    cross_asset::{AssetParams, CrossAssetError},
    LendingContract, LendingContractClient,
};

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

fn setup_lending(env: &Env) -> (LendingContractClient<'_>, Address, Address, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    client.initialize(&admin, &1_000_000_000, &1000);
    (client, admin, user, asset)
}

/// Build a minimal AssetParams with the given debt_ceiling.
fn asset_params_with_ceiling(env: &Env, ceiling: i128) -> AssetParams {
    AssetParams {
        ltv: 9000,
        liquidation_threshold: 9500,
        price_feed: Address::generate(env),
        debt_ceiling: ceiling,
        is_active: true,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ══════════════════════════════════════════════════════════════════════════════
// PHASE 1 — BUG CONDITION EXPLORATION TESTS
// These tests MUST FAIL on unfixed code (proving each bug exists).
// They MUST PASS after the fix is applied.
// ══════════════════════════════════════════════════════════════════════════════
// ─────────────────────────────────────────────────────────────────────────────

/// Defect 1 — Zero-ceiling semantics mismatch in cross_asset.rs
///
/// isBugCondition: path == CROSS_ASSET AND asset_params[asset].debt_ceiling == 0 AND amount > 0
///
/// UNFIXED: borrow_asset returns DebtCeilingReached when debt_ceiling == 0
/// FIXED:   borrow_asset returns Ok(()) — 0 means unlimited, consistent with borrow.rs
///
/// Security: A newly configured asset with debt_ceiling = 0 should be usable.
/// Treating 0 as "zero capacity" silently blocks all borrows on new assets,
/// which is an operational failure mode that could be exploited to grief users.
#[test]
fn test_defect1_zero_ceiling_cross_asset_should_allow_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_admin(&admin);

    // Configure asset with debt_ceiling = 0 (should mean unlimited)
    let params = asset_params_with_ceiling(&env, 0);
    client.set_asset_params(&asset, &params);

    // Deposit collateral so health factor is satisfied
    client.deposit_collateral_asset(&user, &asset, &100_000);

    // EXPECTED (fixed): Ok(()) — zero ceiling means unlimited
    // ACTUAL (unfixed): DebtCeilingReached — zero is treated as a real ceiling of 0
    let result = client.try_borrow_asset(&user, &asset, &1000);
    assert_eq!(
        result,
        Ok(Ok(())),
        "Defect 1: debt_ceiling=0 should mean unlimited, not zero capacity"
    );
}

/// Defect 2 — Interest accrual bypass in borrow.rs
///
/// isBugCondition: BorrowTotalDebt + accrued_interest > BorrowDebtCeiling
///                 AND BorrowTotalDebt <= BorrowDebtCeiling
///
/// UNFIXED: borrow accepts new principal even when principal + interest > ceiling
/// FIXED:   borrow returns DebtCeilingReached when total outstanding > ceiling
///
/// Security: Interest accrual silently inflates total debt above the ceiling.
/// This allows the protocol to issue more debt than the ceiling permits,
/// undermining the systemic risk cap.
#[test]
fn test_defect2_interest_accrual_should_enforce_ceiling() {
    let env = Env::default();
    env.mock_all_auths();

    env.ledger().with_mut(|li| {
        li.timestamp = 1_000_000;
    });

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    // Ceiling = 100_000; min_borrow = 100
    client.initialize(&admin, &100_000, &100);

    // Borrow 99_000 principal (within ceiling)
    client.borrow(&user, &asset, &99_000, &collateral_asset, &200_000);

    // Advance 1 year — 5% interest on 99_000 = ~4_950 accrued
    // Total outstanding = 99_000 + 4_950 = 103_950 > 100_000 ceiling
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000_000 + 31_536_000;
    });

    // EXPECTED (fixed): DebtCeilingReached — total outstanding exceeds ceiling
    // ACTUAL (unfixed): Ok(()) — only principal is checked, interest is ignored
    let result = client.try_borrow(&user, &asset, &500, &collateral_asset, &1000);
    assert_eq!(
        result,
        Err(Ok(BorrowError::DebtCeilingReached)),
        "Defect 2: ceiling must be enforced against principal + accrued interest"
    );
}

/// Defect 3 — Upgrade/migration data loss in borrow.rs
///
/// isBugCondition: upgrade_executed AND BorrowDebtCeiling not explicitly re-initialised
///
/// UNFIXED: get_debt_ceiling returns i128::MAX after upgrade (instance storage cleared)
/// FIXED:   get_debt_ceiling returns the pre-upgrade configured value
///
/// Security: After an upgrade, the ceiling silently becomes i128::MAX, effectively
/// disabling the systemic risk cap. An attacker who can trigger an upgrade (or
/// observe one) can borrow without limit in the window before re-initialisation.
///
/// Test approach: Simulate the post-upgrade state by registering a fresh contract
/// instance where BorrowDebtCeiling has never been written to instance storage
/// (equivalent to instance storage being cleared by an upgrade). On unfixed code,
/// get_debt_ceiling() returns i128::MAX via unwrap_or(i128::MAX), so a borrow of
/// 600_000 that should be rejected (> 500_000) is instead accepted.
#[test]
fn test_defect3_ceiling_must_survive_upgrade() {
    let env = Env::default();
    env.mock_all_auths();

    // Contract A: properly initialised with ceiling = 500_000
    let contract_id_a = env.register(LendingContract, ());
    let client_a = LendingContractClient::new(&env, &contract_id_a);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client_a.initialize(&admin, &500_000, &1000);

    // Pre-upgrade: ceiling is enforced at 500_000
    let result_pre = client_a.try_borrow(&user, &asset, &600_000, &collateral_asset, &1_200_000);
    assert_eq!(
        result_pre,
        Err(Ok(BorrowError::DebtCeilingReached)),
        "Pre-upgrade: ceiling must be enforced at 500_000"
    );

    // Simulate post-upgrade state: register a fresh contract where BorrowDebtCeiling
    // is absent from instance storage (equivalent to instance storage being cleared by upgrade).
    // The borrow function does not require admin auth, so it works on a fresh contract.
    // On unfixed code, get_debt_ceiling() returns i128::MAX via unwrap_or(i128::MAX).
    let contract_id_b = env.register(LendingContract, ());
    let client_b = LendingContractClient::new(&env, &contract_id_b);
    // Intentionally do NOT call initialize() — BorrowDebtCeiling key is absent,
    // simulating the post-upgrade state where instance storage has been cleared.
    // min_borrow defaults to 1000 (from get_min_borrow_amount unwrap_or(1000)).

    // EXPECTED (fixed): ceiling still enforced at 500_000 post-upgrade (stored in persistent storage)
    // ACTUAL (unfixed): ceiling is i128::MAX (absent key fallback), so 600_000 borrow is accepted
    //
    // On unfixed code: Ok(Ok(())) — ceiling = i128::MAX, borrow accepted (BUG)
    // On fixed code: Err(Ok(DebtCeilingReached)) — ceiling = 500_000 preserved (FIXED)
    let result_post = client_b.try_borrow(&user, &asset, &600_000, &collateral_asset, &1_200_000);
    assert_eq!(
        result_post,
        Err(Ok(BorrowError::DebtCeilingReached)),
        "Defect 3: BorrowDebtCeiling must survive upgrade — must not fall back to i128::MAX"
    );
}

/// Defect 4 — Dual-counter gap between borrow.rs and cross_asset.rs
///
/// isBugCondition: amount_A + amount_B > intended_protocol_ceiling
///                 AND each_path_passes_independently
///
/// UNFIXED: both paths accept borrows independently; combined total exceeds ceiling
/// FIXED:   each path enforces its own ceiling correctly; combined total is bounded
///
/// Security: Independent counters allow a user to borrow up to ceiling_A via the
/// simplified path and up to ceiling_B via the cross-asset path simultaneously,
/// potentially doubling the intended protocol-wide exposure.
///
/// Test approach: Set both ceilings to 1_000_000. Borrow 900_000 via simplified path,
/// then borrow 200_000 via cross-asset path. The combined total (1_100_000) exceeds
/// the intended 1_000_000 protocol ceiling. On unfixed code, both borrows succeed
/// because each path only checks its own independent counter.
/// On fixed code, the second borrow should be rejected.
#[test]
fn test_defect4_each_path_independently_enforces_its_ceiling() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    // Simplified path ceiling = 1_000_000
    client.initialize(&admin, &1_000_000, &1000);
    client.initialize_admin(&admin);

    // Cross-asset path ceiling = 1_000_000 (same asset, same intended protocol ceiling)
    let params = asset_params_with_ceiling(&env, 1_000_000);
    client.set_asset_params(&asset, &params);

    // Borrow 900_000 via simplified path (within its own ceiling)
    client.borrow(&user, &asset, &900_000, &collateral_asset, &2_000_000);

    // Deposit collateral for cross-asset path
    client.deposit_collateral_asset(&user, &asset, &2_000_000);

    // Attempt 200_000 via cross-asset path
    // Combined total = 900_000 + 200_000 = 1_100_000 > 1_000_000 intended ceiling
    // EXPECTED (fixed): DebtCeilingReached — combined total exceeds intended ceiling
    // ACTUAL (unfixed): Ok(()) — cross-asset path only checks TotalAssetDebt (0), not BorrowTotalDebt
    let result_cross = client.try_borrow_asset(&user, &asset, &200_000);
    assert_eq!(
        result_cross,
        Err(Ok(CrossAssetError::DebtCeilingReached)),
        "Defect 4: combined borrows across both paths must not exceed the intended protocol ceiling"
    );
}

/// Defect 5 — Check-then-update ordering in cross_asset.rs
///
/// isBugCondition: two_sequential_calls AND each_reads_stale_total_before_storage_update
///
/// UNFIXED: storage update happens after health check; sequential calls can each
///          read the pre-update total and both pass the ceiling check
/// FIXED:   storage update happens immediately after ceiling check; second call
///          reads the updated total and is correctly rejected
///
/// Security: In Soroban, each transaction is atomic, but sequential transactions
/// in the same ledger each read the committed state from the prior transaction.
/// Moving the storage update before the health check ensures the counter is
/// committed before any subsequent transaction can read it.
///
/// NOTE: This bug is a concurrency issue that cannot be demonstrated in sequential
/// unit tests. In sequential execution, each call sees the committed state from
/// the prior call, so the ceiling is correctly enforced. The bug only manifests
/// when two transactions are in the same ledger and both read the pre-update total.
///
/// Test approach: Verify that after two sequential borrows that together exceed the
/// ceiling, the second borrow is correctly rejected. This validates the expected
/// behavior and serves as a regression test for the fix.
///
/// The test encodes the EXPECTED behavior: after the fix, TotalAssetDebt is updated
/// atomically with the ceiling check, ensuring no sequential bypass is possible.
/// On unfixed code, this test PASSES because sequential calls already work correctly.
/// The fix ensures this behavior is preserved even in concurrent scenarios.
#[test]
fn test_defect5_sequential_borrows_cannot_exceed_ceiling() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_admin(&admin);

    // Ceiling = 100_000; current total = 50_000 (after first borrow)
    let params = asset_params_with_ceiling(&env, 100_000);
    client.set_asset_params(&asset, &params);

    // Pre-load: borrow 50_000 to set TotalAssetDebt = 50_000
    client.deposit_collateral_asset(&user1, &asset, &2_000_000);
    client.borrow_asset(&user1, &asset, &50_000);

    // Now two sequential calls each for 60_000 against ceiling of 100_000
    // First call: 50_000 + 60_000 = 110_000 > 100_000 — must be rejected
    client.deposit_collateral_asset(&user2, &asset, &2_000_000);
    let result1 = client.try_borrow_asset(&user2, &asset, &60_000);
    assert_eq!(
        result1,
        Err(Ok(CrossAssetError::DebtCeilingReached)),
        "Defect 5: first sequential call must be rejected when it would exceed ceiling"
    );

    // Confirm TotalAssetDebt is still 50_000 (not corrupted by the rejected call)
    // Second call for 40_000 should succeed (50_000 + 40_000 = 90_000 <= 100_000)
    let result2 = client.try_borrow_asset(&user2, &asset, &40_000);
    assert_eq!(
        result2,
        Ok(Ok(())),
        "Defect 5: valid borrow within ceiling must succeed after rejected call"
    );

    // Third call for 20_000 should be rejected (90_000 + 20_000 = 110_000 > 100_000)
    let result3 = client.try_borrow_asset(&user2, &asset, &20_000);
    assert_eq!(
        result3,
        Err(Ok(CrossAssetError::DebtCeilingReached)),
        "Defect 5: storage update must be committed so subsequent calls see updated total"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ══════════════════════════════════════════════════════════════════════════════
// PHASE 2 — PRESERVATION TESTS
// These tests capture baseline behavior that must remain unchanged.
// They MUST PASS on both unfixed and fixed code.
// ══════════════════════════════════════════════════════════════════════════════
// ─────────────────────────────────────────────────────────────────────────────

/// Preservation 3.1 — Valid borrow below ceiling is accepted
///
/// For any (amount, ceiling) where ceiling > 0, amount > 0, new_total <= ceiling,
/// and collateral ratio is met — borrow succeeds and BorrowTotalDebt increases.
#[test]
fn test_preservation_valid_borrow_below_ceiling_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_lending(&env);
    let collateral_asset = Address::generate(&env);

    // Ceiling = 1_000_000_000 (from initialize), borrow 10_000 — well within ceiling
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 10_000);
    assert_eq!(debt.interest_accrued, 0);
}

/// Preservation 3.2 — Borrow exactly at ceiling is accepted (inclusive upper bound)
#[test]
fn test_preservation_borrow_exactly_at_ceiling_accepted() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    // Ceiling = 50_000
    client.initialize(&admin, &50_000, &1000);

    // Borrow exactly 50_000 — must be accepted (ceiling is inclusive)
    client.borrow(&user, &asset, &50_000, &collateral_asset, &100_000);

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 50_000);
}

/// Preservation 3.3 — Borrow exceeding ceiling returns DebtCeilingReached
#[test]
fn test_preservation_borrow_exceeding_ceiling_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &50_000, &1000);

    let result = client.try_borrow(&user, &asset, &100_000, &collateral_asset, &200_000);
    assert_eq!(result, Err(Ok(BorrowError::DebtCeilingReached)));
}

/// Preservation 3.4 — Protocol pause returns ProtocolPaused before ceiling check
#[test]
fn test_preservation_pause_check_precedes_ceiling_check() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    // Ceiling = 50_000 (would also reject, but pause must come first)
    client.initialize(&admin, &50_000, &1000);

    use crate::pause::PauseType;
    client.set_pause(&admin, &PauseType::Borrow, &true);

    let result = client.try_borrow(&user, &asset, &100_000, &collateral_asset, &200_000);
    assert_eq!(result, Err(Ok(BorrowError::ProtocolPaused)));
}

/// Preservation 3.5 — Repayment reduces BorrowTotalDebt, freeing ceiling capacity
#[test]
fn test_preservation_repayment_frees_ceiling_capacity() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    // Ceiling = 50_000
    client.initialize(&admin, &50_000, &1000);

    // Borrow 50_000 (at ceiling)
    client.borrow(&user, &asset, &50_000, &collateral_asset, &100_000);

    // Another borrow must be rejected (at ceiling)
    let result_at_ceiling = client.try_borrow(&user, &asset, &1000, &collateral_asset, &2000);
    assert_eq!(result_at_ceiling, Err(Ok(BorrowError::DebtCeilingReached)));

    // Repay 10_000 — frees 10_000 of ceiling capacity
    client.repay(&user, &asset, &10_000);

    // Now a borrow of 10_000 must succeed
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 50_000); // 50k - 10k repaid + 10k new borrow
}

/// Preservation 3.6 — InsufficientCollateral returned regardless of ceiling headroom
#[test]
fn test_preservation_insufficient_collateral_regardless_of_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset) = setup_lending(&env);
    let collateral_asset = Address::generate(&env);

    // Ceiling is 1_000_000_000 (plenty of headroom), but collateral is insufficient
    let result = client.try_borrow(&user, &asset, &10_000, &collateral_asset, &1_000);
    assert_eq!(result, Err(Ok(BorrowError::InsufficientCollateral)));
}

/// Preservation 3.7 — Upgrade rollback restores pre-upgrade version and state
#[test]
fn test_preservation_upgrade_rollback_restores_state() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);

    let hash_v1 = BytesN::from_array(&env, &[1u8; 32]);
    let hash_v2 = BytesN::from_array(&env, &[2u8; 32]);
    client.upgrade_init(&admin, &hash_v1, &1);

    assert_eq!(client.current_version(), 0);

    let proposal_id = client.upgrade_propose(&admin, &hash_v2, &1);
    client.upgrade_execute(&admin, &proposal_id);
    assert_eq!(client.current_version(), 1);

    client.upgrade_rollback(&admin, &proposal_id);
    assert_eq!(client.current_version(), 0);
    assert_eq!(client.current_wasm_hash(), hash_v1);
}

/// Preservation 3.8 — Interest uses ceiling-up rounding and is reflected in get_user_debt
#[test]
fn test_preservation_interest_rounds_up_and_reflected_in_get_user_debt() {
    let env = Env::default();
    env.mock_all_auths();

    env.ledger().with_mut(|li| {
        li.timestamp = 10_000;
    });

    let (client, _admin, user, asset) = setup_lending(&env);
    let collateral_asset = Address::generate(&env);

    client.borrow(&user, &asset, &100_000, &collateral_asset, &200_000);

    // Advance 1 second — fractional interest rounds up to 1
    env.ledger().with_mut(|li| {
        li.timestamp = 10_001;
    });

    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 100_000);
    // Ceiling-up rounding: even 1 second of interest on 100_000 at 5%/year rounds up to 1
    assert_eq!(debt.interest_accrued, 1, "Interest must round up for protocol safety");
}

/// Preservation — Cross-asset valid borrow below ceiling is accepted
#[test]
fn test_preservation_cross_asset_valid_borrow_accepted() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_admin(&admin);

    // Ceiling = 1_000_000; borrow 500_000 — within ceiling
    let params = asset_params_with_ceiling(&env, 1_000_000);
    client.set_asset_params(&asset, &params);

    client.deposit_collateral_asset(&user, &asset, &2_000_000);
    client.borrow_asset(&user, &asset, &500_000);

    let summary = client.get_cross_position_summary(&user);
    assert!(summary.total_debt_usd > 0);
}

/// Preservation — Cross-asset repayment frees ceiling capacity
#[test]
fn test_preservation_cross_asset_repayment_frees_ceiling() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_admin(&admin);

    let params = asset_params_with_ceiling(&env, 100_000);
    client.set_asset_params(&asset, &params);

    client.deposit_collateral_asset(&user, &asset, &2_000_000);

    // Borrow 100_000 (at ceiling)
    client.borrow_asset(&user, &asset, &100_000);

    // Another borrow must be rejected
    let result = client.try_borrow_asset(&user, &asset, &1000);
    assert_eq!(result, Err(Ok(CrossAssetError::DebtCeilingReached)));

    // Repay 50_000
    client.repay_asset(&user, &asset, &50_000);

    // Now 50_000 borrow must succeed
    client.borrow_asset(&user, &asset, &50_000);
}
