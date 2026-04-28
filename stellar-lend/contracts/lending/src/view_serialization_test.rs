extern crate alloc;

use super::*;
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    xdr::{ScMap, ScMapEntry, ScSymbol, ScVal, ToXdr},
    Address, Env, TryFromVal, Val,
};

use crate::{
    deposit::DepositCollateral,
    views::{UserPositionSummary, HEALTH_FACTOR_NO_DEBT, VIEW_SCHEMA_VERSION},
};

#[contract]
pub struct MockOracle;

#[contractimpl]
impl MockOracle {
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

fn map_key(name: &str) -> ScVal {
    ScSymbol(name.try_into().unwrap()).into()
}

fn scval<T>(env: &Env, value: &T) -> ScVal
where
    Val: TryFromVal<Env, T>,
    <Val as TryFromVal<Env, T>>::Error: core::fmt::Debug,
{
    let val = Val::try_from_val(env, value).unwrap();
    ScVal::try_from_val(env, &val).unwrap()
}

fn expected_map(entries: alloc::vec::Vec<(&str, ScVal)>) -> ScVal {
    let entries = entries
        .into_iter()
        .map(|(name, val)| ScMapEntry {
            key: map_key(name),
            val,
        })
        .collect::<alloc::vec::Vec<_>>();

    ScVal::Map(Some(ScMap::sorted_from(entries).unwrap()))
}

fn assert_map_keys(actual: &ScVal, expected_keys: &[&str]) {
    let ScVal::Map(Some(map)) = actual else {
        panic!("expected a map-backed ScVal");
    };

    assert_eq!(map.len(), expected_keys.len());
    for (entry, key) in map.iter().zip(expected_keys.iter()) {
        assert_eq!(entry.key, map_key(key));
    }
}

fn assert_struct_snapshot<T>(env: &Env, value: &T, expected: ScVal, expected_keys: &[&str])
where
    for<'a> &'a T: TryInto<ScVal, Error = soroban_sdk::xdr::Error> + ToXdr,
{
    let actual_scval: ScVal = value.try_into().unwrap();
    let actual_xdr = value.to_xdr(env);
    let expected_xdr = Val::try_from_val(env, &expected).unwrap().to_xdr(env);

    assert_eq!(actual_scval, expected);
    assert_eq!(actual_xdr, expected_xdr);
    assert_map_keys(&actual_scval, expected_keys);
}

#[test]
fn test_view_schema_version_is_v1() {
    assert_eq!(VIEW_SCHEMA_VERSION, 1);
}

#[test]
fn test_get_user_debt_xdr_snapshot_is_stable() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.timestamp = 1_234);

    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let debt = client.get_user_debt(&user);
    let expected = expected_map(alloc::vec![
        ("asset", scval(&env, &asset)),
        ("borrowed_amount", scval(&env, &10_000_i128)),
        ("interest_accrued", scval(&env, &0_i128)),
        ("last_update", scval(&env, &1_234_u64)),
    ]);

    assert_struct_snapshot(
        &env,
        &debt,
        expected,
        &[
            "asset",
            "borrowed_amount",
            "interest_accrued",
            "last_update",
        ],
    );
}

#[test]
fn test_get_user_collateral_xdr_snapshot_is_stable() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let collateral = client.get_user_collateral(&user);
    let expected = expected_map(alloc::vec![
        ("amount", scval(&env, &20_000_i128)),
        ("asset", scval(&env, &collateral_asset)),
    ]);

    assert_struct_snapshot(&env, &collateral, expected, &["amount", "asset"]);
}

#[test]
fn test_get_user_collateral_deposit_xdr_snapshot_is_stable() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.timestamp = 9_999);

    let (client, _admin, user, asset, _collateral_asset) = setup(&env);
    client.deposit(&user, &asset, &15_000);

    let deposit_position: DepositCollateral = client.get_user_collateral_deposit(&user, &asset);
    let expected = expected_map(alloc::vec![
        ("amount", scval(&env, &15_000_i128)),
        ("asset", scval(&env, &asset)),
        ("last_deposit_time", scval(&env, &9_999_u64)),
    ]);

    assert_struct_snapshot(
        &env,
        &deposit_position,
        expected,
        &["amount", "asset", "last_deposit_time"],
    );
}

#[test]
fn test_get_user_position_xdr_snapshot_is_stable() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, user, asset, collateral_asset, _oracle) = setup_with_oracle(&env);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let position: UserPositionSummary = client.get_user_position(&user);
    let expected = expected_map(alloc::vec![
        ("collateral_balance", scval(&env, &20_000_i128)),
        ("collateral_value", scval(&env, &20_000_i128)),
        ("debt_balance", scval(&env, &10_000_i128)),
        ("debt_value", scval(&env, &10_000_i128)),
        ("health_factor", scval(&env, &16_000_i128)),
    ]);

    assert_struct_snapshot(
        &env,
        &position,
        expected,
        &[
            "collateral_balance",
            "collateral_value",
            "debt_balance",
            "debt_value",
            "health_factor",
        ],
    );
}

#[test]
fn test_empty_user_position_xdr_snapshot_is_stable() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, user, _asset, _collateral_asset, _oracle) = setup_with_oracle(&env);

    let position: UserPositionSummary = client.get_user_position(&user);
    let expected = expected_map(alloc::vec![
        ("collateral_balance", scval(&env, &0_i128)),
        ("collateral_value", scval(&env, &0_i128)),
        ("debt_balance", scval(&env, &0_i128)),
        ("debt_value", scval(&env, &0_i128)),
        ("health_factor", scval(&env, &HEALTH_FACTOR_NO_DEBT)),
    ]);

    assert_struct_snapshot(
        &env,
        &position,
        expected,
        &[
            "collateral_balance",
            "collateral_value",
            "debt_balance",
            "debt_value",
            "health_factor",
        ],
    );
}
