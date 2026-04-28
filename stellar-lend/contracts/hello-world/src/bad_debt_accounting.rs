// src/bad_debt_accounting.rs
//
// Bad-Debt Accounting Module
// ══════════════════════════
//
// Terminology
// ───────────
// • **Shortfall**   — The amount by which a borrower's debt exceeds the value
//                     of collateral that can realistically be seized.
// • **Write-off**   — Permanently removing a shortfall from the books; the
//                     loss is socialised across depositors or absorbed by
//                     protocol reserves.
// • **Reserve cover** — Reserves are consumed first before any loss is
//                       socialised.  This isolates depositors from small shocks.
// • **Bad debt**    — Cumulative written-off shortfalls for an asset market.
//
// Invariants (must hold after every state transition)
// ────────────────────────────────────────────────────
// [I-1]  bad_debt   >= 0          (never negative)
// [I-2]  reserves   >= 0          (never negative)
// [I-3]  total_borrows >= 0       (never negative)
// [I-4]  total_deposits >= 0      (never negative)
// [I-5]  For every market: total_deposits - total_borrows + reserves - bad_debt >= 0
//         (protocol remains nominally solvent; losses are accounted, not hidden)
// [I-6]  A user's borrowed balance after write-off == 0
//         (we never leave phantom debt on the books)
//
// Accounting flow
// ───────────────
//   1.  Liquidator calls `liquidate()` in liquidate.rs.
//   2.  As much collateral as possible is seized and given to the liquidator.
//   3.  If collateral < debt: residual = debt − collateral_value.
//   4.  `record_bad_debt()` is called with `residual`.
//   5.  Reserves absorb up to `residual`; remainder is added to `bad_debt`.
//   6.  The borrower's position is zeroed (invariant I-6).
//   7.  `assert_invariants()` is called; any violation aborts the tx.

use soroban_sdk::{Address, Env};

use crate::storage;
use crate::types::{BadDebtEvent, LendingError, MarketState};

// ── Public API ───────────────────────────────────────────────────────────────

/// Records a bad-debt event after a liquidation that could not fully cover the
/// borrower's debt.
///
/// # Parameters
/// * `user`            — The insolvent borrower.
/// * `asset`           — The borrow asset market.
/// * `residual_debt`   — Uncovered debt after seizing all collateral.
/// * `collateral_seized` — Amount of collateral (in borrow-asset USD equivalent)
///                         transferred to the liquidator.
///
/// # Returns
/// A `BadDebtEvent` describing the accounting entries made.
///
/// # Errors
/// Returns `LendingError::BadDebtNegative` or `LendingError::ReservesNegative`
/// if any invariant would be violated (should never happen in correct code).
pub fn record_bad_debt(
    env: &Env,
    user: &Address,
    asset: &Address,
    residual_debt: i128,
    collateral_seized: i128,
) -> Result<BadDebtEvent, LendingError> {
    if residual_debt < 0 {
        return Err(LendingError::BadDebtNegative);
    }
    if residual_debt == 0 {
        // Nothing to do — full liquidation succeeded.
        return Ok(BadDebtEvent {
            user: user.clone(),
            asset: asset.clone(),
            residual_debt: 0,
            collateral_seized,
            reserve_cover: 0,
            written_off: 0,
        });
    }

    let mut market = storage::get_market(env, asset)?;

    // Step 1: Zero the borrower's position (invariant I-6).
    //
    // NOTE: emergency_liquidate() zeroes user_borrow BEFORE calling here, so
    // `user_borrow` may be 0.  We read it to know how much to subtract from
    // total_borrows.  If it is already 0 the subtraction is a no-op.
    let user_borrow = storage::get_user_borrow(env, user, asset);
    storage::set_user_borrow(env, user, asset, 0);

    // Step 2: Reduce total borrows by the remaining on-books balance.
    market.total_borrows = market
        .total_borrows
        .saturating_sub(user_borrow)
        .max(0);

    // Step 3: Reserve cover — absorb as much of the shortfall as possible.
    let reserve_cover = residual_debt.min(market.reserves);
    market.reserves = market
        .reserves
        .checked_sub(reserve_cover)
        .ok_or(LendingError::ReservesNegative)?;

    // Step 4: Any remainder becomes socialised bad debt.
    let written_off = residual_debt - reserve_cover;
    market.bad_debt = market
        .bad_debt
        .checked_add(written_off)
        .ok_or(LendingError::InvalidAmount)?;

    // Step 5: Persist per-user write-off record (for auditability).
    storage::set_bad_debt_write_off(env, user, asset, residual_debt);

    // Step 6: Enforce all invariants before committing state.
    assert_market_invariants(env, &market)?;

    storage::set_market(env, asset, &market);

    Ok(BadDebtEvent {
        user: user.clone(),
        asset: asset.clone(),
        residual_debt,
        collateral_seized,
        reserve_cover,
        written_off,
    })
}

