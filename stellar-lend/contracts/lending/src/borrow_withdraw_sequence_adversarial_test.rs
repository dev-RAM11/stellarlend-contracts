//! # Borrow-Withdraw Sequence Adversarial Tests — Issue #472 Extension
//!
//! Realistic multi-step adversarial sequences that attempt to exploit
//! rounding, timing, view inconsistencies, and oracle price changes.
//!
//! ## Threat Model
//!
//! | # | Attack Vector | Defence |
//! |---|---------------|---------|
//! | 1 | Borrow → add collateral → price drop → over-withdraw | Raw 150 % ratio check |
//! | 2 | Partial repay → price drop → boundary withdraw | Debt-aware ratio check |
//! | 3 | Interest accrual + price drop combined | Fresh interest calc + ratio |
//! | 4 | View inconsistency during price volatility | Views read-only; withdraw uses raw state |
//! | 5 | Rounding exploitation across price changes | Integer math ceiling |
//! | 6 | Deposit path vs borrow path isolation | Separate collateral stores |
//! | 7 | Add collateral → immediate withdraw attempt | Ratio check on total collateral |
//! | 8 | Price spike → attempt larger withdraw | Withdraw ignores oracle prices |
//! | 9 | Stale oracle → withdraw bypass attempt | Raw ratio still enforced |
//! | 10 | Full realistic sequence: borrow, add, withdraw, repay, price change | Consistent ratio enforcement |
//!
//! ## Security Invariant
//! After every successful `withdraw`, the remaining deposit-path collateral
//! must satisfy `collateral >= debt * MIN_COLLATERAL_RATIO_BPS / BPS_SCALE`
//! (150 % default). This invariant is enforced by
//! `validate_collateral_ratio_after_withdraw`, which delegates to the same
//! `borrow::validate_collateral_ratio` used at borrow time.
//!
//! ## Design Note
//! The contract maintains two collateral stores:
//! - `DepositDataKey::UserCollateral` — used by `deposit()` and `withdraw()`
//! - `BorrowDataKey::BorrowUserCollateral` — used by `borrow()` and `deposit_collateral()`
//!
//! `withdraw()` reads collateral from the deposit path but checks debt from the
//! borrow path. This means collateral added via `deposit_collateral()` cannot be
//! withdrawn via `withdraw()` — a property these tests verify as a security
//! boundary.

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use views::{HEALTH_FACTOR_SCALE, HEALTH_FACTOR_NO_DEBT};

// ─── helpers ────────────────────────────────────────────────────────────────

const PRICE_UNITY: i128 = 100_000_000; // 1.0 with 8 decimals
const PRICE_HALF: i128 = 50_000_000;   // 0.5 with 8 decimals
const PRICE_DOUBLE: i128 = 200_000_000; // 2.0 with 8 decimals

fn setup(
    env: &Env,
) -> (LendingContractClient<'_>, Address, Address, Address, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let asset = Address::generate(env);
    let collateral_asset = Address::generate(env);
    client.initialize(&admin, &1_000_000_000, &1000);
    client.initialize_deposit_settings(&1_000_000_000, &100);
    client.initialize_withdraw_settings(&100);
    (client, admin, user, asset, collateral_asset)
}

fn setup_with_oracle(
    env: &Env,
) -> (LendingContractClient<'_>, Address, Address, Address, Address) {
    let (client, admin, user, asset, collateral_asset) = setup(env);
    env.ledger().with_mut(|li| li.timestamp = 0);
    // Seed initial prices for both assets
    client.update_price_feed(&admin, &asset, &PRICE_UNITY);
    client.update_price_feed(&admin, &collateral_asset, &PRICE_UNITY);
    (client, admin, user, asset, collateral_asset)
}

/// Compute required collateral for a given debt at 150 %.
fn required_collateral(debt: i128) -> i128 {
    debt.checked_mul(15_000).unwrap().checked_div(10_000).unwrap()
}

