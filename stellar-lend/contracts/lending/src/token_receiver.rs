//! # Token Receiver Implementation
//!
//! Provides a token-aware entrypoint for collateral deposits and debt
//! repayments.
//!
//! ## Security model
//! - The caller must be the user whose balance is being debited.
//! - The user must have approved the lending contract as a token spender.
//! - The contract validates pause state *before* pulling funds.
//! - Funds are transferred with `transfer_from`, then the internal lending
//!   state is updated.
//!
//! This matches the Soroban token interface exposed in this repository, which
//! supports `approve` and `transfer_from` but not a standard authenticated
//! `transfer_call`/receiver-hook flow.

use crate::{
    asset_registry,
    borrow::{deposit, repay, BorrowError},
    pause::{self, blocks_high_risk_ops, PauseType},
};
use soroban_sdk::{token, Address, Env, FromVal, Symbol, Val, Vec};

/// Current payload version for token receiver operations
const PAYLOAD_VERSION: u32 = 1;

/// Maximum allowed payload length to prevent DoS attacks
const MAX_PAYLOAD_LENGTH: u32 = 10;

/// Token-aware receive entrypoint for Soroban tokens.
///
/// The entrypoint expects the caller to be the token owner (`from`). The owner
/// must authorize the call and pre-approve the lending contract to spend at
/// least `amount` of `token_asset`. The contract then pulls the tokens via the
/// Soroban token `transfer_from` interface and routes the amount to either the
/// collateral deposit path or the debt repayment path.
///
/// ## Security Enhancements
/// - Validates token contract is registered in the asset registry
/// - Validates payload version and structure to prevent malformed inputs
/// - Enforces payload length limits to prevent DoS attacks
/// - Strict action validation with only supported operations
///
/// # Arguments
/// * `env` - The contract environment
/// * `token_asset` - The token contract to pull funds from
/// * `from` - The owner whose balance will be debited
/// * `amount` - The amount of tokens to pull
/// * `payload` - A vector containing [version: u32, action: Symbol] with optional data
pub fn receive(
    env: Env,
    token_asset: Address,
    from: Address,
    amount: i128,
    payload: Vec<Val>,
) -> Result<(), BorrowError> {
    // Validate token contract is registered
    asset_registry::require_registered_asset(&env, &token_asset)?;

    // Validate amount
    if amount <= 0 {
        return Err(BorrowError::InvalidAmount);
    }

    // Validate payload structure and version
    let action = validate_and_parse_payload(&env, &payload)?;

    // Validate action and pause state
    validate_action_and_pause_state(&env, &action)?;

    // Validate sender authorization
    validate_sender(&env, &from, &token_asset)?;

    // Pull tokens securely
    pull_tokens(&env, &token_asset, &from, amount)?;

    // Execute the requested action
    execute_action(&env, from, token_asset, amount, &action)
}

/// Validates payload structure, version, and extracts the action
fn validate_and_parse_payload(env: &Env, payload: &Vec<Val>) -> Result<Symbol, BorrowError> {
    // Check payload length limits
    if payload.len() < 2 {
        return Err(BorrowError::MalformedPayload);
    }
    
    if payload.len() > MAX_PAYLOAD_LENGTH {
        return Err(BorrowError::MalformedPayload);
    }

    // Extract and validate version
    let version = u32::from_val(env, &payload.get(0).ok_or(BorrowError::MalformedPayload)?);
    if version != PAYLOAD_VERSION {
        return Err(BorrowError::InvalidPayloadVersion);
    }

    // Extract and validate action
    let action = Symbol::from_val(env, &payload.get(1).ok_or(BorrowError::MalformedPayload)?);
    
    // Validate action is a known symbol
    let valid_actions = vec![&env, Symbol::new(env, "deposit"), Symbol::new(env, "repay")];
    if !valid_actions.contains(&action) {
        return Err(BorrowError::AssetNotSupported);
    }

    Ok(action)
}

/// Validates action against pause state and other protocol constraints
fn validate_action_and_pause_state(env: &Env, action: &Symbol) -> Result<(), BorrowError> {
    if *action == Symbol::new(env, "deposit") {
        if pause::is_paused(env, PauseType::Deposit) || blocks_high_risk_ops(env) {
            return Err(BorrowError::ProtocolPaused);
        }
    } else if *action == Symbol::new(env, "repay") {
        if pause::is_paused(env, PauseType::Repay)
            || (!pause::is_recovery(env) && blocks_high_risk_ops(env))
        {
            return Err(BorrowError::ProtocolPaused);
        }
    } else {
        return Err(BorrowError::AssetNotSupported);
    }
    Ok(())
}

/// Validates sender authorization and token contract integrity
fn validate_sender(env: &Env, from: &Address, token_asset: &Address) -> Result<(), BorrowError> {
    // Require explicit authorization from the sender
    from.require_auth();
    
    // Additional validation: ensure the sender is not the token contract itself
    // This prevents token contracts from directly calling receive()
    if from == token_asset {
        return Err(BorrowError::UnauthorizedSender);
    }
    
    // Additional validation: ensure the sender is not the lending contract itself
    // This prevents self-call attacks
    let contract_address = env.current_contract_address();
    if from == &contract_address {
        return Err(BorrowError::UnauthorizedSender);
    }
    
    Ok(())
}

/// Executes the validated action
fn execute_action(
    env: &Env,
    from: Address,
    token_asset: Address,
    amount: i128,
    action: &Symbol,
) -> Result<(), BorrowError> {
    if *action == Symbol::new(env, "deposit") {
        deposit(env, from, token_asset, amount)
    } else {
        repay(env, from, token_asset, amount)
    }
}

fn pull_tokens(
    env: &Env,
    token_asset: &Address,
    from: &Address,
    amount: i128,
) -> Result<(), BorrowError> {
    let spender = env.current_contract_address();
    let token_client = token::Client::new(env, token_asset);

    if token_client.allowance(from, &spender) < amount {
        return Err(BorrowError::Unauthorized);
    }

    if token_client.balance(from) < amount {
        return Err(BorrowError::InvalidAmount);
    }

    token_client.transfer_from(&spender, from, &spender, &amount);
    Ok(())
}
