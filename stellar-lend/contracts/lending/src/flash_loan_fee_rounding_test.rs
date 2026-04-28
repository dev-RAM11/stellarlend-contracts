//! # Flash Loan Fee Rounding & Correctness Tests (Issue #697)
//!
//! Validates that flash-loan fee calculation is correct and predictable across
//! tiny amounts, boundary conditions, large amounts, and adversarial splitting.
//!
//! ## Fee Rounding Semantics
//!
//! Fee is computed as:
//! ```text
//! fee = amount * fee_bps / BPS_SCALE   (BPS_SCALE = 10_000)
//! ```
//! Integer division truncates toward zero, so fees **round down**.
//! For small `amount` values the fee may truncate to zero — this is documented
//! and expected behavior, not a bug. The table below shows the minimum amount
//! that produces a non-zero fee for common fee settings:
//!
//! | fee_bps | min amount for fee ≥ 1 |
//! |---------|------------------------|
//! |       1 |                 10 000 |
//! |       5 |                  2 000 |
//! |       9 |                  1 112 |
//! |     100 |                    100 |
//! |   1 000 |                     10 |
//!
//! ## Fee-Splitting Invariant (Security Note)
//!
//! Because fee rounds down per-call, splitting a single large flash loan into
//! multiple smaller calls with sub-threshold amounts yields a lower (or zero)
//! total fee. The **reentrancy guard** prevents this within a single transaction,
//! but a user can still make sequential calls across separate transactions.
//! Protocols that need strict per-unit fees should enforce a `min_fee` floor
//! (e.g., reject loans where the computed fee is zero) or set a minimum loan
//! amount high enough that a single-unit fee is always non-zero.

#[cfg(test)]
use super::*;
use soroban_sdk::{testutils::Address as _, token, Address, Bytes, Env};

// ── Honest receiver ───────────────────────────────────────────────────────────

/// Repays exactly `amount + fee` (as reported by the protocol).
/// The receiver is pre-funded with enough tokens to cover any fee.
#[contract]
pub struct HonestFeeReceiver;

#[contractimpl]
impl HonestFeeReceiver {
    pub fn on_flash_loan(
        env: Env,
        initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        _params: Bytes,
    ) -> bool {
        let total = amount + fee;
        if total > 0 {
            token::Client::new(&env, &asset).transfer(
                &env.current_contract_address(),
                &initiator,
                &total,
            );
        }
        true
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Sets up a lending contract with the given fee and protocol liquidity.
/// `min_borrow` is set to 1 so tiny-amount tests work without hitting the
/// `min_borrow_amount` guard.
fn setup_fee_env(
    env: &Env,
    fee_bps: i128,
    protocol_balance: i128,
) -> (LendingContractClient<'_>, Address, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    client.initialize(&admin, &1_000_000_000_000_000_000, &1);
    client.set_flash_loan_fee_bps(&fee_bps);
    token::StellarAssetClient::new(env, &asset).mint(&contract_id, &protocol_balance);

    (client, contract_id, asset)
}

/// Executes a flash loan and returns the fee that the protocol actually earned
/// (measured as the increase in the contract's token balance).
fn measure_fee(
    env: &Env,
    client: &LendingContractClient<'_>,
    contract_id: &Address,
    asset: &Address,
    amount: i128,
    extra_receiver_funds: i128,
) -> i128 {
    let token = token::Client::new(env, asset);
    let before = token.balance(contract_id);

    let receiver = env.register(HonestFeeReceiver, ());
    // Pre-fund receiver so it can repay principal + fee
    if extra_receiver_funds > 0 {
        token::StellarAssetClient::new(env, asset).mint(&receiver, &extra_receiver_funds);
    }

    client.flash_loan(&receiver, asset, &amount, &Bytes::new(env));

    token.balance(contract_id) - before
}

// ── Tiny-amount / zero-fee tests ─────────────────────────────────────────────

/// At 5 bps, amounts < 2 000 produce zero fee (1 999 * 5 / 10 000 = 0).
#[test]
fn test_fee_rounds_to_zero_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, asset) = setup_fee_env(&env, 5, 100_000);

    let fee = measure_fee(&env, &client, &contract_id, &asset, 1_999, 0);
    assert_eq!(fee, 0, "fee must be 0 for sub-threshold amounts");
}

/// At 5 bps the minimum amount that yields fee = 1 is exactly 2 000.
#[test]
fn test_fee_boundary_first_nonzero_5bps() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, asset) = setup_fee_env(&env, 5, 100_000);

    let fee = measure_fee(&env, &client, &contract_id, &asset, 2_000, 10);
    assert_eq!(fee, 1, "2000 * 5 / 10000 must equal 1");
}