// ─── 1. Borrow → add collateral → price drop → over-withdraw blocked ───────

#[test]
fn test_borrow_add_collateral_price_drop_withdraw_blocked() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 100,000 of withdrawable collateral (deposit path)
    client.deposit(&user, &collateral_asset, &100_000);

    // Borrow 10,000 with 20,000 borrow-path collateral (200 %)
    client.borrow(&user, &borrow_asset, &10_000, &collateral_asset, &20_000);

    // Add 10,000 more borrow-path collateral
    client.deposit_collateral(&user, &collateral_asset, &10_000);

    // Drop collateral price 50 % — views now show worse health
    client.update_price_feed(&admin, &collateral_asset, &PRICE_HALF);

    let hf = client.get_health_factor(&user);
    assert!(hf < HEALTH_FACTOR_SCALE, "HF should be < 1.0 after price drop");

    // Attempt to withdraw 86,001 from deposit path.
    // Deposit path has 100,000. Debt = 10,000. Required = 15,000.
    // Max safe = 85,000. Withdrawing 86,001 → remaining 13,999 < 15,000.
    let result = client.try_withdraw(&user, &collateral_asset, &86_001);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Exactly 85,000 should succeed
    let remaining = client.withdraw(&user, &collateral_asset, &85_000);
    assert_eq!(remaining, 15_000);
}

// ─── 2. Partial repay → price drop → boundary withdraw blocked ─────────────

#[test]
fn test_partial_repay_price_drop_boundary_withdraw() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 100,000 withdrawable collateral
    client.deposit(&user, &collateral_asset, &100_000);

    // Borrow 10,000 with 20,000 collateral (200 %)
    client.borrow(&user, &borrow_asset, &10_000, &collateral_asset, &20_000);

    // Partial repay 4,000 → debt now 6,000, required = 9,000
    client.repay(&user, &borrow_asset, &4_000);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 6_000);

    // Drop collateral price 50 %
    client.update_price_feed(&admin, &collateral_asset, &PRICE_HALF);

    // Views show changed health, but withdraw uses raw amounts.
    // Max safe withdraw = 100,000 - 9,000 = 91,000.
    let result = client.try_withdraw(&user, &collateral_asset, &91_001);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Exactly 91,000 should succeed
    let remaining = client.withdraw(&user, &collateral_asset, &91_000);
    assert_eq!(remaining, 9_000);
}

// ─── 3. Interest accrual + price drop combined attack ───────────────────────

#[test]
fn test_interest_plus_price_drop_combined_attack() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    env.ledger().with_mut(|li| li.timestamp = 0);

    // Deposit 150,000 withdrawable collateral
    client.deposit(&user, &collateral_asset, &150_000);

    // Borrow exactly at 150 % boundary: 100,000 debt, 0 additional collateral
    // (deposit path provides the collateral for ratio check)
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Advance 1 year — interest accrues (~5,000)
    env.ledger().with_mut(|li| li.timestamp = 31_536_000);

    let debt = client.get_user_debt(&user);
    let total_debt = debt.borrowed_amount + debt.interest_accrued;
    assert!(total_debt > 100_000, "interest must have accrued");

    // Drop collateral price 50 %
    client.update_price_feed(&admin, &collateral_asset, &PRICE_HALF);

    // Position is deeply underwater in view terms
    let hf = client.get_health_factor(&user);
    assert!(hf < HEALTH_FACTOR_SCALE, "HF must be < 1.0");

    // Required collateral = total_debt * 1.5
    let req = required_collateral(total_debt);
    assert!(req > 150_000, "required must exceed original collateral due to interest");

    // Even withdrawing 1 unit should fail because deposit path = 150,000 < req
    let result = client.try_withdraw(&user, &collateral_asset, &1);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );
}

// ─── 4. View inconsistency during price volatility ──────────────────────────

