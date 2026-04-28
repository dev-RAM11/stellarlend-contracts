# Security Notes & Trust Boundaries

## Trust Boundaries
- **Admins:** The highest level of privilege. Admins can update parameters (such as minimum borrow amounts, deposit ceilings, and oracles), pause the protocol, trigger emergency shutdown, and designate guardians. They are also responsible for upgrading the protocol.
- **Guardians:** Designed for rapid response. Guardians can only trigger emergency shutdowns. They cannot upgrade contracts, unpause the system, or change parameters.
- **Users:** End-users interact with the protocol via `deposit`, `borrow`, `repay`, and `withdraw` mechanisms subject to protocol checks. User operations are sandboxed to their respective `Address` scopes.
- **Oracles:** Trusted entities providing price feeds used for health factor checks. If an oracle becomes malicious, it could trigger improper liquidations, but internal checks restrict maximum liquidation amounts (via close factor limits).

## Authorization Model
All external entry points modifying state or user balances call `user.require_auth()`. This delegates authorization entirely to the Soroban SDK's robust authorization framework. 
Protocol functions restricted to Admins enforce validation via `admin.require_auth()` and ensure the caller matches the registered Admin in the data store.

## Reentrancy Protections
In Soroban, contract logic guarantees atomicity. However, as an added measure against logic-based reentrancy across cross-contract calls:
- All external calls to update state (e.g. `save_deposit_position`) occur *before* external token transfers where applicable (the Checks-Effects-Interactions pattern).
- High-risk operations are guarded by global pause mappings which an Admin or Guardian can engage via the pause module if anomalous behavior occurs.

## Cross-Asset Module Hardening
- **Token Transfer Enforcement:** All position operations (`deposit`, `borrow`, `repay`, `withdraw`) now explicitly enforce token transfers via the Soroban `token::Client`.
- **Granular Pause Support:** Cross-asset operations now respect specific `PauseType` settings (e.g. `PauseType::Borrow`), allowing for targeted emergency interventions.
- **Event-Driven Transparency:** Each significant operation emits a unique contract event (`CrossDepositEvent`, etc.), facilitating robust off-chain monitoring and audit trails.
- **Initialization Safety:** The `initialize_admin` function now returns a `Result` and prevents re-initialization if an admin is already set.

## Arithmetic Bounds
Protocol parameters strictly utilize `checked_add`, `checked_sub`, `checked_mul`, and `checked_div` to prevent overflow and underflow paths. Zero-amount and uninitialized parameter paths intentionally return structured `ContractError` values rather than panicking where possible.

## Withdraw path (`withdraw.rs`)
- **Pause module**: Withdraw is blocked when `pause::is_paused(Withdraw)` is true (this includes global `PauseType::All`), when the legacy `WithdrawDataKey::Paused` flag is set, or when the protocol is in **emergency shutdown** (`blocks_high_risk_ops` and not in **recovery**). In **recovery**, users may still withdraw (and repay) to unwind positions.
- **Collateral ratio**: Post-withdraw collateral must satisfy the same minimum ratio as borrows, via shared `borrow::validate_collateral_ratio` (150% default, `MIN_COLLATERAL_RATIO_BPS`).
- **Authorization**: Only the position owner can withdraw; `user.require_auth()` is enforced before state changes.

### Liquidation Boundary and Health Factor Scaling
The protocol represents the Health Factor using a scalar where `10_000` equates to `1.0`. 
To ensure determinism and avoid rounding ambiguity, the protocol strictly enforces the `<` threshold for liquidation eligibility. 
* A position with a Health Factor `<= 9_999` **is eligible** for liquidation.
* A position with a Health Factor `>= 10_000` **is completely immune** to liquidation. 

There are no edge cases where a `10_000` Health Factor allows for liquidation. All price oracle rounding uses integer truncations designed to safely error on the side of protecting the borrower from false-positive liquidations.

## Oracle Migration Risks and Mitigation

Changing the protocol oracle (either the legacy address or the hardened module primary/fallback slots) is a high-risk administrative action that impacts the valuation of all open positions.

### Risks
- **Price Jump Liquidation**: Swapping to an oracle that reports a significantly lower price for collateral (or higher for debt) can instantly push healthy positions into liquidation eligibility.
- **Staleness Gaps**: If a new oracle has not yet submitted a price feed, valuation will fail (returning 0), blocking withdrawals and potentially enabling liquidations if not handled safely.
- **Misconfiguration**: Setting an incorrect oracle address or one with different decimal scales (the protocol expects 8 decimals) leads to incorrect health factor calculations.

### Mitigation and Operational Guidance
- **Deterministic Precedence**: The protocol prioritizes the Hardened Oracle Module over the Legacy Oracle address. This allows for a "staged" migration where a hardened feed is configured and verified before removing the legacy fallback.
- **Auditable Transitions**: All oracle changes emit events (`OracleSetEvent` or `OracleConfigEvent`) containing the admin, the new address, and the timestamp, ensuring a clear audit trail of price-source transitions.
- **Safe Failure Modes**: If an oracle returns an invalid price or is missing, the health factor defaults to 0. The `liquidate` function explicitly rejects positions with HF=0 to prevent "phantom liquidations" caused by missing price data.
- **Pre-Migration Valuation**: Admins should use view functions (`get_user_position`) with the proposed oracle price off-chain before committing the change on-chain to ensure no mass-liquidation event is triggered.
