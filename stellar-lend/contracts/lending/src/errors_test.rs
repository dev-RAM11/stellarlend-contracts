#![cfg(test)]
use crate::errors::{
    BorrowError, CrossAssetError, DepositError, FlashLoanError, OracleError, WithdrawError,
};

#[test]
fn test_borrow_error_stability() {
    assert_eq!(BorrowError::InsufficientCollateral as u32, 1001);
    assert_eq!(BorrowError::DebtCeilingReached as u32, 1002);
    assert_eq!(BorrowError::ProtocolPaused as u32, 1003);
    assert_eq!(BorrowError::InvalidAmount as u32, 1004);
    assert_eq!(BorrowError::Overflow as u32, 1005);
    assert_eq!(BorrowError::Unauthorized as u32, 1006);
    assert_eq!(BorrowError::AssetNotSupported as u32, 1007);
    assert_eq!(BorrowError::BelowMinimumBorrow as u32, 1008);
    assert_eq!(BorrowError::RepayAmountTooHigh as u32, 1009);
}

#[test]
fn test_deposit_error_stability() {
    assert_eq!(DepositError::InvalidAmount as u32, 2001);
    assert_eq!(DepositError::DepositPaused as u32, 2002);
    assert_eq!(DepositError::Overflow as u32, 2003);
    assert_eq!(DepositError::AssetNotSupported as u32, 2004);
    assert_eq!(DepositError::ExceedsDepositCap as u32, 2005);
    assert_eq!(DepositError::Unauthorized as u32, 2006);
}

#[test]
fn test_withdraw_error_stability() {
    assert_eq!(WithdrawError::InvalidAmount as u32, 3001);
    assert_eq!(WithdrawError::WithdrawPaused as u32, 3002);
    assert_eq!(WithdrawError::Overflow as u32, 3003);
    assert_eq!(WithdrawError::InsufficientCollateral as u32, 3004);
    assert_eq!(WithdrawError::InsufficientCollateralRatio as u32, 3005);
    assert_eq!(WithdrawError::Unauthorized as u32, 3006);
}

#[test]
fn test_flash_loan_error_stability() {
    assert_eq!(FlashLoanError::InvalidAmount as u32, 4001);
    assert_eq!(FlashLoanError::InsufficientRepayment as u32, 4002);
    assert_eq!(FlashLoanError::Unauthorized as u32, 4003);
    assert_eq!(FlashLoanError::InvalidFee as u32, 4004);
    assert_eq!(FlashLoanError::CallbackFailed as u32, 4005);
    assert_eq!(FlashLoanError::Reentrancy as u32, 4006);
    assert_eq!(FlashLoanError::ProtocolPaused as u32, 4007);
}

#[test]
fn test_oracle_error_stability() {
    assert_eq!(OracleError::InvalidPrice as u32, 5001);
    assert_eq!(OracleError::StalePrice as u32, 5002);
    assert_eq!(OracleError::Unauthorized as u32, 5003);
    assert_eq!(OracleError::NoPriceFeed as u32, 5004);
    assert_eq!(OracleError::InvalidOracle as u32, 5005);
    assert_eq!(OracleError::OraclePaused as u32, 5006);
}

#[test]
fn test_cross_asset_error_stability() {
    assert_eq!(CrossAssetError::InsufficientCollateral as u32, 6001);
    assert_eq!(CrossAssetError::DebtCeilingReached as u32, 6002);
    assert_eq!(CrossAssetError::ProtocolPaused as u32, 6003);
    assert_eq!(CrossAssetError::InvalidAmount as u32, 6004);
    assert_eq!(CrossAssetError::Overflow as u32, 6005);
    assert_eq!(CrossAssetError::Unauthorized as u32, 6006);
    assert_eq!(CrossAssetError::AssetNotSupported as u32, 6007);
    assert_eq!(CrossAssetError::PriceUnavailable as u32, 6008);
    assert_eq!(CrossAssetError::AlreadyInitialized as u32, 6009);
}