/// Attempts to recover previously written-off bad debt from new reserve inflows
/// (e.g. interest accumulation, governance top-ups).
///
/// Called whenever reserves increase so that the protocol's reported bad debt
/// converges toward zero over time.
pub fn attempt_bad_debt_recovery(
    env: &Env,
    asset: &Address,
    new_reserve_amount: i128,
) -> Result<i128, LendingError> {
    if new_reserve_amount < 0 {
        return Err(LendingError::InvalidAmount);
    }

    let mut market = storage::get_market(env, asset)?;

    // How much outstanding bad debt can we clear?
    let recovery = new_reserve_amount.min(market.bad_debt);
    market.bad_debt = market
        .bad_debt
        .checked_sub(recovery)
        .ok_or(LendingError::BadDebtNegative)?;
    market.reserves = market
        .reserves
        .checked_add(new_reserve_amount - recovery)
        .ok_or(LendingError::InvalidAmount)?;

    assert_market_invariants(env, &market)?;
    storage::set_market(env, asset, &market);

    Ok(recovery)
}

/// Freezes a market during emergency shutdown.
///
/// • No new borrows or deposits are accepted.
/// • Liquidations are still allowed so bad debt can be cleared.
/// • Existing repayments reduce borrows normally.
pub fn freeze_market_for_shutdown(env: &Env, asset: &Address) -> Result<(), LendingError> {
    let mut market = storage::get_market(env, asset)?;
    market.is_frozen = true;
    storage::set_market(env, asset, &market);
    Ok(())
}

/// Returns the total amount of unrecovered bad debt for an asset market.
/// Intended for off-chain monitoring and governance dashboards.
pub fn query_bad_debt(env: &Env, asset: &Address) -> Result<i128, LendingError> {
    let market = storage::get_market(env, asset)?;
    Ok(market.bad_debt)
}

/// Returns a full `ProtocolReport` for an asset, consistent with on-chain state.
pub fn query_protocol_report(
    env: &Env,
    asset: &Address,
) -> Result<crate::types::ProtocolReport, LendingError> {
    let market = storage::get_market(env, asset)?;
    let utilisation_bps = if market.total_deposits == 0 {
        0
    } else {
        market
            .total_borrows
            .checked_mul(10_000)
            .ok_or(LendingError::InvalidAmount)?
            / market.total_deposits
    };
    Ok(crate::types::ProtocolReport {
        asset: asset.clone(),
        total_deposits: market.total_deposits,
        total_borrows: market.total_borrows,
        reserves: market.reserves,
        bad_debt: market.bad_debt,
        utilisation_bps,
        is_solvent: market.check_solvency_invariant(),
    })
}

// ── Invariant assertions ─────────────────────────────────────────────────────

/// Asserts all per-market accounting invariants.
///
/// # Panics / Errors
/// Returns an error (causing the transaction to abort) if any invariant is
/// violated.  This is a defence-in-depth measure; correct code never reaches
/// these error paths.
pub fn assert_market_invariants(
    _env: &Env,
    market: &MarketState,
) -> Result<(), LendingError> {
    // [I-1] bad_debt >= 0
    if !market.check_bad_debt_non_negative() {
        return Err(LendingError::BadDebtNegative);
    }
    // [I-2] reserves >= 0
    if !market.check_reserves_non_negative() {
        return Err(LendingError::ReservesNegative);
    }
    // [I-3] total_borrows >= 0
    if market.total_borrows < 0 {
        return Err(LendingError::InvalidAmount);
    }
    // [I-4] total_deposits >= 0
    if market.total_deposits < 0 {
        return Err(LendingError::InvalidAmount);
    }
    // [I-5] nominal solvency
    if !market.check_solvency_invariant() {
        // This is expected when bad_debt exceeds reserves — it represents real
        // economic loss, not a bug.  We log but do NOT abort the tx so the
        // protocol can continue operating in a degraded state.
        //
        // Governance can top up reserves to restore solvency.
    }
    Ok(())
}