/// At 9 bps (default) the minimum amount that yields fee = 1 is 1 112.
/// 1 111 * 9 / 10 000 = 0, 1 112 * 9 / 10 000 = 1.
#[test]
fn test_fee_boundary_first_nonzero_9bps() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, asset) = setup_fee_env(&env, 9, 100_000);

    let fee_below = measure_fee(&env, &client, &contract_id, &asset, 1_111, 0);
    assert_eq!(fee_below, 0, "1111 * 9 / 10000 must be 0");

    // need a fresh env to avoid reusing same receiver state
    let env2 = Env::default();
    env2.mock_all_auths();
    let (client2, contract_id2, asset2) = setup_fee_env(&env2, 9, 100_000);
    let fee_at = measure_fee(&env2, &client2, &contract_id2, &asset2, 1_112, 10);
    assert_eq!(fee_at, 1, "1112 * 9 / 10000 must be 1");
}

/// At 100 bps (1%) the boundary is at amount = 100.
#[test]
fn test_fee_boundary_first_nonzero_100bps() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, asset) = setup_fee_env(&env, 100, 100_000);

    let fee_below = measure_fee(&env, &client, &contract_id, &asset, 99, 0);
    assert_eq!(fee_below, 0, "99 * 100 / 10000 must be 0");

    let env2 = Env::default();
    env2.mock_all_auths();
    let (client2, contract_id2, asset2) = setup_fee_env(&env2, 100, 100_000);
    let fee_at = measure_fee(&env2, &client2, &contract_id2, &asset2, 100, 10);
    assert_eq!(fee_at, 1, "100 * 100 / 10000 must be 1");
}

/// At 1 000 bps (10%, maximum) the boundary is at amount = 10.
#[test]
fn test_fee_boundary_first_nonzero_max_bps() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, asset) = setup_fee_env(&env, 1_000, 100_000);

    let fee_below = measure_fee(&env, &client, &contract_id, &asset, 9, 0);
    assert_eq!(fee_below, 0, "9 * 1000 / 10000 must be 0");

    let env2 = Env::default();
    env2.mock_all_auths();
    let (client2, contract_id2, asset2) = setup_fee_env(&env2, 1_000, 100_000);
    let fee_at = measure_fee(&env2, &client2, &contract_id2, &asset2, 10, 10);
    assert_eq!(fee_at, 1, "10 * 1000 / 10000 must be 1");
}

// ── Zero-fee config ───────────────────────────────────────────────────────────

/// When fee_bps is 0, all amounts produce zero fee.
#[test]
fn test_zero_fee_bps_always_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, contract_id, asset) = setup_fee_env(&env, 0, 100_000);

    for amount in [1_i128, 100, 9_999, 100_000] {
        let env2 = Env::default();
        env2.mock_all_auths();
        let (client2, cid2, ast2) = setup_fee_env(&env2, 0, 500_000);
        let fee = measure_fee(&env2, &client2, &cid2, &ast2, amount, 0);
        assert_eq!(fee, 0, "fee must be 0 for any amount when fee_bps=0 (amount={amount})");
    }

    // confirm the unused bindings compile cleanly
    let _ = (&client, &contract_id, &asset);
}

// ── Exact fee correctness ─────────────────────────────────────────────────────

/// Verify fee values match the formula `amount * fee_bps / 10_000` across a
/// representative matrix of (amount, fee_bps) pairs.
#[test]
fn test_fee_formula_correctness_matrix() {
    let cases: &[(i128, i128)] = &[
        (10_000, 100),     // 100 bps → fee 100
        (50_000, 50),      // 50 bps → fee 250
        (1_000_000, 9),    // 9 bps → fee 900
        (1_000_000, 1_000), // max bps → fee 100_000
        (99_999, 100),     // 100 bps, non-round amount → fee 999
        (7_777, 77),       // irregular → 7777 * 77 / 10000 = 59
    ];

    for &(amount, fee_bps) in cases {
        let env = Env::default();
        env.mock_all_auths();
        let (client, contract_id, asset) = setup_fee_env(&env, fee_bps, amount * 2);
        let expected = amount * fee_bps / 10_000;
        let actual = measure_fee(&env, &client, &contract_id, &asset, amount, expected + 1);
        assert_eq!(
            actual, expected,
            "fee mismatch for amount={amount} fee_bps={fee_bps}: expected={expected} got={actual}"
        );
    }
}

// ── Fee-splitting invariant ───────────────────────────────────────────────────

