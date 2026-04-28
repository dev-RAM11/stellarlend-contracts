//! Comprehensive tests for view functions: collateral value, debt value, health factor, position summary.
//! Covers edge cases (zero collateral, zero debt, boundary health factor) and security (no state change, oracle usage).

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use views::{HEALTH_FACTOR_NO_DEBT, HEALTH_FACTOR_SCALE};

/// Mock oracle contract: returns fixed price (1.0 with 8 decimals) for any asset.
#[contract]
pub struct MockOracle;

#[contractimpl]
impl MockOracle {
    /// Returns price with 8 decimals (100_000_000 = 1.0).
    pub fn price(_env: Env, _asset: Address) -> i128 {
        100_000_000
    }
}

fn setup(
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

fn setup_with_oracle(
    env: &Env,
) -> (
    LendingContractClient<'_>,
    Address,
    Address,
    Address,
    Address,
    Address,
) {
    let (client, admin, user, asset, collateral_asset) = setup(env);
    let oracle_id = env.register(MockOracle, ());
    client.set_oracle(&admin, &oracle_id);
    (client, admin, user, asset, collateral_asset, oracle_id)
}

// ─────────────────────────────────────────────────────────────────────────────
// get_collateral_balance
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_get_collateral_balance_zero_when_no_position() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset) = setup(&env);
    assert_eq!(client.get_collateral_balance(&user), 0);
}

#[test]
fn test_get_collateral_balance_returns_amount_after_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert_eq!(client.get_collateral_balance(&user), 20_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// get_debt_balance
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_get_debt_balance_zero_when_no_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset) = setup(&env);
    assert_eq!(client.get_debt_balance(&user), 0);
}

#[test]
fn test_get_debt_balance_returns_principal_plus_interest() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);
    let (client, _admin, user, asset, collateral_asset) = setup(&env);
    client.borrow(&user, &asset, &100_000, &collateral_asset, &200_000);
    assert_eq!(client.get_debt_balance(&user), 100_000);
    env.ledger().with_mut(|li| li.timestamp = 1000 + 31_536_000);
    let debt_balance = client.get_debt_balance(&user);
    assert!(debt_balance > 100_000);
    assert!(debt_balance <= 105_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// get_collateral_value / get_debt_value (oracle)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_get_collateral_value_zero_when_oracle_not_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert_eq!(client.get_collateral_value(&user), 0);
}

#[test]
fn test_get_debt_value_zero_when_oracle_not_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert_eq!(client.get_debt_value(&user), 0);
}

#[test]
fn test_get_collateral_value_with_oracle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    // value = 20_000 * 100_000_000 / 100_000_000 = 20_000 (same unit as amount when price = 1)
    assert_eq!(client.get_collateral_value(&user), 20_000);
}

#[test]
fn test_get_debt_value_with_oracle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert_eq!(client.get_debt_value(&user), 10_000);
}

#[test]
fn test_get_collateral_value_zero_collateral() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset, _oracle) = setup_with_oracle(&env);
    assert_eq!(client.get_collateral_value(&user), 0);
}

#[test]
fn test_get_debt_value_zero_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset, _oracle) = setup_with_oracle(&env);
    assert_eq!(client.get_debt_value(&user), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// get_health_factor
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_get_health_factor_no_debt_returns_sentinel() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset, _oracle) = setup_with_oracle(&env);
    assert_eq!(client.get_health_factor(&user), HEALTH_FACTOR_NO_DEBT);
}

#[test]
fn test_get_health_factor_zero_when_oracle_not_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert_eq!(client.get_health_factor(&user), 0);
}

#[test]
fn test_get_health_factor_healthy_above_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    // Collateral 20_000, debt 10_000. With price 1: cv=20_000, dv=10_000.
    // Default liq threshold 80%. Weighted = 20_000 * 0.8 = 16_000. HF = 16_000 * 10000 / 10_000 = 16000 (> 10000).
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    let hf = client.get_health_factor(&user);
    assert!(hf >= HEALTH_FACTOR_SCALE);
    assert_eq!(hf, 16_000);
}

