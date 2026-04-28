//! Cross-Asset Position Summary Invariant Tests
//!
//! These tests verify that `get_cross_position_summary` and related view
//! methods satisfy the guarantees documented in docs/CROSS_ASSET_RULES.md
//! (G-1 through G-10).
//!
//! Coverage:
//! - I-1..I-15: Per-invariant tests (empty, sentinel HF, formula, totals, rounding, isolation)
//! - Table-driven: 5 scenarios with varying asset counts / debt levels
//! - Security: view immutability, cross-user isolation

use crate::cross_asset::{AssetParams, CrossAssetError};
use crate::{LendingContract, LendingContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

const BPS_SCALE: i128 = 10_000;
const HF_NO_DEBT: i128 = 1_000_000;
const MOCK_PRICE: i128 = 10_000_000; // $1.00 with 7 decimals
const PRICE_DIVISOR: i128 = 10_000_000;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (LendingContractClient<'_>, Address) {
    let contract_id = env.register(LendingContract, ());
    let client = LendingContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin, &1_000_000_000, &1_000);
    client.initialize_admin(&admin);
    (client, admin)
}

fn make_params(env: &Env, ltv: i128) -> AssetParams {
    AssetParams {
        ltv,
        liquidation_threshold: (ltv + 500).min(BPS_SCALE),
        price_feed: Address::generate(env),
        debt_ceiling: 1_000_000_000_000,
        is_active: true,
    }
}

fn register_asset(env: &Env, client: &LendingContractClient<'_>, ltv: i128) -> Address {
    let asset = Address::generate(env);
    client.set_asset_params(&asset, &make_params(env, ltv));
    asset
}

fn usd_value(amount: i128) -> i128 {
    amount * MOCK_PRICE / PRICE_DIVISOR
}

fn weighted(amount: i128, ltv: i128) -> i128 {
    usd_value(amount) * ltv / BPS_SCALE
}

fn expected_hf(total_weighted: i128, total_debt: i128) -> i128 {
    if total_debt == 0 {
        HF_NO_DEBT
    } else {
        total_weighted * BPS_SCALE / total_debt
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I-1: Empty position
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_empty_position_zero_totals_max_health() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let user = Address::generate(&env);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_collateral_usd, 0);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-2: No-debt sentinel
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_no_debt_yields_sentinel_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &50_000);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.health_factor, HF_NO_DEBT);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.total_collateral_usd, usd_value(50_000));
}

// ─────────────────────────────────────────────────────────────────────────────
// I-3: Single-asset collateral value (G-3)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_single_collateral_value_matches_amount_at_unit_price() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 8_000);
    let user = Address::generate(&env);
    let amount = 123_456i128;
    client.deposit_collateral_asset(&user, &asset, &amount);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_collateral_usd, usd_value(amount));
}

// ─────────────────────────────────────────────────────────────────────────────
// I-4: Single-asset debt value (G-4)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_single_debt_value_matches_borrow_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 9_000);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &20_000);
    client.borrow_asset(&user, &asset, &10_000);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, usd_value(10_000));
}

// ─────────────────────────────────────────────────────────────────────────────
// I-5: Health factor formula (G-5)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_health_factor_formula_single_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let ltv = 7_500i128;
    let asset = register_asset(&env, &client, ltv);
    let user = Address::generate(&env);
    let coll = 10_000i128;
    let debt = 5_000i128;
    client.deposit_collateral_asset(&user, &asset, &coll);
    client.borrow_asset(&user, &asset, &debt);
    let s = client.get_cross_position_summary(&user);
    let w = weighted(coll, ltv);
    let d = usd_value(debt);
    assert_eq!(s.health_factor, expected_hf(w, d));
}

