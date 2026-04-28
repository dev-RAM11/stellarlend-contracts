//! # Full Lifecycle Tests Across Pause and Emergency States
//!
//! Validates the complete deposit → borrow → repay → withdraw lifecycle under:
//! - Granular per-operation pauses (mid-flow)
//! - Global (`All`) protocol pause (mid-flow)
//! - Emergency Shutdown state (hard stop)
//! - Emergency Recovery state (unwind-only path)
//! - Full recovery back to Normal (all ops re-enabled)
//! - Multi-cycle shutdown/recovery with partial pauses inside recovery
//!
//! ## Security Notes
//!
//! | State        | Deposit | Borrow | Repay | Withdraw | Flash | Liquidate |
//! |--------------|---------|--------|-------|----------|-------|-----------|
//! | Normal       | ✓       | ✓      | ✓     | ✓        | ✓     | ✓         |
//! | GranularPause| varies  | varies | varies| varies   | varies| varies    |
//! | Shutdown     | ✗       | ✗      | ✗     | ✗        | ✗     | ✗         |
//! | Recovery     | ✗       | ✗      | ✓     | ✓        | ✗     | ✗         |
//! | Normal (post)| ✓       | ✓      | ✓     | ✓        | ✓     | ✓         |
//!
//! During an incident:
//! 1. Guardian triggers `emergency_shutdown` immediately.
//! 2. Admin analyses and calls `start_recovery` when safe.
//! 3. Users call `repay` / `withdraw` to unwind positions.
//! 4. Admin calls `complete_recovery` when all positions are clear.
//!
//! See `LIFECYCLE_SECURITY_NOTES.md` for the full operational runbook.

use super::*;
use crate::deposit::DepositError;
use crate::withdraw::WithdrawError;
use soroban_sdk::{testutils::Address as _, Address, Env};

// ─────────────────────────────────────────────────────────────────────────────
// Setup helper
// ─────────────────────────────────────────────────────────────────────────────

