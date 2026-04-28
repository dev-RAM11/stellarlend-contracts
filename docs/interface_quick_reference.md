# Contract Interface Quick Reference

This document provides a single source of truth for frontend integrators interacting with the StellarLend smart contracts.

## 1. Unit Scales & Precisions
| Parameter | Scale | Description |
|-----------|-------|-------------|
| Amounts | 10^7 | All asset amounts (XLM, USDC, etc.) use 7 decimal places. |
| Health Factor | 10^4 | 1.0 = 10000. Values < 10000 are subject to liquidation. |
| BPS (Basis Points) | 10^4 | 1% = 100 BPS. Used for interest rates and fees. |
| Timestamps | Seconds | Unix epoch timestamps. |

## 2. Core View Functions
### `get_protocol_report()`
**Returns:** `ProtocolReport`
- **Use Case:** Dashboard stats (TVL, Total Borrows, Utilization).

### `get_user_report(user: Address)`
**Returns:** `UserReport`
- **Use Case:** User portfolio, health factor, and active positions.

## 3. Error Mapping Guidance
| Error Code | Name | UI Suggestion |
|------------|------|---------------|
| 1 | InsufficientCollateral | "You need more collateral to borrow this amount." |
| 8 | BelowMinimumBorrow | "Amount is too small. Minimum borrow required." |
| 3 | ProtocolPaused | "Borrowing is temporarily disabled for maintenance." |

## 4. Events to Subscribe
- `BorrowEvent`: Emitted on successful loan.
- `RepayEvent`: Emitted on debt repayment.
- `LiquidationEvent`: Emitted when a position is liquidated.

## 5. Integration Checklist
- [ ] Convert UI inputs to 10^7 scale before sending to contract.
- [ ] Check `health_factor` from `get_user_report` before allowing further borrows.
- [ ] Verify `user.require_auth()` is handled by the wallet connector.
