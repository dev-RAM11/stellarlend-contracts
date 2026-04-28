# Developer Glossary

This glossary defines key protocol terms, numeric scales, and common pitfalls for the StellarLend protocol.

## Protocol Terms

### Basis Points (BPS)
- **Definition**: A unit of measure for percentages in the protocol. 1 basis point = 0.01%.
- **Scale**: `10,000` = `100%`.
- **Usage**: Used for interest rates, collateral factors, liquidation thresholds, and fees.
- **Example**: `1,000 BPS` = `10%`.

### Health Factor (HF)
- **Definition**: A numeric representation of the safety of a user's borrow position. A position is healthy if its health factor is greater than 1.0.
- **Scale**: `10,000` = `1.0`.
- **Threshold**: Below `10,000` means the position is eligible for liquidation.
- **Formula**: `health_factor = (collateral_value * liquidation_threshold_bps / 10000) * 10000 / debt_value`.
- **Special Case**: A position with no debt has a sentinel health factor of `100,000,000`.

### Close Factor
- **Definition**: The maximum proportion of a distressed borrower's debt that a liquidator can repay in a single transaction.
- **Scale**: Basis points (`5,000` = `50%`).
- **Safety Limit**: Usually capped at `7,500` (75%) to prevent total wipeout in a single block.

### Reserve Factor
- **Definition**: The percentage of interest paid by borrowers that is redirected to the protocol treasury rather than distributed to lenders.
- **Scale**: Basis points (`1,000` = `10%`).
- **Range**: `0 - 5,000 BPS` (0% - 50%).

### Utilization Rate
- **Definition**: The ratio of total borrowed funds to total deposited funds for a given asset.
- **Scale**: Basis points (`8,000` = `80%`).
- **Formula**: `utilization = (total_borrows * 10,000) / total_deposits`.
- **Impact**: Higher utilization typically triggers higher interest rates via the "kink" model.

### Minimum Collateral Ratio (MCR)
- **Definition**: The minimum ratio of collateral value to debt value that a user must maintain to stay in good standing.
- **Scale**: Basis points (`11,000` = `110%`).
- **Requirement**: Users cannot withdraw collateral or borrow more if it would push their ratio below the MCR.

### Liquidation Threshold
- **Definition**: The specific collateral ratio at which a borrower is considered distressed and eligible for liquidation.
- **Scale**: Basis points (`10,500` = `105%`).
- **Invariant**: Must always be less than or equal to the MCR.

### Liquidation Incentive (Bonus)
- **Definition**: The bonus given to liquidators for helping clear bad debt from the protocol. It is paid out in the borrower's collateral at a discount.
- **Scale**: Basis points (`1,000` = `10%`).
- **Example**: A 10% incentive means a liquidator receives $110 worth of collateral for every $100 of debt repaid.

## Numeric Scales Summary

| Term | Scale | Example |
|------|-------|---------|
| Percentages (BPS) | `10,000 = 100%` | `500 = 5%` |
| Health Factor | `10,000 = 1.0` | `12,500 = 1.25` |
| Utilization | `10,000 = 100%` | `8,000 = 80%` |
| Oracle Price | `100,000,000 = 1.0` | `10^8` scaling |

## Common Pitfalls

### 1. Rounding Directions
- **Debt/Interest**: The protocol generally **rounds up** (favors the protocol/lenders) when calculating interest and debt to prevent dust accumulation and ensure solvency.
- **Collateral**: The protocol generally **rounds down** (favors the protocol) when calculating maximum borrowable amounts.

### 2. Decimal Scaling
- Different tokens on Stellar have different decimals (e.g., XLM has 7, others may have 8, 12, or 14).
- Always normalize to a common scale (usually 18 decimals internally or using the oracle's 8-decimal scale) before performing cross-asset comparisons.
- **Example**: Comparing 100 XLM (7 decimals) to 100 USDC (6 decimals) requires scaling both to a common denominator.

### 3. Stale Prices
- Health factors and liquidation eligibility depend on oracle prices.
- Integrators should check the `last_updated` timestamp of prices. The protocol enforces a Heartbeat/TTL, but frontends should provide visual cues for stale data.

### 4. Health Factor vs. Collateral Ratio
- **Collateral Ratio** is `Collateral Value / Debt Value`.
- **Health Factor** is `(Collateral Value * Liquidation Threshold) / Debt Value`.
- A position is liquidatable when `Collateral Ratio < Liquidation Threshold`, which is equivalent to `Health Factor < 1.0`.