/// Common test setup: registers contract, initialises protocol, returns useful
/// addresses and a ready-to-use client.
fn setup_lifecycle(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address, // admin
    Address, // guardian
    Address, // user
    Address, // asset
    Address, // collateral_asset
) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);

    let admin = Address::generate(env);
    let guardian = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    let collateral_asset = Address::generate(env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.set_guardian(&admin, &guardian);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    (client, admin, guardian, user, asset, collateral_asset)
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: Granular pause interrupts the lifecycle mid-flow, then resumes
// ─────────────────────────────────────────────────────────────────────────────

/// A borrow-specific pause raised after deposit must block borrow, but all
/// other in-flight operations (repay, withdraw) remain unaffected once unpaused.
#[test]
fn test_deposit_borrow_granular_pause_mid_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _guardian, user, asset, collateral_asset) = setup_lifecycle(&env);

    // ── Phase 1: deposit succeeds in Normal state ──────────────────────────
    client.deposit(&user, &asset, &50_000);
    assert_eq!(client.get_user_collateral_deposit(&user, &asset).amount, 50_000);

    // ── Phase 2: pause Borrow mid-lifecycle ───────────────────────────────
    client.set_pause(&admin, &PauseType::Borrow, &true);
    assert!(client.get_pause_state(&PauseType::Borrow));

    let result = client.try_borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert_eq!(result, Err(Ok(BorrowError::ProtocolPaused)));

    // Deposit is still open (not paused)
    client.deposit(&user, &asset, &5_000);

    // ── Phase 3: unpause Borrow, complete lifecycle ────────────────────────
    client.set_pause(&admin, &PauseType::Borrow, &false);
    assert!(!client.get_pause_state(&PauseType::Borrow));

    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 10_000);

    client.repay(&user, &asset, &5_000);
    let debt_after = client.get_user_debt(&user);
    assert!(debt_after.borrowed_amount <= 10_000);

    client.withdraw(&user, &asset, &5_000);

    // Protocol remains Normal throughout
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: Global "All" pause blocks every operation mid-lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// After deposit + borrow, a global All pause must block repay and withdraw.
/// Lifting it must re-enable both.
#[test]
fn test_deposit_borrow_global_pause_mid_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _guardian, user, asset, collateral_asset) = setup_lifecycle(&env);

    // ── Phase 1: open position ─────────────────────────────────────────────
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert_eq!(client.get_user_debt(&user).borrowed_amount, 10_000);

    // ── Phase 2: global pause ─────────────────────────────────────────────
    client.set_pause(&admin, &PauseType::All, &true);
    assert!(client.get_pause_state(&PauseType::All));

    assert_eq!(
        client.try_repay(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw(&user, &asset, &1_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral_asset, &2_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );

    // ── Phase 3: lift global pause, complete lifecycle ─────────────────────
    client.set_pause(&admin, &PauseType::All, &false);

    client.repay(&user, &asset, &5_000);
    client.withdraw(&user, &asset, &5_000);

    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: Emergency shutdown triggered mid-lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// Shutdown after deposit + borrow must block ALL operations.
/// Recovery opens only repay/withdraw. complete_recovery restores Normal.
#[test]
fn test_shutdown_mid_lifecycle_blocks_new_risk() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, guardian, user, asset, collateral_asset) = setup_lifecycle(&env);

    // ── Phase 1: open position ─────────────────────────────────────────────
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // ── Phase 2: emergency shutdown ────────────────────────────────────────
    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    // Every operation is blocked
    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral_asset, &2_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_repay(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw(&user, &asset, &1_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );
    assert_eq!(
        client.try_liquidate(&user, &user, &asset, &collateral_asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // ── Phase 3: start recovery ────────────────────────────────────────────
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // Unwind path is now open
    client.repay(&user, &asset, &5_000);
    client.withdraw(&user, &asset, &5_000);

    // ── Phase 4: complete recovery ─────────────────────────────────────────
    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: Recovery mode allows only unwind (repay + withdraw)
// ─────────────────────────────────────────────────────────────────────────────

/// In Recovery state, high-risk operations remain blocked while repay and
/// withdraw are explicitly permitted to allow safe position unwind.
#[test]
fn test_recovery_mode_allows_only_unwind() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, guardian, user, asset, collateral_asset) = setup_lifecycle(&env);

    // ── Phase 1: open position ─────────────────────────────────────────────
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // ── Phase 2: transition to Recovery ───────────────────────────────────
    client.emergency_shutdown(&guardian);
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // High-risk ops remain blocked
    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral_asset, &2_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_liquidate(&user, &user, &asset, &collateral_asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Unwind ops are allowed
    client.repay(&user, &asset, &5_000);
    let debt = client.get_user_debt(&user);
    assert!(debt.borrowed_amount <= 10_000);

    client.withdraw(&user, &asset, &1_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 5: complete_recovery re-enables full lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// After a complete shutdown → recovery → complete cycle, the protocol must
/// accept the full deposit → borrow → repay → withdraw lifecycle again.
#[test]
fn test_complete_recovery_re_enables_full_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, guardian, user, asset, collateral_asset) = setup_lifecycle(&env);

    // ── Phase 1: open position ─────────────────────────────────────────────
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // ── Phase 2: full emergency cycle ─────────────────────────────────────
    client.emergency_shutdown(&guardian);
    client.start_recovery(&admin);
    // Unwind
    client.repay(&user, &asset, &10_000);
    client.withdraw(&user, &asset, &10_000);
    client.complete_recovery(&admin);

    // ── Phase 3: verify full lifecycle is restored ─────────────────────────
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // All ops should work again
    let user2 = Address::generate(&env);
    client.deposit(&user2, &asset, &30_000);
    client.borrow(&user2, &asset, &5_000, &collateral_asset, &10_000);
    assert_eq!(client.get_user_debt(&user2).borrowed_amount, 5_000);
    client.repay(&user2, &asset, &2_000);
    client.withdraw(&user2, &asset, &2_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 6: Multi-cycle shutdown/recovery with partial pauses inside recovery
// ─────────────────────────────────────────────────────────────────────────────

/// Runs two complete shutdown/recovery cycles with granular pause flags applied
/// during recovery to simulate a realistic incident response. Verifies that
/// granular pauses during recovery layer correctly on top of emergency rules.
#[test]
fn test_multi_cycle_with_partial_pauses_in_recovery() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, guardian, user, asset, collateral_asset) = setup_lifecycle(&env);

    // ─── Cycle 1 ──────────────────────────────────────────────────────────
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    client.emergency_shutdown(&guardian);
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // Granular Repay pause ON in recovery — repay must still be denied
    client.set_pause(&admin, &PauseType::Repay, &true);
    assert_eq!(
        client.try_repay(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Lift Repay pause — repay should succeed
    client.set_pause(&admin, &PauseType::Repay, &false);
    client.repay(&user, &asset, &5_000);

    // Granular Withdraw pause ON in recovery — withdraw must be denied
    client.set_pause(&admin, &PauseType::Withdraw, &true);
    assert_eq!(
        client.try_withdraw(&user, &asset, &1_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );

    // Lift Withdraw pause — withdraw should succeed
    client.set_pause(&admin, &PauseType::Withdraw, &false);
    client.withdraw(&user, &asset, &5_000);

    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // ─── Cycle 2 ──────────────────────────────────────────────────────────
    let user2 = Address::generate(&env);
    client.deposit(&user2, &asset, &30_000);
    client.borrow(&user2, &asset, &5_000, &collateral_asset, &10_000);

    client.emergency_shutdown(&admin);
    client.start_recovery(&admin);

    client.repay(&user2, &asset, &2_000);
    client.withdraw(&user2, &asset, &2_000);

    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // Verify no residual pauses are leaking across cycles
    let user3 = Address::generate(&env);
    client.deposit(&user3, &asset, &20_000);
    client.borrow(&user3, &asset, &3_000, &collateral_asset, &6_000);
    assert_eq!(client.get_user_debt(&user3).borrowed_amount, 3_000);
    client.repay(&user3, &asset, &3_000);
    client.withdraw(&user3, &asset, &3_000);
}