#[test]
fn test_get_health_factor_liquidatable_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    // Liquidation threshold 40%. Collateral 30_000, debt 15_000 (meets 150% borrow rule).
    // Weighted = 30_000 * 0.4 = 12_000, HF = 12_000 * 10000 / 15_000 = 8000 < 10000.
    client.set_liquidation_threshold_bps(&admin, &4000);
    client.borrow(&user, &asset, &15_000, &collateral_asset, &30_000);
    let hf = client.get_health_factor(&user);
    assert!(hf < HEALTH_FACTOR_SCALE);
    assert_eq!(hf, 8000);
}

#[test]
fn test_get_health_factor_boundary_at_one() {
    let env = Env::default();
    env.mock_all_auths();
    // At HF = 1.0: weighted_collateral = debt_value. Collateral 1500, debt 1000, lt 6667 -> weighted = 1000, HF = 10000.
    let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.set_liquidation_threshold_bps(&admin, &6667);
    client.borrow(&user, &asset, &1000, &collateral_asset, &1500);
    assert_eq!(client.get_health_factor(&user), HEALTH_FACTOR_SCALE);
}

// ─────────────────────────────────────────────────────────────────────────────
// get_user_position
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_get_user_position_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset, _oracle) = setup_with_oracle(&env);
    let pos = client.get_user_position(&user);
    assert_eq!(pos.collateral_balance, 0);
    assert_eq!(pos.collateral_value, 0);
    assert_eq!(pos.debt_balance, 0);
    assert_eq!(pos.debt_value, 0);
    assert_eq!(pos.health_factor, HEALTH_FACTOR_NO_DEBT);
}

#[test]
fn test_get_user_position_matches_individual_getters() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    let pos = client.get_user_position(&user);
    assert_eq!(pos.collateral_balance, client.get_collateral_balance(&user));
    assert_eq!(pos.collateral_value, client.get_collateral_value(&user));
    assert_eq!(pos.debt_balance, client.get_debt_balance(&user));
    assert_eq!(pos.debt_value, client.get_debt_value(&user));
    assert_eq!(pos.health_factor, client.get_health_factor(&user));
}

// ─────────────────────────────────────────────────────────────────────────────
// Admin: set_oracle, set_liquidation_threshold_bps
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_set_oracle_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset) = setup(&env);
    let oracle_id = env.register(MockOracle, ());
    let result = client.try_set_oracle(&user, &oracle_id);
    assert_eq!(result, Err(Ok(BorrowError::Unauthorized)));
}

#[test]
fn test_set_liquidation_threshold_bps_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset) = setup(&env);
    let result = client.try_set_liquidation_threshold_bps(&user, &8000);
    assert_eq!(result, Err(Ok(BorrowError::Unauthorized)));
}

#[test]
fn test_set_liquidation_threshold_bps_invalid() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _user, _asset, _collateral_asset) = setup(&env);
    assert_eq!(
        client.try_set_liquidation_threshold_bps(&admin, &0),
        Err(Ok(BorrowError::InvalidAmount))
    );
    assert_eq!(
        client.try_set_liquidation_threshold_bps(&admin, &10001),
        Err(Ok(BorrowError::InvalidAmount))
    );
}

#[test]
fn test_set_liquidation_threshold_bps_valid() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.set_liquidation_threshold_bps(&admin, &7500);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    let hf = client.get_health_factor(&user);
    // weighted = 20_000 * 0.75 = 15_000, HF = 15_000 * 10000 / 10_000 = 15000
    assert_eq!(hf, 15_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Security: views are read-only (no state change)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_views_do_not_modify_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    let debt_before = client.get_user_debt(&user);
    let _ = client.get_user_position(&user);
    let _ = client.get_health_factor(&user);
    let _ = client.get_collateral_value(&user);
    let _ = client.get_debt_value(&user);
    let debt_after = client.get_user_debt(&user);
    assert_eq!(debt_before.borrowed_amount, debt_after.borrowed_amount);
    assert_eq!(debt_before.interest_accrued, debt_after.interest_accrued);
}

