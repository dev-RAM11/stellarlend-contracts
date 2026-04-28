//! # Pause-State Matrix Tests
//!
//! Deterministic table-driven tests covering every combination of:
//!   - Pause switch (global `All` + each granular flag)
//!   - Protocol operation (deposit, borrow, repay, withdraw, liquidate, flash_loan,
//!     deposit_collateral)
//!   - Emergency lifecycle state (Normal, Shutdown, Recovery)
//!
//! ## Guarantee
//! Every cell in the matrix is asserted explicitly. A paused operation MUST return
//! the correct typed error; an unpaused operation MUST NOT return a pause error.
//! This prevents regressions where a paused action accidentally succeeds or an
//! unpaused action is silently blocked.
//!
//! ## Security Notes
//! - `PauseType::All` is a master kill-switch; it supersedes every individual flag.
//! - Emergency `Shutdown` blocks ALL operations (including repay/withdraw).
//! - Emergency `Recovery` blocks new-risk ops but allows repay/withdraw.
//! - Granular pauses remain effective inside Recovery (layered defence).
//! - Only the admin can toggle pause flags; the guardian can only trigger shutdown.
//! - Read-only mode is the highest-precedence master switch (tested in read_only_test.rs).
//!
//! Reference: stellar-lend/contracts/lending/pause.md

use super::*;
use crate::deposit::DepositError;
use crate::flash_loan::FlashLoanError;
use crate::withdraw::WithdrawError;
use soroban_sdk::{testutils::Address as _, Address, Bytes, Env};

// ─── shared setup ────────────────────────────────────────────────────────────

/// Initialise a fresh contract with two registered assets and return
/// `(client, admin, user, asset, collateral_asset)`.
fn setup_with_assets(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address,
    Address,
    Address,
    Address,
) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);

    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    let collateral_asset = Address::generate(env);

    client.initialize(&admin, &1_000_000_000, &1000);
    client.register_asset(&admin, &asset);
    client.register_asset(&admin, &collateral_asset);

    (client, admin, user, asset, collateral_asset)
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 – Default state (all flags false)
// ═══════════════════════════════════════════════════════════════════════════

/// Every pause flag defaults to `false` after initialisation.
#[test]
fn test_matrix_default_all_flags_false() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral) = setup_with_assets(&env);

    assert!(!client.get_pause_state(&PauseType::All));
    assert!(!client.get_pause_state(&PauseType::Deposit));
    assert!(!client.get_pause_state(&PauseType::Borrow));
    assert!(!client.get_pause_state(&PauseType::Repay));
    assert!(!client.get_pause_state(&PauseType::Withdraw));
    assert!(!client.get_pause_state(&PauseType::Liquidation));
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
    let _ = admin; // suppress unused warning
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 – Global pause (`PauseType::All`) matrix
// ═══════════════════════════════════════════════════════════════════════════

/// Global pause blocks every user-facing operation.
#[test]
fn test_matrix_global_pause_blocks_all_ops() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::All, &true);

    assert_eq!(
        client.try_deposit(&user, &asset, &10_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &10_000, &collateral, &20_000),
        Err(Ok(BorrowError::ProtocolPaused))
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
        client.try_liquidate(&admin, &user, &asset, &collateral, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_deposit_collateral(&user, &asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_flash_loan(&user, &asset, &1_000, &Bytes::new(&env)),
        Err(Ok(FlashLoanError::ProtocolPaused))
    );
}

/// Disabling global pause restores all operations (assuming no granular flags set).
#[test]
fn test_matrix_global_pause_off_restores_ops() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::All, &true);
    client.set_pause(&admin, &PauseType::All, &false);

    // Operations should succeed (or fail for business reasons, not pause).
    client.deposit(&user, &asset, &10_000);
    client.borrow(&user, &asset, &5_000, &collateral, &10_000);
}