// ─────────────────────────────────────────────────────────────────────────────
// I-6: Multi-asset health factor (G-5 with multiple assets)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_health_factor_formula_multi_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let ltv_a = 8_000i128;
    let ltv_b = 6_000i128;
    let asset_a = register_asset(&env, &client, ltv_a);
    let asset_b = register_asset(&env, &client, ltv_b);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset_a, &20_000);
    client.deposit_collateral_asset(&user, &asset_b, &10_000);
    client.borrow_asset(&user, &asset_a, &5_000);
    client.borrow_asset(&user, &asset_b, &3_000);
    let s = client.get_cross_position_summary(&user);
    let total_w = weighted(20_000, ltv_a) + weighted(10_000, ltv_b);
    let total_d = usd_value(5_000) + usd_value(3_000);
    assert_eq!(s.total_collateral_usd, usd_value(20_000) + usd_value(10_000));
    assert_eq!(s.total_debt_usd, total_d);
    assert_eq!(s.health_factor, expected_hf(total_w, total_d));
}

// ─────────────────────────────────────────────────────────────────────────────
// I-7: Monotonicity — more collateral → HF never decreases (G-6)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_more_collateral_increases_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    let hf_before = client.get_cross_position_summary(&user).health_factor;
    client.deposit_collateral_asset(&user, &asset, &5_000);
    let hf_after = client.get_cross_position_summary(&user).health_factor;
    assert!(hf_after > hf_before);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-8: Monotonicity — more debt → HF never increases (G-6)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_more_debt_decreases_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 9_000);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &30_000);
    client.borrow_asset(&user, &asset, &5_000);
    let hf_before = client.get_cross_position_summary(&user).health_factor;
    client.borrow_asset(&user, &asset, &3_000);
    let hf_after = client.get_cross_position_summary(&user).health_factor;
    assert!(hf_after < hf_before);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-9: HF ≥ BPS_SCALE immediately after a borrow (G-5 + borrow guard)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_health_factor_always_ge_healthy_after_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &7_000);
    let s = client.get_cross_position_summary(&user);
    // BPS_SCALE = 10_000
    assert!(s.health_factor >= 10_000);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-10: Idempotency — reading the summary twice returns identical values (G-2)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_summary_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &4_000);
    let s1 = client.get_cross_position_summary(&user);
    let s2 = client.get_cross_position_summary(&user);
    assert_eq!(s1.total_collateral_usd, s2.total_collateral_usd);
    assert_eq!(s1.total_debt_usd, s2.total_debt_usd);
    assert_eq!(s1.health_factor, s2.health_factor);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-11: Deposit order invariance (G-8)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_deposit_order_does_not_affect_total_collateral() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset_a = register_asset(&env, &client, 7_500);
    let asset_b = register_asset(&env, &client, 7_500);

    let user1 = Address::generate(&env);
    client.deposit_collateral_asset(&user1, &asset_a, &5_000);
    client.deposit_collateral_asset(&user1, &asset_b, &3_000);

    let user2 = Address::generate(&env);
    client.deposit_collateral_asset(&user2, &asset_b, &3_000);
    client.deposit_collateral_asset(&user2, &asset_a, &5_000);

    let s1 = client.get_cross_position_summary(&user1);
    let s2 = client.get_cross_position_summary(&user2);
    assert_eq!(s1.total_collateral_usd, s2.total_collateral_usd);
    assert_eq!(s1.health_factor, s2.health_factor);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-12: User isolation (G-7)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_users_have_isolated_positions() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);
    client.deposit_collateral_asset(&user_a, &asset, &10_000);
    client.borrow_asset(&user_a, &asset, &5_000);
    // user_b has an empty position
    let s_b = client.get_cross_position_summary(&user_b);
    assert_eq!(s_b.total_collateral_usd, 0);
    assert_eq!(s_b.total_debt_usd, 0);
    assert_eq!(s_b.health_factor, HF_NO_DEBT);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-13: user_b operations do not affect user_a (G-7)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_user_b_activity_does_not_affect_user_a_summary() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);
    client.deposit_collateral_asset(&user_a, &asset, &10_000);
    client.borrow_asset(&user_a, &asset, &5_000);
    let before = client.get_cross_position_summary(&user_a);
    // user_b does various operations
    client.deposit_collateral_asset(&user_b, &asset, &30_000);
    client.borrow_asset(&user_b, &asset, &10_000);
    client.repay_asset(&user_b, &asset, &10_000);
    let after = client.get_cross_position_summary(&user_a);
    assert_eq!(before.total_collateral_usd, after.total_collateral_usd);
    assert_eq!(before.total_debt_usd, after.total_debt_usd);
    assert_eq!(before.health_factor, after.health_factor);
}