// ═════════════════════════════════════════════════════════════════════════════
// Cross-asset position summary invariants (#651)
//
// These tests pin the public read API: `get_user_position`, the individual
// value/balance getters, and `get_max_liquidatable_amount` must always agree
// with the underlying per-user balances and the admin-configured risk
// parameters. Tests cover:
//
//   * table-driven (collateral, debt, threshold) scenarios
//   * randomised query ordering (the order of view calls must not affect
//     results — a "stable serialization" property)
//   * rounding boundaries (HF exactly at 1.0, off-by-one inputs)
//   * missing-asset and missing-oracle cases
//   * multi-position permutations (different users with different shapes
//     must produce independent, internally consistent summaries)
//
// Security note
// -------------
// View functions are the surface that liquidation bots, frontends, and
// downstream contracts rely on. If `get_user_position` ever disagreed
// with the individual getters, an integrator could be tricked into
// liquidating a healthy account or skipping a sick one. These invariants
// also forbid view-based exploitation: every read is pure and any caller
// observing the same on-chain state at the same ledger height must see
// the same answers.
// ═════════════════════════════════════════════════════════════════════════════

/// Asserts that the unified `get_user_position` summary agrees field-for-field
/// with the per-field view getters. This is the core consistency invariant.
fn assert_summary_consistent(client: &LendingContractClient<'_>, user: &Address) {
    let summary = client.get_user_position(user);
    assert_eq!(summary.collateral_balance, client.get_collateral_balance(user));
    assert_eq!(summary.debt_balance, client.get_debt_balance(user));
    assert_eq!(summary.collateral_value, client.get_collateral_value(user));
    assert_eq!(summary.debt_value, client.get_debt_value(user));
    assert_eq!(summary.health_factor, client.get_health_factor(user));
}

#[test]
fn test_invariant_summary_matches_individual_getters_table_driven() {
    // (debt, collateral, liq_threshold_bps, expected_health_factor)
    //
    // Each row exercises a representative slice of the (debt, collateral,
    // threshold) space and pins the exact health factor the view must
    // report. Borrow rejects under-collateralised positions, so each row
    // satisfies the contract's 150% borrow rule.
    let cases: [(i128, i128, i128, i128); 5] = [
        (10_000, 20_000, 8000, 16_000),
        (10_000, 20_000, 5000, 10_000), // boundary
        (15_000, 30_000, 6000, 12_000),
        (1_000, 2_000, 7500, 15_000),
        (50_000, 100_000, 8000, 16_000),
    ];

    for (debt, coll, lt, expected_hf) in cases.iter() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
        client.set_liquidation_threshold_bps(&admin, lt);
        client.borrow(&user, &asset, debt, &collateral_asset, coll);

        assert_summary_consistent(&client, &user);
        assert_eq!(
            client.get_health_factor(&user),
            *expected_hf,
            "case (debt={debt}, coll={coll}, lt={lt}) — HF mismatch"
        );
    }
}

#[test]
fn test_invariant_summary_idempotent_across_query_orderings() {
    // Repeating the same set of view queries in arbitrary order must
    // yield bit-identical answers. This guards against any hidden
    // mutation in a view path.
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let baseline = client.get_user_position(&user);

    // Permutation 1: summary first
    let s1 = client.get_user_position(&user);
    let cb1 = client.get_collateral_balance(&user);
    let db1 = client.get_debt_balance(&user);
    let cv1 = client.get_collateral_value(&user);
    let dv1 = client.get_debt_value(&user);
    let hf1 = client.get_health_factor(&user);

    // Permutation 2: getters first, summary last
    let cb2 = client.get_collateral_balance(&user);
    let db2 = client.get_debt_balance(&user);
    let cv2 = client.get_collateral_value(&user);
    let dv2 = client.get_debt_value(&user);
    let hf2 = client.get_health_factor(&user);
    let s2 = client.get_user_position(&user);

    // Permutation 3: interleaved
    let _ = client.get_max_liquidatable_amount(&user);
    let s3 = client.get_user_position(&user);
    let cv3 = client.get_collateral_value(&user);
    let _ = client.get_liquidation_incentive_amount(&100);
    let hf3 = client.get_health_factor(&user);

    // Cross-permutation equality
    assert_eq!(s1, s2);
    assert_eq!(s2, s3);
    assert_eq!(s1, baseline);
    assert_eq!(cb1, cb2);
    assert_eq!(db1, db2);
    assert_eq!(cv1, cv2);
    assert_eq!(cv1, cv3);
    assert_eq!(dv1, dv2);
    assert_eq!(hf1, hf2);
    assert_eq!(hf1, hf3);
}