#[test]
fn test_view_price_volatility_consistency() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 200,000
    client.deposit(&user, &collateral_asset, &200_000);

    // Borrow 100,000 with 0 additional collateral (200 % effective)
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Record views at initial price
    let pos_before = client.get_user_position(&user);
    let hf_before = client.get_health_factor(&user);
    assert!(hf_before >= HEALTH_FACTOR_SCALE);

    // Collateral price doubles — views look amazing
    client.update_price_feed(&admin, &collateral_asset, &PRICE_DOUBLE);
    let hf_after_spike = client.get_health_factor(&user);
    assert!(hf_after_spike > hf_before, "HF must increase with price spike");

    // Attempt to withdraw more than raw ratio allows.
    // Debt = 100,000. Required = 150,000. Deposit = 200,000.
    // Max safe = 50,000. Try 50,001.
    let result = client.try_withdraw(&user, &collateral_asset, &50_001);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Price crashes to 25 % of original
    client.update_price_feed(&admin, &collateral_asset, &25_000_000);
    let hf_after_crash = client.get_health_factor(&user);
    assert!(hf_after_crash < HEALTH_FACTOR_SCALE);

    // Withdraw must still be blocked by raw ratio, not oracle views
    let result2 = client.try_withdraw(&user, &collateral_asset, &50_001);
    assert_eq!(
        result2,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Valid withdrawal should still succeed
    let remaining = client.withdraw(&user, &collateral_asset, &50_000);
    assert_eq!(remaining, 150_000);
}

// ─── 5. Rounding exploitation across price changes ──────────────────────────

#[test]
fn test_rounding_exploit_across_price_changes() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 1,500 — smallest amount that passes 150 % for 1,000 borrow
    client.deposit(&user, &collateral_asset, &1_500);

    // Borrow 1,000 with 0 additional collateral
    client.borrow(&user, &borrow_asset, &1_000, &collateral_asset, &0);

    // Change prices to create rounding edge cases
    client.update_price_feed(&admin, &collateral_asset, &99_999_999);
    client.update_price_feed(&admin, &borrow_asset, &100_000_001);

    // Views now have slightly different values, but withdraw uses raw ratio.
    // Required = 1,000 * 1.5 = 1,500. Deposit = 1,500.
    // Withdrawing 1 → remaining 1,499 < 1,500 → must fail.
    let result = client.try_withdraw(&user, &collateral_asset, &1);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Verify position unchanged
    let deposit_pos = client.get_user_collateral_deposit(&user, &collateral_asset);
    assert_eq!(deposit_pos.amount, 1_500);
}

// ─── 6. Deposit path vs borrow path isolation with price change ─────────────

#[test]
fn test_deposit_borrow_path_isolation_with_price_change() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 10,000 via deposit path (withdrawable)
    client.deposit(&user, &collateral_asset, &10_000);

    // Borrow 5,000 with 10,000 via borrow path (NOT withdrawable via withdraw())
    client.borrow(&user, &borrow_asset, &5_000, &collateral_asset, &10_000);

    // Verify separation
    let deposit_pos = client.get_user_collateral_deposit(&user, &collateral_asset);
    assert_eq!(deposit_pos.amount, 10_000);

    let borrow_pos = client.get_user_collateral(&user);
    assert_eq!(borrow_pos.amount, 10_000);

    // Change prices
    client.update_price_feed(&admin, &collateral_asset, &PRICE_HALF);

    // withdraw() reads deposit path (10,000) and checks borrow debt (5,000).
    // Required = 7,500. Max safe withdraw = 2,500.
    let result = client.try_withdraw(&user, &collateral_asset, &2_501);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Exactly 2,500 should succeed
    let remaining = client.withdraw(&user, &collateral_asset, &2_500);
    assert_eq!(remaining, 7_500);

    // Borrow-path collateral remains untouched
    let borrow_pos_after = client.get_user_collateral(&user);
    assert_eq!(borrow_pos_after.amount, 10_000);
}

// ─── 7. Manipulative collateral add then immediate withdraw blocked ─────────

