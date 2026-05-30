use soroban_sdk::contracttype;

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingMode {
    Truncate,
    Floor,
    Bankers,
    Ceil,
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingError {
    InvalidParameters,
    Overflow,
    UnacceptableDrift,
}

pub const INTEREST_PRECISION: i128 = 1_000_000;
pub const SECONDS_PER_YEAR: u64 = 365 * 24 * 60 * 60;
pub const BASIS_POINTS_SCALE: i128 = 10_000;

#[derive(Clone, Debug)]
pub struct InterestCalcResult {
    pub interest: i128,
    pub remainder: i128,
    pub total_drift: i128,
    pub mode: RoundingMode,
}

impl InterestCalcResult {
    pub fn new(interest: i128, remainder: i128, mode: RoundingMode) -> Self {
        InterestCalcResult {
            interest,
            remainder,
            total_drift: remainder,
            mode,
        }
    }
}

pub fn calculate_interest_with_rounding(
    borrowed_amount: i128,
    elapsed_seconds: u64,
    rate_bps: i128,
    mode: RoundingMode,
) -> Result<InterestCalcResult, RoundingError> {
    if borrowed_amount < 0 || rate_bps < 0 {
        return Err(RoundingError::InvalidParameters);
    }
    if borrowed_amount == 0 {
        return Ok(InterestCalcResult::new(0, 0, mode));
    }
    let amount_times_seconds = borrowed_amount
        .checked_mul(elapsed_seconds as i128)
        .ok_or(RoundingError::Overflow)?;
    let amount_times_seconds_times_rate = amount_times_seconds
        .checked_mul(rate_bps)
        .ok_or(RoundingError::Overflow)?;
    let with_precision = amount_times_seconds_times_rate
        .checked_mul(INTEREST_PRECISION)
        .ok_or(RoundingError::Overflow)?;
    let denominator = (SECONDS_PER_YEAR as i128)
        .checked_mul(BASIS_POINTS_SCALE)
        .ok_or(RoundingError::Overflow)?;
    let full_division = with_precision / denominator;
    let remainder = with_precision % denominator;
    let (rounded_interest, _actual_remainder) =
        apply_rounding(full_division, remainder, denominator, mode);
    let final_interest = rounded_interest / INTEREST_PRECISION;
    let final_remainder = rounded_interest % INTEREST_PRECISION;
    Ok(InterestCalcResult::new(
        final_interest,
        final_remainder,
        mode,
    ))
}

fn apply_rounding(
    quotient: i128,
    remainder: i128,
    divisor: i128,
    mode: RoundingMode,
) -> (i128, i128) {
    let half_divisor = divisor / 2;
    match mode {
        RoundingMode::Truncate | RoundingMode::Floor => (quotient, remainder),
        RoundingMode::Bankers => {
            if remainder < half_divisor {
                (quotient, remainder)
            } else if remainder > half_divisor {
                (quotient + 1, remainder - divisor)
            } else {
                if quotient % 2 == 0 { (quotient, remainder) } else { (quotient + 1, remainder - divisor) }
            }
        }
        RoundingMode::Ceil => {
            if remainder == 0 { (quotient, 0) } else { (quotient + 1, remainder - divisor) }
        }
    }
}
pub fn reconcile_debt_with_drift_correction(
    stored_debt: i128,
    freshly_calculated_debt: i128,
    accumulated_drift: i128,
    max_allowed_drift_bps: i128,
)-> Result<(i128, i128), RoundingError> {
    let debt_basis = if stored_debt > 0 {
        (freshly_calculated_debt - stored_debt) * 10000 / stored_debt
    } else {
        0
    };
    if debt_basis.abs() > max_allowed_drift_bps {
        return Err(RoundingError::UnacceptableDrift);
    }
    Ok((
        freshly_calculated_debt,
        accumulated_drift + (freshly_calculated_debt - stored_debt),
    ))
}