// ─────────────────────────────────────────────────────────────────────────────
// I-14: Multi-collateral, multi-debt totals sum independently
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_multi_collateral_total_is_sum_of_individual_values() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let amounts = [1_000i128, 5_000, 10_000];
    let mut total_expected = 0i128;
    let user = Address::generate(&env);
    for amt in &amounts {
        let asset = register_asset(&env, &client, 7_500);
        client.deposit_collateral_asset(&user, &asset, amt);
        total_expected += usd_value(*amt);
    }
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_collateral_usd, total_expected);
}

#[test]
fn invariant_multi_debt_total_is_sum_of_individual_debt_values() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let user = Address::generate(&env);
    let debt_amounts = [1_000i128, 2_000, 3_000];
    let mut total_debt_expected = 0i128;
    for debt in &debt_amounts {
        let asset = register_asset(&env, &client, 9_000);
        client.deposit_collateral_asset(&user, &asset, &(debt * 2));
        client.borrow_asset(&user, &asset, debt);
        total_debt_expected += usd_value(*debt);
    }
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, total_debt_expected);
}

#[test]
fn invariant_multi_collateral_multi_debt_sum_independently() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let user = Address::generate(&env);
    let ltv = 8_000i128;
    let asset_a = register_asset(&env, &client, ltv);
    let asset_b = register_asset(&env, &client, ltv);
    client.deposit_collateral_asset(&user, &asset_a, &10_000);
    client.deposit_collateral_asset(&user, &asset_b, &5_000);
    client.borrow_asset(&user, &asset_a, &4_000);
    client.borrow_asset(&user, &asset_b, &2_000);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_collateral_usd, usd_value(15_000));
    assert_eq!(s.total_debt_usd, usd_value(6_000));
    let tw = weighted(10_000, ltv) + weighted(5_000, ltv);
    assert_eq!(s.health_factor, expected_hf(tw, usd_value(6_000)));
}

// ─────────────────────────────────────────────────────────────────────────────
// I-15: LTV rounding truncates toward zero (G-9)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_ltv_rounding_truncates_toward_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let ltv = 7_500i128;
    let asset = register_asset(&env, &client, ltv);
    let user = Address::generate(&env);
    let coll = 3i128; // small number to expose floor rounding: 3 * 7500 / 10000 = 2.25 → 2
    client.deposit_collateral_asset(&user, &asset, &coll);
    client.borrow_asset(&user, &asset, &1);
    let s = client.get_cross_position_summary(&user);
    // weighted = 3 * 7500 / 10000 = 2 (floor); HF = 2 * 10000 / 1 = 20000
    assert_eq!(s.health_factor, weighted(coll, ltv) * BPS_SCALE / usd_value(1));
}

// ─────────────────────────────────────────────────────────────────────────────
// I-16: Full repay restores HF_NO_DEBT
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_full_repay_restores_max_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    client.repay_asset(&user, &asset, &5_000);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

#[test]
fn invariant_overpay_also_restores_max_health_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    client.repay_asset(&user, &asset, &999_999_999);
    let s = client.get_cross_position_summary(&user);
    assert_eq!(s.total_debt_usd, 0);
    assert_eq!(s.health_factor, HF_NO_DEBT);
}