#[test]
fn test_manipulative_collateral_add_then_immediate_withdraw() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, borrow_asset, collateral_asset) = setup(&env);

    // Borrow 10,000 with 15,000 collateral (exactly 150 %)
    client.borrow(&user, &borrow_asset, &10_000, &collateral_asset, &15_000);

    // Add 20,000 more collateral via deposit_collateral (borrow path)
    client.deposit_collateral(&user, &collateral_asset, &20_000);

    // Total borrow-path collateral = 35,000. Debt = 10,000. Required = 15,000.
    // Attempt to withdraw the newly added 20,001 from deposit path.
    // Deposit path has 0 (we never called deposit()), so this fails on balance.
    let result = client.try_withdraw(&user, &collateral_asset, &20_001);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateral))
    );

    // Now deposit 20,000 into withdrawable path
    client.deposit(&user, &collateral_asset, &20_000);

    // Try to withdraw all 20,001 of the newly added deposit-path collateral.
    // Total deposit path = 20,000. Required for 10,000 debt = 15,000.
    // Max safe = 5,000. Withdrawing 20,001 fails.
    let result2 = client.try_withdraw(&user, &collateral_asset, &20_001);
    assert_eq!(
        result2,
        Err(Ok(WithdrawError::InsufficientCollateral))
    );

    // Even withdrawing 5,001 should fail on ratio
    let result3 = client.try_withdraw(&user, &collateral_asset, &5_001);
    assert_eq!(
        result3,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Exactly 5,000 should succeed
    let remaining = client.withdraw(&user, &collateral_asset, &5_000);
    assert_eq!(remaining, 15_000);
}

// ─── 8. Price spike does not increase withdraw limit ────────────────────────

#[test]
fn test_price_spike_does_not_increase_withdraw_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 200,000
    client.deposit(&user, &collateral_asset, &200_000);

    // Borrow 100,000 with 0 additional collateral
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Collateral price spikes 10x
    client.update_price_feed(&admin, &collateral_asset, &1_000_000_000);

    // Views show massive health factor
    let hf = client.get_health_factor(&user);
    assert!(hf > HEALTH_FACTOR_SCALE * 5, "HF should be huge after spike");

    // Attempt to withdraw 51,000 (would leave 149,000 < 150,000 required)
    // Withdraw uses RAW ratio, not oracle prices, so this must fail.
    let result = client.try_withdraw(&user, &collateral_asset, &51_000);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Max safe = 50,000
    let remaining = client.withdraw(&user, &collateral_asset, &50_000);
    assert_eq!(remaining, 150_000);
}

// ─── 9. Stale oracle → withdraw still uses raw ratio ────────────────────────

#[test]
fn test_stale_oracle_withdraw_uses_raw_ratio() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 200,000
    client.deposit(&user, &collateral_asset, &200_000);

    // Borrow 100,000
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Advance past default staleness threshold (3600s)
    env.ledger().with_mut(|li| li.timestamp = 4_000);

    // Views return 0 for values because oracle is stale
    let cv = client.get_collateral_value(&user);
    let dv = client.get_debt_value(&user);
    let hf = client.get_health_factor(&user);
    assert_eq!(cv, 0, "collateral value must be 0 with stale oracle");
    assert_eq!(dv, 0, "debt value must be 0 with stale oracle");
    assert_eq!(hf, 0, "health factor must be 0 with stale oracle");

    // Withdraw must still enforce raw 150 % ratio despite stale views
    let result = client.try_withdraw(&user, &collateral_asset, &51_000);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // Valid withdrawal should still work
    let remaining = client.withdraw(&user, &collateral_asset, &50_000);
    assert_eq!(remaining, 150_000);
}

// ─── 10. Full realistic sequence: borrow, add, withdraw, repay, price change ─