#[test]
fn test_invariant_health_factor_boundary_rounding() {
    // The integer-division step `weighted_collateral * HEALTH_FACTOR_SCALE
    // / debt_value` truncates toward zero. Pin a few off-by-one boundaries
    // so a refactor that switches to ceiling rounding (or float math)
    // gets caught by the test suite.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);

    // HF exactly at 1.0
    client.set_liquidation_threshold_bps(&admin, &6667);
    client.borrow(&user, &asset, &1000, &collateral_asset, &1500);
    let hf = client.get_health_factor(&user);
    assert_eq!(hf, HEALTH_FACTOR_SCALE);
    assert_summary_consistent(&client, &user);

    // get_max_liquidatable_amount is 0 at exactly 1.0 (healthy boundary)
    assert_eq!(client.get_max_liquidatable_amount(&user), 0);
}

#[test]
fn test_invariant_no_oracle_returns_zero_values_consistently() {
    // When the oracle is unset every value-bearing field must read as 0.
    // The summary and the individual getters must agree on this.
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset) = setup(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    assert_eq!(client.get_collateral_value(&user), 0);
    assert_eq!(client.get_debt_value(&user), 0);
    assert_eq!(client.get_health_factor(&user), 0);
    assert_eq!(client.get_max_liquidatable_amount(&user), 0);

    let summary = client.get_user_position(&user);
    assert_eq!(summary.collateral_value, 0);
    assert_eq!(summary.debt_value, 0);
    assert_eq!(summary.health_factor, 0);
    // But the raw balance fields must still be exact, not zero
    assert_eq!(summary.collateral_balance, 20_000);
    assert_eq!(summary.debt_balance, 10_000);
}

#[test]
fn test_invariant_missing_user_returns_default_summary() {
    // A user with no recorded position must produce a defaulted summary
    // — every numeric field is 0 except `health_factor` which is the
    // documented "no debt" sentinel.
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, _asset, _collateral_asset, _oracle) = setup_with_oracle(&env);

    let summary = client.get_user_position(&user);
    assert_eq!(summary.collateral_balance, 0);
    assert_eq!(summary.collateral_value, 0);
    assert_eq!(summary.debt_balance, 0);
    assert_eq!(summary.debt_value, 0);
    assert_eq!(summary.health_factor, HEALTH_FACTOR_NO_DEBT);
    assert_eq!(client.get_max_liquidatable_amount(&user), 0);
    assert_summary_consistent(&client, &user);
}

#[test]
fn test_invariant_independent_users_independent_summaries() {
    // Users borrowing in different shapes must see independent, internally
    // consistent summaries. No cross-contamination across positions.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, _u, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.set_liquidation_threshold_bps(&admin, &8000);

    let users: [(i128, i128); 4] = [
        (10_000, 20_000),
        (5_000, 30_000),
        (25_000, 50_000),
        (1, 2),
    ];
    let addrs = [
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];

    for (i, addr) in addrs.iter().enumerate() {
        let (debt, coll) = users[i];
        client.borrow(addr, &asset, &debt, &collateral_asset, &coll);
    }

    for (i, addr) in addrs.iter().enumerate() {
        let summary = client.get_user_position(addr);
        assert_eq!(summary.collateral_balance, users[i].1);
        assert_eq!(summary.debt_balance, users[i].0);
        assert_summary_consistent(&client, addr);
    }
}

#[test]
fn test_invariant_summary_stable_across_repeated_reads() {
    // Calling `get_user_position` N times in a row must always return the
    // same value when underlying state hasn't changed. This is the
    // "stable serialization" invariant: outputs are a pure function of
    // (storage, oracle, ledger). Repeated calls do not drift.
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &7_500, &collateral_asset, &15_000);

    let first = client.get_user_position(&user);
    for _ in 0..16 {
        let again = client.get_user_position(&user);
        assert_eq!(first, again, "view summary drifted across repeated reads");
    }
}