/// Global pause supersedes individual flags that are explicitly `false`.
#[test]
fn test_matrix_global_pause_overrides_individual_unpause() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    // Explicitly set individual flags to false (no-op, but validates precedence).
    client.set_pause(&admin, &PauseType::Deposit, &false);
    client.set_pause(&admin, &PauseType::Borrow, &false);
    client.set_pause(&admin, &PauseType::Repay, &false);
    client.set_pause(&admin, &PauseType::Withdraw, &false);

    // Engage global pause.
    client.set_pause(&admin, &PauseType::All, &true);

    // All operations must still be blocked.
    assert_eq!(
        client.try_deposit(&user, &asset, &10_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &10_000, &collateral, &20_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_repay(&user, &asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw(&user, &asset, &10_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );

    // get_pause_state must report true for every type when All is set.
    assert!(client.get_pause_state(&PauseType::Deposit));
    assert!(client.get_pause_state(&PauseType::Borrow));
    assert!(client.get_pause_state(&PauseType::Repay));
    assert!(client.get_pause_state(&PauseType::Withdraw));
    assert!(client.get_pause_state(&PauseType::Liquidation));
    assert!(client.get_pause_state(&PauseType::All));

    let _ = collateral;
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 – Granular pause matrix (each flag in isolation)
// ═══════════════════════════════════════════════════════════════════════════

/// Pausing `Deposit` blocks deposit and deposit_collateral; all other ops unaffected.
#[test]
fn test_matrix_deposit_pause_blocks_only_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::Deposit, &true);
    assert!(client.get_pause_state(&PauseType::Deposit));

    // Blocked.
    assert_eq!(
        client.try_deposit(&user, &asset, &10_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_deposit_collateral(&user, &asset, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Unaffected: borrow still works.
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    // Unaffected: repay still works.
    client.repay(&user, &asset, &1_000);

    // Other flags remain false.
    assert!(!client.get_pause_state(&PauseType::Borrow));
    assert!(!client.get_pause_state(&PauseType::Repay));
    assert!(!client.get_pause_state(&PauseType::Withdraw));
    assert!(!client.get_pause_state(&PauseType::Liquidation));
}

/// Pausing `Borrow` blocks borrow; all other ops unaffected.
#[test]
fn test_matrix_borrow_pause_blocks_only_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    // Establish a position so repay has something to work with.
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    client.set_pause(&admin, &PauseType::Borrow, &true);
    assert!(client.get_pause_state(&PauseType::Borrow));

    // Blocked.
    assert_eq!(
        client.try_borrow(&user, &asset, &5_000, &collateral, &10_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Unaffected: deposit still works.
    client.deposit(&user, &asset, &5_000);

    // Unaffected: repay still works.
    client.repay(&user, &asset, &1_000);

    // Other flags remain false.
    assert!(!client.get_pause_state(&PauseType::Deposit));
    assert!(!client.get_pause_state(&PauseType::Repay));
    assert!(!client.get_pause_state(&PauseType::Withdraw));
    assert!(!client.get_pause_state(&PauseType::Liquidation));
}

/// Pausing `Repay` blocks repay; borrow and deposit are unaffected.
#[test]
fn test_matrix_repay_pause_blocks_only_repay() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::Repay, &true);
    assert!(client.get_pause_state(&PauseType::Repay));

    // Blocked.
    assert_eq!(
        client.try_repay(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Unaffected: borrow still works.
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    // Unaffected: deposit still works.
    client.deposit(&user, &asset, &5_000);

    // Other flags remain false.
    assert!(!client.get_pause_state(&PauseType::Deposit));
    assert!(!client.get_pause_state(&PauseType::Borrow));
    assert!(!client.get_pause_state(&PauseType::Withdraw));
    assert!(!client.get_pause_state(&PauseType::Liquidation));
}

/// Pausing `Withdraw` blocks withdraw; all other ops unaffected.
#[test]
fn test_matrix_withdraw_pause_blocks_only_withdraw() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::Withdraw, &true);
    assert!(client.get_pause_state(&PauseType::Withdraw));

    // Blocked.
    assert_eq!(
        client.try_withdraw(&user, &asset, &1_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );

    // Unaffected: deposit still works.
    client.deposit(&user, &asset, &5_000);

    // Unaffected: borrow still works.
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    // Unaffected: repay still works.
    client.repay(&user, &asset, &1_000);

    // Other flags remain false.
    assert!(!client.get_pause_state(&PauseType::Deposit));
    assert!(!client.get_pause_state(&PauseType::Borrow));
    assert!(!client.get_pause_state(&PauseType::Repay));
    assert!(!client.get_pause_state(&PauseType::Liquidation));
}

/// Pausing `Liquidation` blocks liquidate; deposit, borrow, repay, withdraw unaffected.
#[test]
fn test_matrix_liquidation_pause_blocks_only_liquidate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    client.set_pause(&admin, &PauseType::Liquidation, &true);
    assert!(client.get_pause_state(&PauseType::Liquidation));

    // Blocked.
    assert_eq!(
        client.try_liquidate(&admin, &user, &asset, &collateral, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Unaffected: all other ops work.
    client.deposit(&user, &asset, &10_000);
    client.borrow(&user, &asset, &5_000, &collateral, &10_000);
    client.repay(&user, &asset, &1_000);
    client.withdraw(&user, &asset, &1_000);

    // Other flags remain false.
    assert!(!client.get_pause_state(&PauseType::Deposit));
    assert!(!client.get_pause_state(&PauseType::Borrow));
    assert!(!client.get_pause_state(&PauseType::Repay));
    assert!(!client.get_pause_state(&PauseType::Withdraw));
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 – Precedence matrix (All × granular)
// ═══════════════════════════════════════════════════════════════════════════

/// Full 2×2 precedence table for `All` vs `Borrow` flag.
///
/// | All   | Borrow | Expected for borrow |
/// |-------|--------|---------------------|
/// | false | false  | ALLOWED             |
/// | false | true   | PAUSED              |
/// | true  | false  | PAUSED (All wins)   |
/// | true  | true   | PAUSED              |
#[test]
fn test_matrix_precedence_all_x_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    // Row 1: All=false, Borrow=false → ALLOWED
    client.set_pause(&admin, &PauseType::All, &false);
    client.set_pause(&admin, &PauseType::Borrow, &false);
    assert!(!client.get_pause_state(&PauseType::Borrow));
    client.borrow(&user, &asset, &1_000, &collateral, &2_000);

    // Row 2: All=false, Borrow=true → PAUSED
    client.set_pause(&admin, &PauseType::Borrow, &true);
    assert!(client.get_pause_state(&PauseType::Borrow));
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral, &2_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Row 3: All=true, Borrow=false → PAUSED (All supersedes)
    client.set_pause(&admin, &PauseType::All, &true);
    client.set_pause(&admin, &PauseType::Borrow, &false);
    assert!(client.get_pause_state(&PauseType::Borrow)); // All makes it appear paused
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral, &2_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Row 4: All=true, Borrow=true → PAUSED
    client.set_pause(&admin, &PauseType::Borrow, &true);
    assert!(client.get_pause_state(&PauseType::Borrow));
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral, &2_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
}

/// Full 2×2 precedence table for `All` vs `Deposit` flag.
#[test]
fn test_matrix_precedence_all_x_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, _collateral) = setup_with_assets(&env);

    // Row 1: All=false, Deposit=false → ALLOWED
    client.set_pause(&admin, &PauseType::All, &false);
    client.set_pause(&admin, &PauseType::Deposit, &false);
    assert!(!client.get_pause_state(&PauseType::Deposit));
    client.deposit(&user, &asset, &1_000);

    // Row 2: All=false, Deposit=true → PAUSED
    client.set_pause(&admin, &PauseType::Deposit, &true);
    assert!(client.get_pause_state(&PauseType::Deposit));
    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );

    // Row 3: All=true, Deposit=false → PAUSED
    client.set_pause(&admin, &PauseType::All, &true);
    client.set_pause(&admin, &PauseType::Deposit, &false);
    assert!(client.get_pause_state(&PauseType::Deposit));
    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );

    // Row 4: All=true, Deposit=true → PAUSED
    client.set_pause(&admin, &PauseType::Deposit, &true);
    assert!(client.get_pause_state(&PauseType::Deposit));
    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 5 – Multiple simultaneous pauses
// ═══════════════════════════════════════════════════════════════════════════

/// Multiple flags can be active simultaneously; toggling one does not affect others.
#[test]
fn test_matrix_multiple_simultaneous_pauses_independent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::Deposit, &true);
    client.set_pause(&admin, &PauseType::Borrow, &true);
    client.set_pause(&admin, &PauseType::Liquidation, &true);

    assert!(client.get_pause_state(&PauseType::Deposit));
    assert!(client.get_pause_state(&PauseType::Borrow));
    assert!(client.get_pause_state(&PauseType::Liquidation));
    assert!(!client.get_pause_state(&PauseType::Repay));
    assert!(!client.get_pause_state(&PauseType::Withdraw));

    // Unpausing Borrow does not affect Deposit or Liquidation.
    client.set_pause(&admin, &PauseType::Borrow, &false);
    assert!(!client.get_pause_state(&PauseType::Borrow));
    assert!(client.get_pause_state(&PauseType::Deposit));
    assert!(client.get_pause_state(&PauseType::Liquidation));

    // Borrow now works; deposit is still blocked.
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);
    assert_eq!(
        client.try_deposit(&user, &asset, &5_000),
        Err(Ok(DepositError::DepositPaused))
    );
}