#[test]
fn test_multi_step_sequence_every_operation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    env.ledger().with_mut(|li| li.timestamp = 0);

    // Step 1: Deposit 200,000 withdrawable collateral
    client.deposit(&user, &collateral_asset, &200_000);

    // Step 2: Borrow 100,000 with 0 additional collateral (200 % effective)
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Step 3: Add 50,000 more to borrow path
    client.deposit_collateral(&user, &collateral_asset, &50_000);

    // Step 4: Partial withdraw 30,000 from deposit path
    // Deposit path = 200,000. Debt = 100,000. Required = 150,000.
    // Remaining after = 170,000 > 150,000 → OK
    let remaining = client.withdraw(&user, &collateral_asset, &30_000);
    assert_eq!(remaining, 170_000);

    // Step 5: Partial repay 40,000
    // Debt = 60,000. Required = 90,000.
    client.repay(&user, &borrow_asset, &40_000);
    let debt_after_repay = client.get_user_debt(&user);
    assert_eq!(debt_after_repay.borrowed_amount, 60_000);

    // Step 6: Collateral price drops 20 %
    client.update_price_feed(&admin, &collateral_asset, &80_000_000);

    // Views show reduced health, but withdraw limit unchanged (raw ratio)
    let hf = client.get_health_factor(&user);
    assert!(hf < HEALTH_FACTOR_SCALE, "HF < 1.0 after price drop");

    // Step 7: Withdraw 80,000 from deposit path
    // Deposit path = 170,000. Required = 90,000.
    // Remaining after = 90,000 = required → OK
    let remaining2 = client.withdraw(&user, &collateral_asset, &80_000);
    assert_eq!(remaining2, 90_000);

    // Step 8: Advance 6 months — interest accrues
    env.ledger().with_mut(|li| li.timestamp = 15_768_000); // ~0.5 year

    let debt_with_interest = client.get_user_debt(&user);
    let total_debt = debt_with_interest.borrowed_amount + debt_with_interest.interest_accrued;
    assert!(total_debt > 60_000, "interest must accrue");

    // Step 9: Try to withdraw 1 unit over safe limit
    let req = required_collateral(total_debt);
    let max_safe = 90_000 - req;
    assert!(max_safe >= 0);

    if max_safe > 0 {
        let result = client.try_withdraw(&user, &collateral_asset, &(max_safe + 1));
        assert_eq!(
            result,
            Err(Ok(WithdrawError::InsufficientCollateralRatio))
        );
    }

    // Step 10: Repay all debt
    client.repay(&user, &borrow_asset, &total_debt);
    let debt_final = client.get_user_debt(&user);
    assert_eq!(debt_final.borrowed_amount, 0);
    assert_eq!(debt_final.interest_accrued, 0);

    // Step 11: Full withdraw allowed
    let remaining3 = client.withdraw(&user, &collateral_asset, &90_000 - max_safe);
    // After withdrawing the safe amount, remaining = req. But since debt is 0,
    // we can withdraw everything.
    let remaining4 = client.withdraw(&user, &collateral_asset, &remaining3);
    assert_eq!(remaining4, 0);
}

// ─── 11. Borrow-path collateral cannot be withdrawn via deposit path ────────

#[test]
fn test_borrow_path_collateral_not_withdrawable() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, borrow_asset, collateral_asset) = setup(&env);

    // Add collateral ONLY via deposit_collateral (borrow path)
    client.deposit_collateral(&user, &collateral_asset, &50_000);

    // Borrow against it
    client.borrow(&user, &borrow_asset, &20_000, &collateral_asset, &0);

    // Attempt to withdraw from deposit path — deposit path is empty
    let result = client.try_withdraw(&user, &collateral_asset, &1);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateral))
    );

    // Borrow-path collateral should still be intact
    let borrow_pos = client.get_user_collateral(&user);
    assert_eq!(borrow_pos.amount, 50_000);
}

// ─── 12. Interest-only repayment does not enable extra withdraw ─────────────