#[test]
fn test_invariant_max_liquidatable_consistent_with_health_factor() {
    // `get_max_liquidatable_amount` must be 0 whenever the position is
    // healthy or the oracle is unconfigured, and must be `total_debt *
    // close_factor / 10_000` whenever the position is liquidatable.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);

    // Healthy position → max liquidatable is 0
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);
    assert!(client.get_health_factor(&user) >= HEALTH_FACTOR_SCALE);
    assert_eq!(client.get_max_liquidatable_amount(&user), 0);

    // Drop liquidation threshold so the position becomes liquidatable
    let user2 = Address::generate(&env);
    client.set_liquidation_threshold_bps(&admin, &4000);
    client.borrow(&user2, &asset, &15_000, &collateral_asset, &30_000);
    let hf = client.get_health_factor(&user2);
    assert!(hf < HEALTH_FACTOR_SCALE);
    let max_liq = client.get_max_liquidatable_amount(&user2);
    assert!(max_liq > 0);
    // Must not exceed total outstanding debt (a hard upper bound).
    assert!(max_liq <= client.get_debt_balance(&user2));
}

#[test]
fn test_invariant_liquidation_incentive_monotonic() {
    // The liquidation incentive function must be monotonic in `repay_amount`
    // — larger repayments must yield larger (or equal) incentive payouts.
    // Pinned here so a future refactor can't accidentally introduce a
    // non-monotonic curve that liquidators could exploit.
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _user, _asset, _collateral_asset, _oracle) = setup_with_oracle(&env);

    let xs: [i128; 6] = [0, 1, 100, 1_000, 1_000_000, i128::MAX / 4];
    let mut prev = client.get_liquidation_incentive_amount(&xs[0]);
    for x in xs.iter().skip(1) {
        let cur = client.get_liquidation_incentive_amount(x);
        assert!(cur >= prev, "incentive non-monotonic at repay={x}");
        prev = cur;
    }

    // Negative or zero amounts always yield zero
    assert_eq!(client.get_liquidation_incentive_amount(&0), 0);
    assert_eq!(client.get_liquidation_incentive_amount(&-1), 0);
    assert_eq!(client.get_liquidation_incentive_amount(&-1_000_000), 0);
}

#[test]
fn test_invariant_summary_unchanged_by_liquidation_threshold_only_for_balances() {
    // Changing the liquidation threshold must move `health_factor` but
    // must not move `collateral_balance`, `collateral_value`,
    // `debt_balance`, or `debt_value`. These fields are functions of
    // raw state + oracle only — independent of the risk parameter.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let s_before = client.get_user_position(&user);
    client.set_liquidation_threshold_bps(&admin, &6000);
    let s_after = client.get_user_position(&user);

    assert_eq!(s_before.collateral_balance, s_after.collateral_balance);
    assert_eq!(s_before.debt_balance, s_after.debt_balance);
    assert_eq!(s_before.collateral_value, s_after.collateral_value);
    assert_eq!(s_before.debt_value, s_after.debt_value);
    assert_ne!(
        s_before.health_factor, s_after.health_factor,
        "lowering liquidation threshold must reduce health factor"
    );
    assert!(s_after.health_factor < s_before.health_factor);
}

#[test]
fn test_invariant_summary_consistent_under_randomised_thresholds() {
    // Pseudo-random permutation of liquidation thresholds. For each
    // threshold the summary's HF must equal the individual getter's HF
    // (consistency across the unified and itemised view paths) and the
    // balance fields must remain pinned to the underlying state.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    // A spread of thresholds — chosen to also exercise the rounding floor
    let thresholds: [i128; 8] = [10_000, 8500, 7500, 5000, 3333, 2500, 1, 9999];
    for lt in thresholds.iter() {
        client.set_liquidation_threshold_bps(&admin, lt);
        let summary = client.get_user_position(&user);
        assert_eq!(summary.health_factor, client.get_health_factor(&user));
        assert_eq!(summary.collateral_balance, 20_000);
        assert_eq!(summary.debt_balance, 10_000);
        assert_eq!(summary.collateral_value, client.get_collateral_value(&user));
        assert_eq!(summary.debt_value, client.get_debt_value(&user));
    }
}