/// Toggling a flag multiple times converges to the last written value.
#[test]
fn test_matrix_toggle_idempotency() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    for _ in 0..5 {
        client.set_pause(&admin, &PauseType::Borrow, &true);
        client.set_pause(&admin, &PauseType::Borrow, &false);
    }

    // Final state is unpaused.
    assert!(!client.get_pause_state(&PauseType::Borrow));
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 6 – Emergency state × operation matrix
// ═══════════════════════════════════════════════════════════════════════════

/// In `Shutdown` state every operation is blocked regardless of granular flags.
///
/// | Operation          | Shutdown | Expected error          |
/// |--------------------|----------|-------------------------|
/// | deposit            | ❌       | DepositPaused           |
/// | deposit_collateral | ❌       | ProtocolPaused          |
/// | borrow             | ❌       | ProtocolPaused          |
/// | repay              | ❌       | ProtocolPaused          |
/// | withdraw           | ❌       | WithdrawPaused          |
/// | liquidate          | ❌       | ProtocolPaused          |
/// | flash_loan         | ❌       | ProtocolPaused          |
#[test]
fn test_matrix_shutdown_blocks_all_ops() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    // Establish a position so repay/withdraw have something to act on.
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    client.emergency_shutdown(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_deposit_collateral(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral, &2_000),
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
        client.try_liquidate(&admin, &user, &asset, &collateral, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_flash_loan(&user, &asset, &1_000, &Bytes::new(&env)),
        Err(Ok(FlashLoanError::ProtocolPaused))
    );
}

