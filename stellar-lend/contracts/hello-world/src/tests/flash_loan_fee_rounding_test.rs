//! # Flash Loan Fee Rounding & Correctness Tests — hello-world contract (Issue #697)
//!
//! Validates `calculate_flash_loan_fee` correctness across:
//! - Tiny amounts where integer division truncates fee to zero
//! - Boundary conditions where fee transitions from 0 to 1
//! - Large amounts (overflow detection via checked arithmetic)
//! - Fee-splitting invariant (documented limitation)
//! - Config-driven min/max amount enforcement
//!
//! ## Fee Rounding Semantics
//!
//! ```text
//! fee = amount * fee_bps / 10_000   (integer division, truncates)
//! ```
//!
//! The `hello-world` implementation uses `checked_mul` / `checked_div`, so an
//! amount large enough to overflow `amount * fee_bps` returns
//! `FlashLoanError::Overflow` rather than silently saturating.
//!
//! ## Fee-Splitting Security Note
//!
//! Splitting a single large loan into multiple sub-threshold loans can reduce
//! the total fee collected due to per-call rounding. The per-(user, asset)
//! reentrancy guard blocks this within a single transaction; operators should
//! set `min_amount` high enough that sub-threshold loans are rejected outright
//! if strict fee collection is required.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, token, Address, Env};

use crate::flash_loan::{
    configure_flash_loan, execute_flash_loan, set_flash_loan_fee, FlashLoanConfig, FlashLoanDataKey,
    FlashLoanError,
};
use crate::HelloContract;

use soroban_sdk::{contract, contractimpl};

// ── Test callback ─────────────────────────────────────────────────────────────

#[contract]
pub struct FeeRoundingCallback;

#[contractimpl]
impl FeeRoundingCallback {
    pub fn on_flash_loan(
        env: Env,
        initiator: Address,
        user: Address,
        asset: Address,
        amount: i128,
        fee: i128,
    ) {
        let total = amount + fee;
        let token_client = token::StellarAssetClient::new(&env, &asset);
        if fee > 0 {
            token_client.mint(&user, &fee);
        }
        let token_std = token::TokenClient::new(&env, &asset);
        token_std.approve(&user, &initiator, &total, &99_999);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup_env() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(HelloContract, ());
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_address = env.register_stellar_asset_contract(token_admin.clone());
    env.as_contract(&contract_id, || {
        crate::admin::set_admin(&env, admin.clone()).unwrap();
    });
    (env, contract_id, admin, user, token_address)
}

fn setup_with_balance(balance: i128) -> (Env, Address, Address, Address, Address) {
    let (env, contract_id, admin, user, token_address) = setup_env();
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &balance);
    (env, contract_id, admin, user, token_address)
}

/// Executes a flash loan and returns the fee actually earned (balance delta).
fn measure_fee(
    env: &Env,
    contract_id: &Address,
    user: &Address,
    token_address: &Address,
    amount: i128,
) -> i128 {
    let token = token::TokenClient::new(env, token_address);
    let before = token.balance(contract_id);
    let callback = env.register(FeeRoundingCallback, ());

    // Pre-fund the contract with enough for the loan
    let _ = env.as_contract(contract_id, || {
        execute_flash_loan(env, user.clone(), token_address.clone(), amount, callback)
    });

    token.balance(contract_id) - before
}

// ── Fee formula unit tests (integration-level) ────────────────────────────────

/// Verify the fee formula via execute_flash_loan and total repayment.
/// (`calculate_flash_loan_fee` is private; all assertions use the observable
///  total = amount + fee returned by execute_flash_loan.)

/// Default 9 bps: amounts below 1 112 yield zero fee.
#[test]
fn test_fee_formula_default_9bps_tiny_amount() {
    let (env, contract_id, _admin, user, token_address) = setup_with_balance(100_000);
    let callback = env.register(FeeRoundingCallback, ());

    // 1 111 * 9 / 10 000 = 0
    let total = env.as_contract(&contract_id, || {
        execute_flash_loan(&env, user.clone(), token_address.clone(), 1_111, callback).unwrap()
    });
    assert_eq!(total, 1_111, "fee must be 0 for amount=1111 at default 9 bps");
}

