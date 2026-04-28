//! # Interest Rate Model Test Suite
//!
//! Validates the kink-based interest rate model, ensuring:
//! - Correct utilization calculation
//! - Monotonicity of rates vs utilization
//! - Continuous behavior at the kink
//! - Enforcement of floor and ceiling bounds

extern crate std;
use soroban_sdk::{testutils::Address as _, Address, Env};
use crate::interest_rate::{self, InterestRateConfig};
use crate::deposit::DepositDataKey;
use crate::borrow::BorrowDataKey;

#[soroban_sdk::contract]
pub struct MockRateContract;

#[soroban_sdk::contractimpl]
impl MockRateContract {
    pub fn init(env: Env) {
        interest_rate::initialize(&env).unwrap();
    }
    pub fn get_utilization(env: Env) -> i128 {
        interest_rate::calculate_utilization(&env).unwrap()
    }
    pub fn get_borrow_rate(env: Env) -> i128 {
        interest_rate::calculate_borrow_rate(&env).unwrap()
    }
    pub fn get_supply_rate(env: Env) -> i128 {
        interest_rate::calculate_supply_rate(&env).unwrap()
    }
    pub fn get_config(env: Env) -> InterestRateConfig {
        interest_rate::get_config(&env)
    }
    pub fn update_config(env: Env, config: InterestRateConfig) {
        interest_rate::update_config(&env, config);
    }
    pub fn set_market(env: Env, deposits: i128, borrows: i128) {
        env.storage().persistent().set(&DepositDataKey::TotalAmount, &deposits);
        env.storage().persistent().set(&BorrowDataKey::BorrowTotalDebt, &borrows);
    }
}

#[test]
fn test_utilization_calculation() {
    let env = Env::default();
    let contract_id = env.register_contract(None, MockRateContract);
    let client = MockRateContractClient::new(&env, &contract_id);
    
    // 0% utilization
    client.set_market(&1000, &0);
    assert_eq!(client.get_utilization(), 0);
    
    // 50% utilization
    client.set_market(&1000, &500);
    assert_eq!(client.get_utilization(), 5000);
    
    // 80% utilization (Kink)
    client.set_market(&1000, &800);
    assert_eq!(client.get_utilization(), 8000);
    
    // 100% utilization
    client.set_market(&1000, &1000);
    assert_eq!(client.get_utilization(), 10000);
    
    // > 100% utilization (capped)
    client.set_market(&1000, &1500);
    assert_eq!(client.get_utilization(), 10000);
    
    // No deposits
    client.set_market(&0, &1000);
    assert_eq!(client.get_utilization(), 0);
}

#[test]
fn test_borrow_rate_kink_behavior() {
    let env = Env::default();
    let contract_id = env.register_contract(None, MockRateContract);
    let client = MockRateContractClient::new(&env, &contract_id);
    client.init();
    let config = client.get_config();
    
    // 0% Utilization -> Base Rate (1%)
    client.set_market(&1000, &0);
    assert_eq!(client.get_borrow_rate(), config.base_rate_bps);
    
    // 40% Utilization (Halfway to kink) -> 1% + (0.4/0.8)*20% = 1% + 10% = 11% (1100 bps)
    client.set_market(&1000, &400);
    assert_eq!(client.get_borrow_rate(), 1100);
    
    // 80% Utilization (Exact Kink) -> 1% + 20% = 21% (2100 bps)
    client.set_market(&1000, &800);
    assert_eq!(client.get_borrow_rate(), 2100);
    
    // 90% Utilization (Halfway above kink) -> 21% + (0.1/0.2)*100% = 21% + 50% = 71% (7100 bps)
    client.set_market(&1000, &900);
    assert_eq!(client.get_borrow_rate(), 7100);
    
    // 100% Utilization -> 21% + 100% = 121% (12100 bps) -> capped at 100% (10000 bps)
    client.set_market(&1000, &1000);
    assert_eq!(client.get_borrow_rate(), 10000);
}

#[test]
fn test_rate_monotonicity() {
    let env = Env::default();
    let contract_id = env.register_contract(None, MockRateContract);
    let client = MockRateContractClient::new(&env, &contract_id);
    client.init();
    
    let mut last_rate = -1;
    for util in (0..=10000).step_by(100) {
        client.set_market(&10000, &util);
        let current_rate = client.get_borrow_rate();
        assert!(current_rate >= last_rate, "Monotonicity violated at util {}", util);
        last_rate = current_rate;
    }
}

#[test]
fn test_rate_bounds() {
    let env = Env::default();
    let contract_id = env.register_contract(None, MockRateContract);
    let client = MockRateContractClient::new(&env, &contract_id);
    client.init();
    
    let mut config = client.get_config();
    config.rate_floor_bps = 500;   // 5% min
    config.rate_ceiling_bps = 5000; // 50% max
    client.update_config(&config);
    
    // Test floor
    client.set_market(&1000, &0); // Natural rate 1%
    assert_eq!(client.get_borrow_rate(), 500);
    
    // Test ceiling
    client.set_market(&1000, &1000); // Natural rate 121%
    assert_eq!(client.get_borrow_rate(), 5000);
}

#[test]
fn test_emergency_adjustment() {
    let env = Env::default();
    let contract_id = env.register_contract(None, MockRateContract);
    let client = MockRateContractClient::new(&env, &contract_id);
    client.init();
    
    let mut config = client.get_config();
    config.emergency_adjustment_bps = 500; // +5%
    client.update_config(&config);
    
    // 0% Util -> 1% base + 5% emergency = 6%
    client.set_market(&1000, &0);
    assert_eq!(client.get_borrow_rate(), 600);
    
    // Negative adjustment
    let mut config2 = client.get_config();
    config2.emergency_adjustment_bps = -200; // -2%
    client.update_config(&config2);
    
    // 0% Util -> 1% base - 2% emergency = -1% -> capped at floor (0.5%)
    client.set_market(&1000, &0);
    assert_eq!(client.get_borrow_rate(), 50);
}

#[test]
fn test_supply_rate() {
    let env = Env::default();
    let contract_id = env.register_contract(None, MockRateContract);
    let client = MockRateContractClient::new(&env, &contract_id);
    client.init();
    
    // 80% Utilization -> Borrow 21%, Supply = 21% - 2% = 19%
    client.set_market(&1000, &800);
    assert_eq!(client.get_supply_rate(), 1900);
    
    // 0% Utilization -> Borrow 1%, Supply = 1% - 2% = -1% -> capped at floor (0.5%)
    client.set_market(&1000, &0);
    assert_eq!(client.get_supply_rate(), 50);
}
