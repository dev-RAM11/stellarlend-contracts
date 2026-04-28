# Lending Contract Error Registry

This document serves as a reference for integrators and frontends interacting with the StellarLend protocol. Error codes are mapped to specific numeric domains and are guaranteed to remain stable across contract upgrades.

## Integration Notes for Frontend
When parsing transaction failures from Soroban, extract the `u32` error code from the revert payload and map it to the corresponding UI message using the tables below. Do not rely on string matching, as internal Rust enum names are stripped in WebAssembly.

---

## Borrowing & Repayment (1000s)
| Code | Name | Description | Mitigation |
| :--- | :--- | :--- | :--- |
| `1001` | `InsufficientCollateral` | Collateral is too low for the borrow. | Check `get_health_factor` before submitting. Deposit more collateral. |
| `1002` | `DebtCeilingReached` | Protocol-wide debt limit reached. | Wait for other users to repay or governance to raise the cap. |
| `1003` | `ProtocolPaused` | Borrow operations are halted. | Verify protocol state via `get_pause_state`. |
| `1004` | `InvalidAmount` | Amount is zero or negative. | Enforce positive inputs on the frontend. |
| `1005` | `Overflow` | Mathematical overflow occurred. | Check for extreme token amounts. |
| `1006` | `Unauthorized` | Caller lacks permissions. | Ensure transaction is signed by the correct account. |
| `1007` | `AssetNotSupported` | Asset is not valid for borrowing. | Check the protocol's supported asset list. |
| `1008` | `BelowMinimumBorrow` | Request does not meet the minimum size. | Increase the requested borrow amount. |
| `1009` | `RepayAmountTooHigh` | Repayment exceeds total debt. | Cap repayment input to `get_debt_balance`. |

## Deposits (2000s)
| Code | Name | Description | Mitigation |
| :--- | :--- | :--- | :--- |
| `2001` | `InvalidAmount` | Deposit amount is invalid/below minimum. | Enforce positive inputs on the frontend. |
| `2002` | `DepositPaused` | Deposit operations are halted. | Verify protocol state via `get_pause_state`. |
| `2003` | `Overflow` | Mathematical overflow occurred. | Check for extreme token amounts. |
| `2004` | `AssetNotSupported` | Asset is not valid for deposit. | Check supported asset configurations. |
| `2005` | `ExceedsDepositCap` | Protocol global deposit limit reached. | Wait for capacity to open up. |
| `2006` | `Unauthorized` | Caller lacks permissions. | Ensure proper signature/auth. |

## Withdrawals (3000s)
| Code | Name | Description | Mitigation |
| :--- | :--- | :--- | :--- |
| `3001` | `InvalidAmount` | Withdraw amount is invalid. | Enforce positive inputs on the frontend. |
| `3002` | `WithdrawPaused` | Withdraw operations are halted. | Verify protocol state via `get_pause_state`. |
| `3003` | `Overflow` | Mathematical overflow occurred. | Check for extreme token amounts. |
| `3004` | `InsufficientCollateral` | Withdrawing more than available balance. | Check user balance before transacting. |
| `3005` | `InsufficientCollateralRatio` | Withdrawal would cause undercollateralization. | Repay debt before withdrawing. |
| `3006` | `Unauthorized` | Caller lacks permissions. | Ensure proper signature/auth. |

## Flash Loans (4000s)
| Code | Name | Description | Mitigation |
| :--- | :--- | :--- | :--- |
| `4001` | `InvalidAmount` | Loan amount is zero or negative. | Supply a valid positive amount. |
| `4002` | `InsufficientRepayment` | Callback did not return enough funds + fee. | Ensure receiver contract logic approves the fee transfer. |
| `4003` | `Unauthorized` | Caller lacks permissions. | Ensure proper signature/auth. |
| `4004` | `InvalidFee` | Configured fee exceeds maximum allowed. | Admin must reconfigure fee within bounds. |
| `4005` | `CallbackFailed` | Receiver `on_flash_loan` returned false. | Debug the receiver contract logic. |
| `4006` | `Reentrancy` | Reentrant call detected. | Do not nest flash loans from the same protocol. |
| `4007` | `ProtocolPaused` | Flash loans are halted. | Verify protocol state. |

## Oracles (5000s)
| Code | Name | Description | Mitigation |
| :--- | :--- | :--- | :--- |
| `5001` | `InvalidPrice` | Submitted price is zero or negative. | Validate upstream data sources. |
| `5002` | `StalePrice` | Price feed exceeds staleness bounds. | Wait for oracle keepers to push an update. |
| `5003` | `Unauthorized` | Caller is not a registered oracle. | Use a registered oracle address. |
| `5004` | `NoPriceFeed` | No price data exists for this asset. | Admin must configure the price feed. |
| `5005` | `InvalidOracle` | Oracle address configuration is invalid. | Admin must correct oracle parameters. |
| `5006` | `OraclePaused` | Price updates are halted. | Wait for admin to unpause feeds. |

## Cross-Asset Operations (6000s)
| Code | Name | Description | Mitigation |
| :--- | :--- | :--- | :--- |
| `6001` | `InsufficientCollateral` | Action drops health factor below 1.0. | Adjust requested amounts to maintain health factor. |
| `6002` | `DebtCeilingReached` | Asset-specific debt ceiling exceeded. | Wait for capacity. |
| `6003` | `ProtocolPaused` | Cross-asset operations halted. | Verify protocol state. |
| `6004` | `InvalidAmount` | Amount is zero or negative. | Validate inputs. |
| `6005` | `Overflow` | Mathematical overflow occurred. | Check limits. |
| `6006` | `Unauthorized` | Caller lacks permissions. | Verify signatures. |
| `6007` | `AssetNotSupported` | Asset not configured for cross-margin. | Request governance to add asset. |
| `6008` | `PriceUnavailable` | Missing oracle data for cross-margin calculation. | Wait for oracle update. |
| `6009` | `AlreadyInitialized` | Protocol is already initialized. | No action required. |