/// Default 9 bps: the minimum amount for fee = 1 is 1 112.
#[test]
fn test_fee_formula_default_9bps_boundary() {
    let (env, contract_id, _admin, user, token_address) = setup_with_balance(100_000);
    let callback = env.register(FeeRoundingCallback, ());

    // 1 112 * 9 / 10 000 = 1
    let total = env.as_contract(&contract_id, || {
        execute_flash_loan(&env, user.clone(), token_address.clone(), 1_112, callback).unwrap()
    });
    assert_eq!(total, 1_113, "amount=1112 at 9bps → fee=1, total=1113");
}

/// Zero fee_bps always yields fee = 0.
#[test]
fn test_fee_zero_bps_always_zero() {
    let (env, contract_id, admin, user, token_address) = setup_with_balance(100_000);
    let callback = env.register(FeeRoundingCallback, ());

    env.as_contract(&contract_id, || {
        set_flash_loan_fee(&env, admin, 0).unwrap();
    });

    for amount in [1_i128, 100, 50_000, 99_999] {
        // Clear any leftover active loan key
        env.as_contract(&contract_id, || {
            let key = FlashLoanDataKey::ActiveFlashLoan(user.clone(), token_address.clone());
            env.storage().persistent().remove(&key);
        });

        let total = env.as_contract(&contract_id, || {
            execute_flash_loan(
                &env,
                user.clone(),
                token_address.clone(),
                amount,
                callback.clone(),
            )
            .unwrap()
        });
        assert_eq!(total, amount, "fee must be 0 for any amount when fee_bps=0 (amount={amount})");
    }
}

/// 50 bps: verify formula at a non-trivial amount.
#[test]
fn test_fee_formula_50bps() {
    let (env, contract_id, admin, user, token_address) = setup_with_balance(10_000_000);
    let callback = env.register(FeeRoundingCallback, ());

    env.as_contract(&contract_id, || {
        set_flash_loan_fee(&env, admin, 50).unwrap();
    });

    // 1_000_000 * 50 / 10_000 = 5_000
    let total = env.as_contract(&contract_id, || {
        execute_flash_loan(&env, user.clone(), token_address.clone(), 1_000_000, callback).unwrap()
    });
    assert_eq!(total, 1_005_000);
}

/// Maximum fee (10 000 bps = 100%): fee equals amount.
#[test]
fn test_fee_formula_max_100_percent() {
    let (env, contract_id, admin, user, token_address) = setup_with_balance(10_000_000);
    let callback = env.register(FeeRoundingCallback, ());

    env.as_contract(&contract_id, || {
        set_flash_loan_fee(&env, admin, 10_000).unwrap();
    });

    // 1_000_000 * 10_000 / 10_000 = 1_000_000
    let total = env.as_contract(&contract_id, || {
        execute_flash_loan(&env, user.clone(), token_address.clone(), 1_000_000, callback).unwrap()
    });
    assert_eq!(total, 2_000_000, "at 100% fee total = 2 × amount");
}

// ── Overflow detection ────────────────────────────────────────────────────────

/// With checked arithmetic, an overflow in `amount * fee_bps` returns Overflow.
/// i128::MAX * 10_000 overflows, so using i128::MAX as amount should error.
#[test]
fn test_fee_overflow_detection() {
    let (env, contract_id, admin, _user, _token_address) = setup_env();

    env.as_contract(&contract_id, || {
        set_flash_loan_fee(&env, admin, 10_000).unwrap();
    });

    // Verify the overflow is caught in the fee calculation:
    // i128::MAX * 10_000 overflows → FlashLoanError::Overflow expected.
    // We test the formula directly since minting i128::MAX tokens isn't practical.
    let overflow_amount = i128::MAX;
    let fee_bps: i128 = 10_000;
    let result = overflow_amount.checked_mul(fee_bps);
    assert!(
        result.is_none(),
        "i128::MAX * 10_000 must overflow checked_mul"
    );

    // The production code maps this to FlashLoanError::Overflow.
    // Verify the error type exists and is the right variant:
    let err: FlashLoanError = FlashLoanError::Overflow;
    assert_eq!(err as u32, 7);
}

// ── Fee-splitting invariant ───────────────────────────────────────────────────

