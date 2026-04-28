use crate::borrow::BorrowCollateral;
use crate::borrow::{calculate_interest, validate_collateral_ratio, BorrowDataKey, DebtPosition};
use crate::flash_loan::FlashLoanError;
use crate::views::{collateral_value, compute_health_factor, HEALTH_FACTOR_NO_DEBT};
use crate::LendingContract;
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, testutils::Ledger as _, token, Address, Bytes,
    Env,
};

#[contract]
struct UnitPriceOracle;

#[contractimpl]
impl UnitPriceOracle {
    pub fn price(_env: Env, _asset: Address) -> i128 {
        100_000_000
    }
}

#[contract]
struct MaxPriceOracle;

#[contractimpl]
impl MaxPriceOracle {
    pub fn price(_env: Env, _asset: Address) -> i128 {
        i128::MAX
    }
}

#[contract]
struct MathSafetyFlashLoanReceiver;

#[contractimpl]
impl MathSafetyFlashLoanReceiver {
    pub fn on_flash_loan(
        env: Env,
        initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        _params: Bytes,
    ) -> bool {
        let token_client = token::Client::new(&env, &asset);
        let repayment = amount.saturating_add(fee);
        token_client.transfer(&env.current_contract_address(), &initiator, &repayment);
        true
    }
}

#[test]
fn test_interest_monotonic_for_large_ledger_jumps() {
    let env = Env::default();
    let position = DebtPosition {
        borrowed_amount: 1_000_000,
        interest_accrued: 0,
        last_update: 0,
        asset: Address::generate(&env),
    };

    let checkpoints = [1u64, 10u64, 100u64, 500u64];
    let mut previous_interest = 0i128;

    for years in checkpoints {
        env.ledger()
            .with_mut(|li| li.timestamp = years * 31_536_000);
        let interest = calculate_interest(&env, &position);
        assert!(interest >= previous_interest);

        let upper_bound = position
            .borrowed_amount
            .checked_mul(5)
            .and_then(|v| v.checked_mul(years as i128))
            .and_then(|v| v.checked_div(100))
            .unwrap();
        assert!(interest <= upper_bound);

        previous_interest = interest;
    }
}

#[test]
fn test_interest_saturates_to_i128_max_at_extreme_horizon() {
    let env = Env::default();
    let position = DebtPosition {
        borrowed_amount: i128::MAX,
        interest_accrued: 0,
        last_update: 0,
        asset: Address::generate(&env),
    };

    env.ledger().with_mut(|li| li.timestamp = u64::MAX);
    let interest = calculate_interest(&env, &position);
    assert_eq!(interest, i128::MAX);
}

#[test]
fn test_get_user_debt_interest_addition_saturates() {
    let env = Env::default();
    let contract_id = env.register(LendingContract, ());
    let user = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let initial = DebtPosition {
            borrowed_amount: i128::MAX,
            interest_accrued: i128::MAX - 10,
            last_update: 0,
            asset: user.clone(),
        };
        env.storage()
            .persistent()
            .set(&BorrowDataKey::BorrowUserDebt(user.clone()), &initial);
    });

    env.ledger().with_mut(|li| li.timestamp = u64::MAX);
    let debt = env.as_contract(&contract_id, || crate::borrow::get_user_debt(&env, &user));
    assert_eq!(debt.interest_accrued, i128::MAX);
}

#[test]
fn test_borrow_amount_zero_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let coll_asset = Address::generate(&env);

    // Register assets in the registry so we reach the amount check
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(
            &crate::asset_registry::RegistryKey::AssetRegistry(asset.clone()),
            &true,
        );
        env.storage().persistent().set(
            &crate::asset_registry::RegistryKey::AssetRegistry(coll_asset.clone()),
            &true,
        );
    });

    let res = env.as_contract(&contract_id, || {
        crate::borrow::borrow(
            &env,
            user.clone(),
            asset.clone(),
            0,
            coll_asset.clone(),
            100,
        )
    });
    assert_eq!(res, Err(crate::borrow::BorrowError::InvalidAmount));

    let res2 = env.as_contract(&contract_id, || {
        crate::borrow::borrow(
            &env,
            user.clone(),
            asset.clone(),
            1000,
            coll_asset.clone(),
            0,
        )
    });
    assert_eq!(
        res2,
        Err(crate::borrow::BorrowError::InsufficientCollateral)
    );
}

#[test]
fn test_borrow_math_exhaustion() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(LendingContract, ());
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let coll_asset = Address::generate(&env);

    // Initial setup for protocol variables to allow tests
    env.as_contract(&contract_id, || {
        crate::borrow::initialize_borrow_settings(&env, i128::MAX, 1).unwrap();
        // Register assets in the registry
        env.storage().persistent().set(
            &crate::asset_registry::RegistryKey::AssetRegistry(asset.clone()),
            &true,
        );
        env.storage().persistent().set(
            &crate::asset_registry::RegistryKey::AssetRegistry(coll_asset.clone()),
            &true,
        );
    });

    // Overflow check on collateral ratio (borrow amount too large)
    let res = env.as_contract(&contract_id, || {
        crate::borrow::borrow(
            &env,
            user.clone(),
            asset.clone(),
            i128::MAX,
            coll_asset.clone(),
            100,
        )
    });
    // With i128::MAX borrow, collateral ratio check will overflow and fail early
    assert_eq!(res, Err(crate::borrow::BorrowError::Overflow));
}