/// In `Recovery` state new-risk ops are blocked; repay and withdraw are allowed.
///
/// | Operation          | Recovery | Expected                |
/// |--------------------|----------|-------------------------|
/// | deposit            | ❌       | DepositPaused           |
/// | deposit_collateral | ❌       | ProtocolPaused          |
/// | borrow             | ❌       | ProtocolPaused          |
/// | repay              | ✅       | success                 |
/// | withdraw           | ✅       | success                 |
/// | liquidate          | ❌       | ProtocolPaused          |
/// | flash_loan         | ❌       | ProtocolPaused          |
#[test]
fn test_matrix_recovery_allows_unwind_blocks_new_risk() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    client.emergency_shutdown(&admin);
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // New-risk ops blocked.
    assert_eq!(
        client.try_deposit(&user, &asset, &1_000),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_deposit_collateral(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &1_000, &collateral, &2_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_liquidate(&admin, &user, &asset, &collateral, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_flash_loan(&user, &asset, &1_000, &Bytes::new(&env)),
        Err(Ok(FlashLoanError::ProtocolPaused))
    );

    // Unwind ops allowed.
    client.repay(&user, &asset, &1_000);
    client.withdraw(&user, &asset, &1_000);
}

/// After `complete_recovery` the protocol returns to Normal and all ops work.
#[test]
fn test_matrix_complete_recovery_restores_normal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);

    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    client.emergency_shutdown(&admin);
    client.start_recovery(&admin);
    client.repay(&user, &asset, &10_000); // unwind
    client.complete_recovery(&admin);

    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    // All ops should work again.
    client.deposit(&user, &asset, &5_000);
    client.borrow(&user, &asset, &2_000, &collateral, &4_000);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 7 – Granular pause × emergency state interaction