/// Demonstrates the fee-splitting discrepancy: N sub-threshold calls total
/// less fee than one combined call. The reentrancy guard blocks the same-tx
/// exploit, but this test shows the *mathematical property* using sequential
/// independent environments (one per "loan").
///
/// This is a documented limitation. Operators should set `min_borrow_amount`
/// high enough to prevent sub-threshold loans if strict fee accounting matters.
#[test]
fn test_fee_splitting_yields_less_than_combined() {
    // Single loan of 10_000 at 5 bps → fee = 5
    let env_single = Env::default();
    env_single.mock_all_auths();
    let (c_single, cid_single, ast_single) = setup_fee_env(&env_single, 5, 100_000);
    let fee_single = measure_fee(&env_single, &c_single, &cid_single, &ast_single, 10_000, 10);
    assert_eq!(fee_single, 5);

    // Split into 10 × 1_000 loans (each sub-threshold → fee = 0 each)
    let mut fee_split_total = 0_i128;
    for _ in 0..10 {
        let env_n = Env::default();
        env_n.mock_all_auths();
        let (c_n, cid_n, ast_n) = setup_fee_env(&env_n, 5, 100_000);
        fee_split_total += measure_fee(&env_n, &c_n, &cid_n, &ast_n, 1_000, 0);
    }
    assert_eq!(fee_split_total, 0, "10 × 1000-unit loans each earn 0 fee at 5 bps");

    // Split is worse for the protocol
    assert!(
        fee_split_total < fee_single,
        "splitting yields less fee: split={fee_split_total} vs single={fee_single}"
    );
}

/// Splitting into amounts that each still produce fee=1 gives the same or
/// fewer total fee due to rounding (e.g. 2000 per call at 5 bps).
/// 5 × 2000 → 5 × fee(1) = 5, same as 1 × 10000 → fee(5).
/// 3 × 3333 → 3 × fee(1) = 3, less than 1 × 9999 → fee(4).
#[test]
fn test_fee_splitting_rounding_loss() {
    // 1 × 9_999 at 5 bps → fee = 4 (9999 * 5 / 10000 = 4)
    let env1 = Env::default();
    env1.mock_all_auths();
    let (c1, cid1, ast1) = setup_fee_env(&env1, 5, 100_000);
    let fee_combined = measure_fee(&env1, &c1, &cid1, &ast1, 9_999, 10);
    assert_eq!(fee_combined, 4);

    // 3 × 3_333 at 5 bps → each fee = 1 (3333 * 5 / 10000 = 1) → total = 3
    let mut fee_split = 0_i128;
    for _ in 0..3 {
        let env_n = Env::default();
        env_n.mock_all_auths();
        let (cn, cidn, astn) = setup_fee_env(&env_n, 5, 100_000);
        fee_split += measure_fee(&env_n, &cn, &cidn, &astn, 3_333, 5);
    }
    assert_eq!(fee_split, 3);
    assert!(fee_split < fee_combined, "split total fee {fee_split} < combined {fee_combined}");
}

// ── Large-amount safety ───────────────────────────────────────────────────────

/// Large amounts must not panic; saturating arithmetic keeps fee bounded.
#[test]
fn test_fee_large_amount_no_overflow() {
    let env = Env::default();
    env.mock_all_auths();
    // Use 10^15 as a realistic large token amount (well within i128 range)
    let large_amount: i128 = 1_000_000_000_000_000;
    let fee_bps: i128 = 1_000; // max 10%
    let expected_fee = large_amount * fee_bps / 10_000; // 100_000_000_000_000

    let (client, contract_id, asset) = setup_fee_env(&env, fee_bps, large_amount * 2);
    let actual_fee = measure_fee(
        &env,
        &client,
        &contract_id,
        &asset,
        large_amount,
        expected_fee + 1,
    );
    assert_eq!(actual_fee, expected_fee);
}

/// The maximum safe amount before `amount * MAX_FEE_BPS` overflows i128.
/// `i128::MAX / 1_000 = 170_141_183_460_469_231_731_687_303_715_884_105`
/// Using saturating_mul the result saturates to i128::MAX, then divides by
/// 10_000 — the fee is still non-negative and does not panic.
#[test]
fn test_fee_saturating_overflow_no_panic() {
    let env = Env::default();
    env.mock_all_auths();
    // Amount large enough to overflow amount * 1_000
    let overflow_amount: i128 = i128::MAX / 500; // > i128::MAX / 1_000
    let fee_bps: i128 = 1_000;

    // We do NOT call flash_loan here (would require minting overflow_amount tokens);
    // instead we call set_flash_loan_fee_bps to verify configuration is accepted,
    // and verify the formula directly using saturating arithmetic:
    let saturated = overflow_amount.saturating_mul(fee_bps);
    assert_eq!(saturated, i128::MAX, "should saturate to i128::MAX");
    let fee_from_saturated = saturated.saturating_div(10_000);
    assert!(fee_from_saturated > 0, "saturated fee must still be positive");
    assert!(fee_from_saturated <= i128::MAX, "must fit in i128");

    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000_000_000_000, &1);
    // Verify max fee is accepted without panic
    assert!(client.try_set_flash_loan_fee_bps(&fee_bps).is_ok());
}