#[test]
#[should_panic(expected = "Error(Auth, InvalidAction)")]
fn test_borrow_unauthorized_fails() {
    let env = Env::default();
    let contract_id = env.register(LendingContract, ());
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let coll_asset = Address::generate(&env);

    // Attempting borrow without mocking auth should fail
    env.as_contract(&contract_id, || {
        crate::borrow::borrow(
            &env,
            user.clone(),
            asset.clone(),
            1000,
            coll_asset.clone(),
            2000,
        )
        .unwrap();
    });
}

#[test]
fn test_constrained_interest_boundaries_are_bounded_and_deterministic() {
    let env = Env::default();
    let principal_cases = [1_i128, 1_000_000_i128, i128::MAX / 10];
    let time_cases = [1_u64, 3_600_u64, 31_536_000_u64, 315_360_000_u64];

    for principal in principal_cases {
        let mut previous = 0_i128;
        for ts in time_cases {
            let position = DebtPosition {
                borrowed_amount: principal,
                interest_accrued: 0,
                last_update: 0,
                asset: Address::generate(&env),
            };
            env.ledger().with_mut(|li| li.timestamp = ts);

            let first = calculate_interest(&env, &position);
            let second = calculate_interest(&env, &position);

            assert_eq!(first, second);
            assert!(first >= 0);
            assert!(first <= i128::MAX);
            assert!(first >= previous);
            previous = first;
        }
    }
}

#[test]
fn test_constrained_liquidation_math_close_factor_boundaries() {
    for close_factor in [1_i128, 2_500_i128, 5_000_i128, 10_000_i128] {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(LendingContract, ());
        let client = crate::LendingContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let liquidator = Address::generate(&env);
        let debt_asset = Address::generate(&env);
        let collateral_asset = Address::generate(&env);

        client.initialize(&admin, &10_000_000_000, &1);
        let oracle_id = env.register(UnitPriceOracle, ());
        client.set_oracle(&admin, &oracle_id);
        client.set_liquidation_threshold_bps(&admin, &4_000);
        client.set_close_factor_bps(&admin, &close_factor);

        client.borrow(
            &borrower,
            &debt_asset,
            &100_000,
            &collateral_asset,
            &150_000,
        );

        let max_liq = client.get_max_liquidatable_amount(&borrower);
        let expected = 100_000_i128
            .checked_mul(close_factor)
            .and_then(|v| v.checked_div(10_000))
            .unwrap();
        assert_eq!(max_liq, expected);

        client.liquidate(
            &liquidator,
            &borrower,
            &debt_asset,
            &collateral_asset,
            &(max_liq + 123),
        );

        let remaining_debt = client.get_debt_balance(&borrower);
        assert_eq!(remaining_debt, 100_000 - max_liq);
        assert!(client.get_collateral_balance(&borrower) >= 0);
    }
}

#[test]
fn test_constrained_collateral_and_debt_value_extreme_price_bounded() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = crate::LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &10_000_000_000, &1);
    let oracle_id = env.register(MaxPriceOracle, ());
    client.set_oracle(&admin, &oracle_id);
    client.borrow(&user, &asset, &10_000, &collateral_asset, &20_000);

    let collateral_first = client.get_collateral_value(&user);
    let collateral_second = client.get_collateral_value(&user);
    let debt_first = client.get_debt_value(&user);
    let debt_second = client.get_debt_value(&user);

    assert_eq!(collateral_first, collateral_second);
    assert_eq!(debt_first, debt_second);
    assert!(collateral_first >= 0);
    assert!(debt_first >= 0);
}

#[test]
fn test_constrained_flash_loan_fee_boundaries() {
    let cases = [
        (0_i128, 1_i128),
        (5_i128, 10_000_i128),
        (100_i128, 1_000_000_i128),
        (1_000_i128, 1_000_000_000_i128),
    ];

    for (fee_bps, amount) in cases {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(LendingContract, ());
        let client = crate::LendingContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let asset = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let token_admin = token::StellarAssetClient::new(&env, &asset);

        client.initialize(&admin, &10_000_000_000, &1);
        client.set_flash_loan_fee_bps(&fee_bps);

        let receiver_id = env.register(MathSafetyFlashLoanReceiver, ());

        let expected_fee = amount.saturating_mul(fee_bps).saturating_div(10_000);
        token_admin.mint(&contract_id, &(amount + 10_000));
        token_admin.mint(&receiver_id, &(expected_fee + 10_000));

        let token_client = token::Client::new(&env, &asset);
        let before = token_client.balance(&contract_id);
        client.flash_loan(&receiver_id, &asset, &amount, &Bytes::new(&env));
        let after = token_client.balance(&contract_id);

        assert_eq!(after - before, expected_fee);
    }
}

#[test]
fn test_constrained_deterministic_error_paths_for_overflow_inputs() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(LendingContract, ());
    let client = crate::LendingContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let asset = Address::generate(&env);
    let collateral_asset = Address::generate(&env);

    client.initialize(&admin, &i128::MAX, &1);

    for _ in 0..4 {
        let borrow_result =
            client.try_borrow(&user, &asset, &i128::MAX, &collateral_asset, &i128::MAX);
        assert_eq!(borrow_result, Err(Ok(crate::borrow::BorrowError::Overflow)));
    }

    for _ in 0..4 {
        let fee_result = client.try_set_flash_loan_fee_bps(&1_001);
        assert_eq!(fee_result, Err(Ok(FlashLoanError::InvalidFee)));
    }
}