// ─────────────────────────────────────────────────────────────────────────────
// Security: view-only calls must not mutate state (G-1, G-10)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn security_view_does_not_mutate_balances() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let asset = register_asset(&env, &client, 7_500);
    let user = Address::generate(&env);
    client.deposit_collateral_asset(&user, &asset, &10_000);
    client.borrow_asset(&user, &asset, &5_000);
    let s1 = client.get_cross_position_summary(&user);
    // Call the view 50 times
    for _ in 0..50 {
        let _ = client.get_cross_position_summary(&user);
    }
    let s2 = client.get_cross_position_summary(&user);
    assert_eq!(s1.total_collateral_usd, s2.total_collateral_usd);
    assert_eq!(s1.total_debt_usd, s2.total_debt_usd);
    assert_eq!(s1.health_factor, s2.health_factor);
}

#[test]
fn security_view_on_unknown_user_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    let stranger = Address::generate(&env);
    let s1 = client.get_cross_position_summary(&stranger);
    let s2 = client.get_cross_position_summary(&stranger);
    assert_eq!(s1.total_collateral_usd, s2.total_collateral_usd);
    assert_eq!(s1.total_debt_usd, s2.total_debt_usd);
    assert_eq!(s1.health_factor, s2.health_factor);
}

// ─────────────────────────────────────────────────────────────────────────────
// Table-driven scenarios
// ─────────────────────────────────────────────────────────────────────────────

struct Scenario {
    collaterals: &'static [(i128, i128)], // (amount, ltv)
    debts: &'static [i128],               // debt amount per collateral asset (same asset)
}

fn run_scenario(env: &Env, client: &LendingContractClient<'_>, s: &Scenario) {
    let user = Address::generate(env);
    let mut asset_slots: [Option<Address>; 5] = [None, None, None, None, None];
    let n = s.collaterals.len().min(5);
    for i in 0..n {
        let (amt, ltv) = s.collaterals[i];
        let asset = register_asset(env, client, ltv);
        client.deposit_collateral_asset(&user, &asset, &amt);
        if i < s.debts.len() && s.debts[i] > 0 {
            client.borrow_asset(&user, &asset, &s.debts[i]);
        }
        asset_slots[i] = Some(asset);
    }
    let summary = client.get_cross_position_summary(&user);
    let mut expected_collateral = 0i128;
    let mut expected_debt = 0i128;
    let mut expected_weighted = 0i128;
    for i in 0..n {
        let (amt, ltv) = s.collaterals[i];
        expected_collateral += usd_value(amt);
        expected_weighted += weighted(amt, ltv);
        if i < s.debts.len() {
            expected_debt += usd_value(s.debts[i]);
        }
    }
    assert_eq!(summary.total_collateral_usd, expected_collateral);
    assert_eq!(summary.total_debt_usd, expected_debt);
    assert_eq!(summary.health_factor, expected_hf(expected_weighted, expected_debt));
}

#[test]
fn table_scenario_single_collateral_no_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    run_scenario(&env, &client, &Scenario {
        collaterals: &[(10_000, 7_500)],
        debts: &[0],
    });
}

#[test]
fn table_scenario_single_asset_healthy_borrow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    run_scenario(&env, &client, &Scenario {
        collaterals: &[(10_000, 8_000)],
        debts: &[4_000],
    });
}

#[test]
fn table_scenario_single_asset_at_exact_capacity() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    // LTV 7500 → max borrow = 7500, HF exactly = 10_000
    run_scenario(&env, &client, &Scenario {
        collaterals: &[(10_000, 7_500)],
        debts: &[7_500],
    });
}

#[test]
fn table_scenario_two_collateral_two_debt() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    run_scenario(&env, &client, &Scenario {
        collaterals: &[(20_000, 8_000), (10_000, 6_000)],
        debts: &[5_000, 2_000],
    });
}

#[test]
fn table_scenario_large_amounts_floor_rounding() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _) = setup(&env);
    run_scenario(&env, &client, &Scenario {
        collaterals: &[(1_000_000, 7_500)],
        debts: &[400_000],
    });
}