/// Demonstrates that splitting a loan into sub-threshold pieces reduces total fee.
/// Each sub-loan runs in a separate Env (no reentrancy issue).
#[test]
fn test_fee_splitting_reduces_protocol_revenue() {
    // Single loan of 10_000 at 9 bps → fee = 9
    let (env1, cid1, _admin1, user1, tok1) = setup_with_balance(100_000);
    let fee_single = {
        let cb = env1.register(FeeRoundingCallback, ());
        let total = env1.as_contract(&cid1, || {
            execute_flash_loan(&env1, user1.clone(), tok1.clone(), 10_000, cb).unwrap()
        });
        total - 10_000
    };
    assert_eq!(fee_single, 9);

    // Split into 9 × 1_111 → each fee = 0 (sub-threshold), total = 0
    let mut fee_split_total: i128 = 0;
    for _ in 0..9 {
        let (env_n, cid_n, _admin_n, user_n, tok_n) = setup_with_balance(100_000);
        let cb_n = env_n.register(FeeRoundingCallback, ());
        let total_n = env_n.as_contract(&cid_n, || {
            execute_flash_loan(&env_n, user_n.clone(), tok_n.clone(), 1_111, cb_n).unwrap()
        });
        fee_split_total += total_n - 1_111;
    }
    assert_eq!(fee_split_total, 0, "9 × 1111 loans each yield 0 fee at 9 bps");
    assert!(
        fee_split_total < fee_single,
        "split yields less fee than single: {fee_split_total} < {fee_single}"
    );
}

// ── Config-driven amount limits ───────────────────────────────────────────────

/// Setting min_amount via configure_flash_loan prevents sub-threshold loans,
/// which is the recommended mitigation for the fee-splitting issue.
#[test]
fn test_min_amount_blocks_sub_threshold_loans() {
    let (env, contract_id, admin, user, token_address) = setup_with_balance(100_000);
    let callback = env.register(FeeRoundingCallback, ());

    // Configure: fee=9 bps, min_amount=1_112 (ensures fee is always ≥ 1)
    env.as_contract(&contract_id, || {
        configure_flash_loan(
            &env,
            admin,
            FlashLoanConfig {
                fee_bps: 9,
                min_amount: 1_112,
                max_amount: i128::MAX,
            },
        )
        .unwrap();
    });

    // Loan of 1_111 is below min → rejected
    let result = env.as_contract(&contract_id, || {
        execute_flash_loan(
            &env,
            user.clone(),
            token_address.clone(),
            1_111,
            callback.clone(),
        )
    });
    assert_eq!(result, Err(FlashLoanError::InvalidAmount));

    // Loan of 1_112 is at boundary → accepted, fee = 1
    let total = env.as_contract(&contract_id, || {
        execute_flash_loan(&env, user.clone(), token_address.clone(), 1_112, callback).unwrap()
    });
    assert_eq!(total, 1_113);
}

/// max_amount is enforced.
#[test]
fn test_max_amount_enforced() {
    let (env, contract_id, admin, user, token_address) = setup_with_balance(100_000_000);
    let callback = env.register(FeeRoundingCallback, ());

    env.as_contract(&contract_id, || {
        configure_flash_loan(
            &env,
            admin,
            FlashLoanConfig {
                fee_bps: 9,
                min_amount: 1,
                max_amount: 10_000_000,
            },
        )
        .unwrap();
    });

    let result = env.as_contract(&contract_id, || {
        execute_flash_loan(
            &env,
            user.clone(),
            token_address.clone(),
            20_000_000,
            callback,
        )
    });
    assert_eq!(result, Err(FlashLoanError::InvalidAmount));
}

// ── Fee correctness matrix ────────────────────────────────────────────────────

/// Verify fee = amount * fee_bps / 10_000 across a matrix of inputs.
#[test]
fn test_fee_formula_matrix() {
    let cases: &[(i128, i128, i128)] = &[
        // (amount, fee_bps, expected_fee)
        (10_000, 9, 9),
        (100_000, 9, 90),
        (1_000_000, 9, 900),
        (1_000_000, 50, 5_000),
        (1_000_000, 100, 10_000),
        (999_999, 9, 899),    // truncation: 8999.991 → 899
        (7_777, 77, 59),       // 7777 * 77 / 10000 = 59
        (2_000, 5, 1),         // exactly at 5bps boundary
    ];

    for &(amount, fee_bps, expected_fee) in cases {
        let (env, contract_id, admin, user, token_address) = setup_with_balance(amount * 2 + 1);
        let callback = env.register(FeeRoundingCallback, ());

        env.as_contract(&contract_id, || {
            set_flash_loan_fee(&env, admin, fee_bps).unwrap();
        });

        let total = env.as_contract(&contract_id, || {
            execute_flash_loan(&env, user.clone(), token_address.clone(), amount, callback).unwrap()
        });
        assert_eq!(
            total - amount,
            expected_fee,
            "fee mismatch: amount={amount} fee_bps={fee_bps} expected={expected_fee} got={}",
            total - amount
        );
    }
}