#[test]
fn test_interest_only_repay_does_not_enable_extra_withdraw() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, borrow_asset, collateral_asset) = setup(&env);

    env.ledger().with_mut(|li| li.timestamp = 0);

    // Deposit 200,000
    client.deposit(&user, &collateral_asset, &200_000);

    // Borrow 100,000
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Advance 1 second — interest = 1
    env.ledger().with_mut(|li| li.timestamp = 1);

    let debt_before = client.get_user_debt(&user);
    assert_eq!(debt_before.interest_accrued, 1);

    // Repay exactly 1 (only interest)
    client.repay(&user, &borrow_asset, &1);
    let debt_after = client.get_user_debt(&user);
    assert_eq!(debt_after.borrowed_amount, 100_000);
    assert_eq!(debt_after.interest_accrued, 0);

    // Debt is still 100,000. Required = 150,000. Deposit = 200,000.
    // Max safe = 50,000. Withdrawing 50,001 must still fail.
    let result = client.try_withdraw(&user, &collateral_asset, &50_001);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    let remaining = client.withdraw(&user, &collateral_asset, &50_000);
    assert_eq!(remaining, 150_000);
}

// ─── 13. Multiple price changes within single ledger timestamp ──────────────

#[test]
fn test_multiple_price_changes_same_timestamp() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, borrow_asset, collateral_asset) = setup_with_oracle(&env);

    // Deposit 200,000
    client.deposit(&user, &collateral_asset, &200_000);

    // Borrow 100,000
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Rapid price changes at same timestamp
    client.update_price_feed(&admin, &collateral_asset, &PRICE_HALF);
    client.update_price_feed(&admin, &collateral_asset, &PRICE_DOUBLE);
    client.update_price_feed(&admin, &collateral_asset, &PRICE_UNITY);

    // Final price is unity — position should be healthy
    let hf = client.get_health_factor(&user);
    assert!(hf >= HEALTH_FACTOR_SCALE || hf == HEALTH_FACTOR_NO_DEBT);

    // Withdraw limit unchanged throughout
    let result = client.try_withdraw(&user, &collateral_asset, &51_000);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );
}

// ─── 14. Zero debt after full repay → withdraw ignores ratio ────────────────

#[test]
fn test_full_repay_then_withdraw_ignores_ratio() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, borrow_asset, collateral_asset) = setup(&env);

    // Deposit 150,000
    client.deposit(&user, &collateral_asset, &150_000);

    // Borrow 100,000
    client.borrow(&user, &borrow_asset, &100_000, &collateral_asset, &0);

    // Repay all
    client.repay(&user, &borrow_asset, &100_000);
    let debt = client.get_user_debt(&user);
    assert_eq!(debt.borrowed_amount, 0);
    assert_eq!(debt.interest_accrued, 0);

    // Full withdraw should succeed even though ratio "would be" 0/150_000
    let remaining = client.withdraw(&user, &collateral_asset, &150_000);
    assert_eq!(remaining, 0);
}

// ─── 15. Attempt to withdraw collateral asset different from debt asset ─────

#[test]
fn test_withdraw_different_asset_than_debt_blocked_by_ratio() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, borrow_asset, collateral_asset) = setup(&env);

    // Deposit two different assets
    let other_asset = Address::generate(&env);
    client.deposit(&user, &collateral_asset, &100_000);
    client.deposit(&user, &other_asset, &50_000);

    // Borrow against collateral_asset
    client.borrow(&user, &borrow_asset, &10_000, &collateral_asset, &20_000);

    // Debt = 10,000. Required = 15,000 total across all deposit paths.
    // Total deposit = 150,000. Max safe withdraw from either = 135,000.
    // Try to withdraw 136,000 from other_asset.
    let result = client.try_withdraw(&user, &other_asset, &136_000);
    assert_eq!(
        result,
        Err(Ok(WithdrawError::InsufficientCollateralRatio))
    );

    // But withdrawing from other_asset up to safe limit should work
    let remaining = client.withdraw(&user, &other_asset, &35_000);
    assert_eq!(remaining, 15_000);
}