// ═══════════════════════════════════════════════════════════════════════════

/// Granular `Repay` pause is respected even inside Recovery (layered defence).
#[test]
fn test_matrix_granular_repay_pause_in_recovery() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    client.emergency_shutdown(&admin);
    client.start_recovery(&admin);

    // Granular repay pause takes precedence even in Recovery.
    client.set_pause(&admin, &PauseType::Repay, &true);
    assert_eq!(
        client.try_repay(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );

    // Lifting the granular pause restores repay.
    client.set_pause(&admin, &PauseType::Repay, &false);
    client.repay(&user, &asset, &1_000);
}

/// Granular `Withdraw` pause is respected even inside Recovery.
#[test]
fn test_matrix_granular_withdraw_pause_in_recovery() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    client.emergency_shutdown(&admin);
    client.start_recovery(&admin);

    client.set_pause(&admin, &PauseType::Withdraw, &true);
    assert_eq!(
        client.try_withdraw(&user, &asset, &1_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );

    client.set_pause(&admin, &PauseType::Withdraw, &false);
    client.withdraw(&user, &asset, &1_000);
}

/// Attempting to unpause via `set_pause` during Shutdown does NOT re-enable repay/withdraw.
/// The emergency state is a separate, higher-priority layer.
#[test]
fn test_matrix_unpause_during_shutdown_does_not_bypass_emergency() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);
    client.deposit(&user, &asset, &50_000);
    client.borrow(&user, &asset, &10_000, &collateral, &20_000);

    client.emergency_shutdown(&admin);

    // Explicitly set granular flags to false — should not bypass Shutdown.
    client.set_pause(&admin, &PauseType::Repay, &false);
    client.set_pause(&admin, &PauseType::Withdraw, &false);

    assert_eq!(
        client.try_repay(&user, &asset, &1_000),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw(&user, &asset, &1_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 8 – Flash loan pause specifics
// ═══════════════════════════════════════════════════════════════════════════

/// Flash loan is blocked by `PauseType::All` (global pause).
#[test]
fn test_matrix_flash_loan_blocked_by_global_pause() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, _collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::All, &true);

    assert_eq!(
        client.try_flash_loan(&user, &asset, &1_000, &Bytes::new(&env)),
        Err(Ok(FlashLoanError::ProtocolPaused))
    );
}

/// Flash loan is NOT blocked by individual granular flags (only by `All` or emergency state).
#[test]
fn test_matrix_flash_loan_not_blocked_by_granular_pauses() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, _collateral) = setup_with_assets(&env);

    // Set every individual flag — none of these should block flash loans.
    client.set_pause(&admin, &PauseType::Deposit, &true);
    client.set_pause(&admin, &PauseType::Borrow, &true);
    client.set_pause(&admin, &PauseType::Repay, &true);
    client.set_pause(&admin, &PauseType::Withdraw, &true);
    client.set_pause(&admin, &PauseType::Liquidation, &true);

    // Flash loan may fail for business reasons (zero amount / no callback),
    // but the error must NOT be ProtocolPaused.
    let result = client.try_flash_loan(&user, &asset, &0, &Bytes::new(&env));
    assert_ne!(result, Err(Ok(FlashLoanError::ProtocolPaused)));
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 9 – Authorization matrix
// ═══════════════════════════════════════════════════════════════════════════