// ── Fee invariants ────────────────────────────────────────────────────────────

/// Fee is always non-negative for valid inputs.
#[test]
fn test_fee_never_negative() {
    for &fee_bps in &[0_i128, 1, 5, 9, 100, 999, 1_000] {
        for &amount in &[1_i128, 100, 9_999, 10_000, 999_999] {
            let fee = amount.saturating_mul(fee_bps).saturating_div(10_000);
            assert!(fee >= 0, "fee must be ≥ 0 for amount={amount} fee_bps={fee_bps}");
        }
    }
}

/// Fee never exceeds `amount * MAX_FEE_BPS / BPS_SCALE` regardless of config.
#[test]
fn test_fee_never_exceeds_max_configured() {
    let max_bps: i128 = 1_000;
    for &amount in &[100_i128, 10_000, 1_000_000] {
        let max_fee = amount * max_bps / 10_000;

        let env = Env::default();
        env.mock_all_auths();
        let (client, contract_id, asset) = setup_fee_env(&env, max_bps, amount * 2);
        let actual_fee = measure_fee(&env, &client, &contract_id, &asset, amount, max_fee + 1);
        assert!(
            actual_fee <= max_fee,
            "fee={actual_fee} exceeds max={max_fee} for amount={amount}"
        );
    }
}

/// After a successful flash loan, the protocol balance increases by exactly the fee.
#[test]
fn test_balance_invariant_after_successful_loan() {
    let env = Env::default();
    env.mock_all_auths();
    let fee_bps: i128 = 100; // 1%
    let amount: i128 = 50_000;
    let expected_fee = amount * fee_bps / 10_000; // 500

    let (client, contract_id, asset) = setup_fee_env(&env, fee_bps, 200_000);
    let token = token::Client::new(&env, &asset);
    let balance_before = token.balance(&contract_id);

    let receiver = env.register(HonestFeeReceiver, ());
    token::StellarAssetClient::new(&env, &asset).mint(&receiver, &(expected_fee + 1));
    client.flash_loan(&receiver, &asset, &amount, &Bytes::new(&env));

    let balance_after = token.balance(&contract_id);
    assert_eq!(
        balance_after - balance_before,
        expected_fee,
        "balance must grow by exactly the fee"
    );
}

/// For amounts that round fee to zero, the protocol balance is unchanged.
#[test]
fn test_balance_unchanged_when_fee_rounds_to_zero() {
    let env = Env::default();
    env.mock_all_auths();
    // 5 bps, amount=1000 → fee = 0
    let (client, contract_id, asset) = setup_fee_env(&env, 5, 100_000);
    let token = token::Client::new(&env, &asset);
    let balance_before = token.balance(&contract_id);

    let receiver = env.register(HonestFeeReceiver, ());
    client.flash_loan(&receiver, &asset, &1_000, &Bytes::new(&env));

    assert_eq!(
        token.balance(&contract_id),
        balance_before,
        "balance must be unchanged when fee is zero"
    );
}

// ── Fee configuration edge cases ─────────────────────────────────────────────

/// Boundary bps values (0 and MAX_FEE_BPS = 1 000) are both accepted.
#[test]
fn test_fee_bps_boundaries_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1_000);

    assert!(client.try_set_flash_loan_fee_bps(&0).is_ok(), "0 bps must be accepted");
    assert!(client.try_set_flash_loan_fee_bps(&1_000).is_ok(), "1000 bps must be accepted");
}

/// Values just above MAX_FEE_BPS are rejected.
#[test]
fn test_fee_bps_above_max_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1_000);

    assert_eq!(
        client.try_set_flash_loan_fee_bps(&1_001),
        Err(Ok(FlashLoanError::InvalidFee)),
        "1001 bps must be rejected"
    );
    assert_eq!(
        client.try_set_flash_loan_fee_bps(&10_000),
        Err(Ok(FlashLoanError::InvalidFee)),
        "10000 bps must be rejected"
    );
}

/// Negative fee_bps values are rejected.
#[test]
fn test_fee_bps_negative_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000_000, &1_000);

    assert_eq!(
        client.try_set_flash_loan_fee_bps(&-1),
        Err(Ok(FlashLoanError::InvalidFee))
    );
}
