//! # Interest Rate Module
//!
//! Implements a kink-based (piecewise linear) interest rate model for the lending protocol.
//!
//! ## Rate Model
//!
//! The borrow rate is determined by protocol utilization (`borrows / deposits`):
//!
//! - **Below kink** (default 80%):
//!   `rate = base_rate + (utilization / kink_utilization) × multiplier`
//! - **Above kink**:
//!   `rate = base_rate + multiplier + ((utilization − kink) / (1 − kink)) × jump_multiplier`
//!
//! The supply rate is derived as: `supply_rate = borrow_rate − spread`

use soroban_sdk::{contracterror, contracttype, Address, Env};
use crate::deposit::DepositDataKey;
use crate::borrow::BorrowDataKey;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum InterestRateError {
    Unauthorized = 1,
    InvalidParameter = 2,
    Overflow = 3,
    DivisionByZero = 4,
    AlreadyInitialized = 5,
}

#[contracttype]
#[derive(Clone)]
pub enum InterestRateDataKey {
    Config,
    Admin,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct InterestRateConfig {
    pub base_rate_bps: i128,
    pub kink_utilization_bps: i128,
    pub multiplier_bps: i128,
    pub jump_multiplier_bps: i128,
    pub rate_floor_bps: i128,
    pub rate_ceiling_bps: i128,
    pub spread_bps: i128,
    pub emergency_adjustment_bps: i128,
}

const BPS_SCALE: i128 = 10_000;

pub fn get_default_config() -> InterestRateConfig {
    InterestRateConfig {
        base_rate_bps: 100,            // 1%
        kink_utilization_bps: 8000,    // 80%
        multiplier_bps: 2000,          // 20%
        jump_multiplier_bps: 10000,    // 100%
        rate_floor_bps: 50,            // 0.5%
        rate_ceiling_bps: 10000,       // 100%
        spread_bps: 200,               // 2%
        emergency_adjustment_bps: 0,
    }
}

pub fn initialize(env: &Env) -> Result<(), InterestRateError> {
    if env.storage().persistent().has(&InterestRateDataKey::Config) {
        return Err(InterestRateError::AlreadyInitialized);
    }
    env.storage().persistent().set(&InterestRateDataKey::Config, &get_default_config());
    Ok(())
}

pub fn get_config(env: &Env) -> InterestRateConfig {
    env.storage()
        .persistent()
        .get(&InterestRateDataKey::Config)
        .unwrap_or_else(|| get_default_config())
}

pub fn calculate_utilization(env: &Env) -> Result<i128, InterestRateError> {
    // Note: We use persistent storage for global totals to ensure reliability across upgrades.
    let total_deposits: i128 = env.storage().persistent().get(&DepositDataKey::TotalAmount).unwrap_or(0);
    
    // Fallback to 0 if not found in persistent
    let total_borrows: i128 = env.storage().persistent().get(&BorrowDataKey::BorrowTotalDebt).unwrap_or(0);

    if total_deposits <= 0 {
        return Ok(0);
    }

    let utilization = total_borrows
        .checked_mul(BPS_SCALE)
        .ok_or(InterestRateError::Overflow)?
        .checked_div(total_deposits)
        .ok_or(InterestRateError::DivisionByZero)?;

    Ok(utilization.min(BPS_SCALE))
}

pub fn calculate_borrow_rate(env: &Env) -> Result<i128, InterestRateError> {
    let config = get_config(env);
    let utilization = calculate_utilization(env)?;

    let mut rate = config.base_rate_bps;

    if utilization <= config.kink_utilization_bps {
        if config.kink_utilization_bps > 0 {
            let increase = utilization
                .checked_mul(config.multiplier_bps)
                .ok_or(InterestRateError::Overflow)?
                .checked_div(config.kink_utilization_bps)
                .ok_or(InterestRateError::DivisionByZero)?;
            rate = rate.checked_add(increase).ok_or(InterestRateError::Overflow)?;
        }
    } else {
        let rate_at_kink = config.base_rate_bps + config.multiplier_bps;
        let excess_util = utilization - config.kink_utilization_bps;
        let excess_range = BPS_SCALE - config.kink_utilization_bps;

        if excess_range > 0 {
            let increase = excess_util
                .checked_mul(config.jump_multiplier_bps)
                .ok_or(InterestRateError::Overflow)?
                .checked_div(excess_range)
                .ok_or(InterestRateError::DivisionByZero)?;
            rate = rate_at_kink.checked_add(increase).ok_or(InterestRateError::Overflow)?;
        } else {
            rate = rate_at_kink;
        }
    }

    rate = rate.checked_add(config.emergency_adjustment_bps).ok_or(InterestRateError::Overflow)?;
    Ok(rate.clamp(config.rate_floor_bps, config.rate_ceiling_bps))
}

pub fn calculate_supply_rate(env: &Env) -> Result<i128, InterestRateError> {
    let config = get_config(env);
    let borrow_rate = calculate_borrow_rate(env)?;
    Ok((borrow_rate - config.spread_bps).max(config.rate_floor_bps))
}

pub fn update_config(env: &Env, config: InterestRateConfig) {
    env.storage().persistent().set(&InterestRateDataKey::Config, &config);
}