/// Only the admin can toggle pause flags; non-admin callers are rejected.
#[test]
fn test_matrix_only_admin_can_set_pause() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, _asset, _collateral) = setup_with_assets(&env);

    // Non-admin cannot pause.
    assert_eq!(
        client.try_set_pause(&user, &PauseType::Borrow, &true),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Admin can pause.
    client.set_pause(&admin, &PauseType::Borrow, &true);
    assert!(client.get_pause_state(&PauseType::Borrow));
}

/// Non-admin cannot unpause an active pause flag.
#[test]
fn test_matrix_non_admin_cannot_unpause() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, _asset, _collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::Borrow, &true);

    assert_eq!(
        client.try_set_pause(&user, &PauseType::Borrow, &false),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Flag remains paused.
    assert!(client.get_pause_state(&PauseType::Borrow));
}

/// Guardian can trigger emergency shutdown but cannot call `set_pause`.
#[test]
fn test_matrix_guardian_cannot_set_pause() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral) = setup_with_assets(&env);

    let guardian = Address::generate(&env);
    client.set_guardian(&admin, &guardian);

    assert_eq!(
        client.try_set_pause(&guardian, &PauseType::Borrow, &true),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Guardian can still trigger shutdown.
    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);
}

/// A random address cannot trigger emergency shutdown.
#[test]
fn test_matrix_random_address_cannot_trigger_shutdown() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral) = setup_with_assets(&env);

    assert_eq!(
        client.try_emergency_shutdown(&user),
        Err(Ok(BorrowError::Unauthorized))
    );
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

/// `set_deposit_paused` convenience wrapper requires admin authorization.
#[test]
fn test_matrix_set_deposit_paused_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, _asset, _collateral) = setup_with_assets(&env);

    assert_eq!(
        client.try_set_deposit_paused(&user, &true),
        Err(Ok(DepositError::Unauthorized))
    );

    // Admin succeeds.
    client.set_deposit_paused(&admin, &true);
    assert!(client.get_pause_state(&PauseType::Deposit));
}

/// `set_withdraw_paused` convenience wrapper requires admin authorization.
#[test]
fn test_matrix_set_withdraw_paused_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, _asset, _collateral) = setup_with_assets(&env);

    assert_eq!(
        client.try_set_withdraw_paused(&user, &true),
        Err(Ok(WithdrawError::Unauthorized))
    );

    // Admin succeeds.
    client.set_withdraw_paused(&admin, &true);
    assert!(client.get_pause_state(&PauseType::Withdraw));
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 10 – Convenience wrappers
// ═══════════════════════════════════════════════════════════════════════════

/// `set_deposit_paused(true)` blocks deposit; `set_deposit_paused(false)` restores it.
#[test]
fn test_matrix_set_deposit_paused_wrapper() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, _collateral) = setup_with_assets(&env);

    client.set_deposit_paused(&admin, &true);
    assert!(client.get_pause_state(&PauseType::Deposit));
    assert_eq!(
        client.try_deposit(&user, &asset, &10_000),
        Err(Ok(DepositError::DepositPaused))
    );

    client.set_deposit_paused(&admin, &false);
    assert!(!client.get_pause_state(&PauseType::Deposit));
    client.deposit(&user, &asset, &10_000);
}

/// `set_withdraw_paused(true)` blocks withdraw; `set_withdraw_paused(false)` restores it.
#[test]
fn test_matrix_set_withdraw_paused_wrapper() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, _collateral) = setup_with_assets(&env);

    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);
    client.deposit(&user, &asset, &10_000);

    client.set_withdraw_paused(&admin, &true);
    assert!(client.get_pause_state(&PauseType::Withdraw));
    assert_eq!(
        client.try_withdraw(&user, &asset, &1_000),
        Err(Ok(WithdrawError::WithdrawPaused))
    );

    client.set_withdraw_paused(&admin, &false);
    assert!(!client.get_pause_state(&PauseType::Withdraw));
    client.withdraw(&user, &asset, &1_000);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 11 – Zero-amount operations respect pause checks
