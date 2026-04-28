# AMM Pool Accounting & Math Invariants

This document outlines the core mathematical guarantees, boundaries, and pool accounting properties of the AMM router module (`stellar-lend/contracts/amm`). These rules apply to the `add_liquidity` and `remove_liquidity` operations managed internally by the AMM wrapper contract.

> [!NOTE]
> The AMM wrapper acts as a unified interface to registered external DEX protocols. While `swap` operations delegate directly to external protocols and rely on their internal state, the wrapper **independently tracks LP share issuance** for liquidity provisioning directly through its interface to support cross-protocol standardization.

## 1. LP Share Minting (Bootstrapping vs Proportional)

The AMM enforces two distinct phases for minting LP tokens, heavily prioritizing pool solvency and preventing dilution.

### Initial Bootstrapping
When the pool has exactly `0` total LP shares, the pool is bootstrapped using a standard geometric mean:
```text
minted_lp = floor(sqrt(amount_a * amount_b))
```
- A strict `floor()` function is applied. This means any fractional value is discarded, ensuring the pool is always marginally over-collateralized from the first deposit.

### Proportional Minting
Once the pool has active shares (`total_lp > 0`), all subsequent deposits are minted proportionally to the existing reserves to prevent value extraction:
```text
minted_lp = floor(min(
    (amount_a * total_lp) / reserve_a,
    (amount_b * total_lp) / reserve_b
))
```
- Using `min()` forces the depositor to accept LP shares based on whichever supplied asset ratio represents a *smaller* share of the pool.
- Any "excess" supplied tokens that do not match the exact pool ratio are **left in the caller's wallet** (they are not pulled into the pool for free).

## 2. Liquidity Removal (Round-Trip Protection)

When burning LP tokens to retrieve underlying assets, the AMM calculates the returned value as:
```text
amount_a = floor((lp_tokens * reserve_a) / total_lp)
amount_b = floor((lp_tokens * reserve_b) / total_lp)
```

### The "No Extraction" Invariant
Because both minting and burning use `floor()` rounding, the protocol guarantees that a user performing an atomic deposit and immediate withdrawal will **never receive more tokens than they put in**. 

*Example:* A user deposits exactly enough to calculate `100.9` LP tokens. They receive `100` LP tokens. If they burn `100` LP tokens immediately, the withdrawal math computes a return of `99.1%` of their original deposit, floored to `99%`. The pool accumulates the `~1%` fractional dust, making the remaining LP shares strictly more valuable.

## 3. Strict Boundary Guarantees

| Boundary / Constraint | Enforcement Mechanism | Failure Mode |
|----------------------|-----------------------|--------------|
| **Zero/Negative Amounts** | Input bounds checks reject `amount <= 0`. | Reverts with `InvalidSwapParams`. |
| **Over-Withdrawal** | Burns are capped at `lp_tokens <= total_lp_shares`. | Reverts with `InsufficientLiquidity`. |
| **Slippage Protection** | `amount_out < min_amount_out` checks on all operations. | Reverts with `MinOutputNotMet`. |
| **Arithmetic Overflow** | All math utilizes `checked_add`, `checked_mul`, etc. | Reverts with `Overflow` (graceful failure instead of panic). |

## 4. Threat Model & Integrator Assumptions

> [!WARNING]
> **Independent Accounting:** The AMM wrapper's `PoolState` strictly tracks liquidity that was added **through this contract**. It does *not* synchronize with or read the external protocol's underlying pool reserves.

If an external protocol suffers an exploit or experiences severe impermanent loss, the `PoolState` in this wrapper will still faithfully represent the *ratio* of liquidity added through the wrapper. However, integrators must ensure the underlying external AMM correctly handles the actual token custody and respects the parameters forwarded to it.