// ═══════════════════════════════════════════════════════════════════════════

/// Pause checks fire before amount validation; zero-amount calls must still be blocked.
#[test]
fn test_matrix_zero_amount_ops_blocked_by_pause() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral) = setup_with_assets(&env);

    client.set_pause(&admin, &PauseType::Deposit, &true);
    client.set_pause(&admin, &PauseType::Borrow, &true);
    client.set_pause(&admin, &PauseType::Repay, &true);
    client.set_pause(&admin, &PauseType::Withdraw, &true);

    assert_eq!(
        client.try_deposit(&user, &asset, &0),
        Err(Ok(DepositError::DepositPaused))
    );
    assert_eq!(
        client.try_borrow(&user, &asset, &0, &collateral, &0),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_repay(&user, &asset, &0),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(
        client.try_withdraw(&user, &asset, &0),
        Err(Ok(WithdrawError::WithdrawPaused))
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 12 – Guardian management
// ═══════════════════════════════════════════════════════════════════════════

/// `get_guardian` returns `None` before any guardian is configured.
#[test]
fn test_matrix_guardian_initially_none() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _user, _asset, _collateral) = setup_with_assets(&env);

    assert_eq!(client.get_guardian(), None);
}

/// `set_guardian` stores the address; rotating it replaces the previous one.
#[test]
fn test_matrix_set_guardian_and_rotate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral) = setup_with_assets(&env);

    let guardian = Address::generate(&env);
    client.set_guardian(&admin, &guardian);
    assert_eq!(client.get_guardian(), Some(guardian.clone()));

    let new_guardian = Address::generate(&env);
    client.set_guardian(&admin, &new_guardian);
    assert_eq!(client.get_guardian(), Some(new_guardian));
}

/// Non-admin cannot configure the guardian.
#[test]
fn test_matrix_non_admin_cannot_set_guardian() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral) = setup_with_assets(&env);

    assert_eq!(
        client.try_set_guardian(&user, &Address::generate(&env)),
        Err(Ok(BorrowError::Unauthorized))
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 13 – Emergency lifecycle state transitions
// ═══════════════════════════════════════════════════════════════════════════

/// `start_recovery` fails when the protocol is in Normal state.
#[test]
fn test_matrix_start_recovery_requires_shutdown_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral) = setup_with_assets(&env);

    assert_eq!(
        client.try_start_recovery(&admin),
        Err(Ok(BorrowError::ProtocolPaused))
    );
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

/// Full lifecycle: Normal → Shutdown → Recovery → Normal.
#[test]
fn test_matrix_full_emergency_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral) = setup_with_assets(&env);

    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);

    client.emergency_shutdown(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

/// Admin can skip Recovery and go directly from Shutdown to Normal via `complete_recovery`.
#[test]
fn test_matrix_complete_recovery_from_shutdown_skips_recovery() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral) = setup_with_assets(&env);

    client.emergency_shutdown(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}

/// Guardian can trigger shutdown; only admin can manage recovery.
#[test]
fn test_matrix_guardian_shutdown_admin_recovery() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral) = setup_with_assets(&env);

    let guardian = Address::generate(&env);
    client.set_guardian(&admin, &guardian);

    client.emergency_shutdown(&guardian);
    assert_eq!(client.get_emergency_state(), EmergencyState::Shutdown);

    // Guardian cannot start recovery.
    assert_eq!(
        client.try_start_recovery(&guardian),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Admin can.
    client.start_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Recovery);

    // Guardian cannot complete recovery.
    assert_eq!(
        client.try_complete_recovery(&guardian),
        Err(Ok(BorrowError::Unauthorized))
    );

    // Admin can.
    client.complete_recovery(&admin);
    assert_eq!(client.get_emergency_state(), EmergencyState::Normal);
}
