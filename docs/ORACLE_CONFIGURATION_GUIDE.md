# Oracle Configuration Management Guide

## Overview

This document outlines the procedures for managing oracle configurations in the StellarLend protocol, including role separation, security considerations, and operational guidelines.

## Architecture Overview

### Components

1. **Off-chain Oracle Service** (TypeScript/Node.js)
   - Fetches prices from multiple external sources
   - Aggregates and validates price data
   - Updates smart contract with validated prices

2. **Smart Contract Oracle Module** (Rust/Soroban)
   - Stores on-chain price feeds
   - Enforces validation rules and role separation
   - Manages oracle configuration and permissions

3. **Price Providers**
   - CoinGecko (primary, 60% weight)
   - Binance (secondary, 40% weight)
   - CoinMarketCap (optional, 35% weight)

## Role-Based Access Control

### Roles and Permissions

| Role | Permissions | Responsibilities |
|------|-------------|------------------|
| **Admin** | - Configure oracle parameters<br>- Set/remove primary oracles<br>- Set/remove fallback oracles<br>- Pause/resume oracle updates<br>- Update prices directly | - System configuration<br>- Oracle management<br>- Emergency operations |
| **Primary Oracle** | - Update prices for registered assets<br>- Read price feeds | - Regular price updates<br>- Market data provision |
| **Fallback Oracle** | - Update prices when primary is stale<br>- Read fallback price feeds | - Backup price provision<br>- Redundancy support |
| **Public/Other** | - Read price feeds only | - Price consumption |

### Authorization Flow

1. **Admin Operations**: Require admin address verification
2. **Oracle Operations**: Verify oracle is registered for the asset
3. **Price Updates**: Validate caller authorization and price data
4. **Configuration Changes**: Admin-only with additional validation

## Configuration Parameters

### Oracle Safety Parameters

```rust
pub struct OracleConfig {
    /// Maximum price deviation in basis points (e.g., 500 = 5%)
    pub max_deviation_bps: i128,
    /// Maximum staleness in seconds (global default for all assets)
    pub max_staleness_seconds: u64,
    /// Cache TTL in seconds
    pub cache_ttl_seconds: u64,
    /// Minimum price sanity check
    pub min_price: i128,
    /// Maximum price sanity check
    pub max_price: i128,
}
```

### Per-Asset Staleness Configuration (Issue #645)

In addition to the global `max_staleness_seconds`, each asset can have its own
staleness limit. This is useful when different assets have different oracle update
cadences — for example, a stablecoin oracle that updates every 60 seconds versus
a long-tail asset oracle that updates every 30 minutes.

**Resolution order** (most specific wins):
1. Per-asset override (`set_asset_max_staleness`) — if set for this asset.
2. Global config (`configure_oracle`) — if no per-asset override.
3. Hard-coded default (3 600 s) — if neither has been stored.

**New contract functions:**

| Function | Description |
|----------|-------------|
| `set_asset_max_staleness(caller, asset, seconds)` | Set per-asset staleness limit. Admin only. `seconds` must be > 0. |
| `clear_asset_max_staleness(caller, asset)` | Remove per-asset override; reverts to global config. Admin only. |
| `get_asset_max_staleness(asset)` | Read the effective staleness limit for `asset` (read-only). |

**Example — tighter limit for a stablecoin:**
```bash
# Stablecoin oracle updates every 60s; reject prices older than 90s
contract.set_asset_max_staleness(admin, usdc_address, 90)

# Verify
effective = contract.get_asset_max_staleness(usdc_address)
assert(effective == 90)
```

**Example — remove per-asset override:**
```bash
contract.clear_asset_max_staleness(admin, usdc_address)
# Now falls back to global max_staleness_seconds
```

**Storage layout note:** The per-asset override is stored under a new
`OracleKey::AssetStaleness(Address)` variant. This is additive — no existing
storage keys are modified and no migration is required when upgrading from a
version that did not have this feature.

### Provider Configuration

```typescript
interface ProviderConfig {
    name: string;
    enabled: boolean;
    priority: number;
    weight: number;
    apiKey?: string;
    baseUrl: string;
    rateLimit: {
        maxRequests: number;
        windowMs: number;
    };
}
```

## Configuration Procedures

### 1. Initial Oracle Setup

#### Prerequisites
- Admin privileges
- Oracle addresses (generated)
- Asset addresses
- Configuration parameters determined

#### Steps

1. **Configure Oracle Parameters**
```bash
# Set conservative initial parameters
max_deviation_bps: 500 (5%)
max_staleness_seconds: 3600 (1 hour)
cache_ttl_seconds: 300 (5 minutes)
min_price: 1
max_price: i128::MAX
```

2. **Set Primary Oracle**
```bash
# For each asset
contract.set_primary_oracle(admin, asset_address, oracle_address)
```

3. **Set Fallback Oracle** (optional but recommended)
```bash
contract.set_fallback_oracle(admin, asset_address, fallback_oracle_address)
```

4. **Initial Price Feed**
```bash
contract.update_price_feed(admin, asset_address, price, decimals, oracle_address)
```

### 2. Switching Primary Oracle

#### When to Switch
- Oracle provider compromise
- Long-term oracle unavailability
- Provider quality degradation
- Strategic provider changes

#### Procedure

1. **Prepare New Oracle**
```bash
# Generate new oracle address
# Verify oracle operational status
# Test oracle connectivity
```

2. **Update Configuration**
```bash
# Set new primary oracle
contract.set_primary_oracle(admin, asset_address, new_oracle_address)
```

3. **Verify Switch**
```bash
# Check oracle registration
primary_oracle = contract.get_primary_oracle(asset_address)
assert(primary_oracle == new_oracle_address)
```

4. **Update Price Feed**
```bash
# Admin updates price with new oracle
contract.update_price_feed(admin, asset_address, price, decimals, new_oracle_address)
```

5. **Monitor Operation**
```bash
# Verify new oracle can update prices
contract.update_price_feed(new_oracle_address, asset_address, price, decimals, new_oracle_address)
```

### 3. Adjusting Safety Parameters

#### Risk Assessment

| Parameter | Conservative | Moderate | Aggressive |
|-----------|-------------|----------|------------|
| max_deviation_bps | 200 (2%) | 500 (5%) | 1000 (10%) |
| max_staleness_seconds | 1800 (30min) | 3600 (1hr) | 7200 (2hr) |
| cache_ttl_seconds | 60 (1min) | 300 (5min) | 600 (10min) |

#### Procedure

1. **Assess Market Conditions**
```bash
# Analyze price volatility
# Consider asset characteristics
# Evaluate risk tolerance
```

2. **Update Configuration**
```bash
new_config = OracleConfig {
    max_deviation_bps: new_value,
    max_staleness_seconds: new_value,
    cache_ttl_seconds: new_value,
    min_price: current_min_price,
    max_price: current_max_price,
}

contract.configure_oracle(admin, new_config)
```

3. **Validate Configuration**
```bash
# Test with sample price updates
# Verify deviation limits work
# Check staleness enforcement
```

### 4. Emergency Procedures

#### Oracle Compromise Response

1. **Immediate Actions**
```bash
# Pause oracle updates
contract.pause_oracle_updates(admin)

# Remove compromised oracle
contract.set_primary_oracle(admin, asset_address, zero_address)
```

2. **Activate Fallback**
```bash
# Ensure fallback oracle is active
# Verify fallback oracle integrity
# Promote fallback if necessary
```

3. **Recovery**
```bash
# Deploy new oracle
# Update oracle registration
# Resume operations
contract.unpause_oracle_updates(admin)
```

#### Market Extreme Volatility

1. **Tighten Parameters**
```bash
# Reduce deviation threshold
max_deviation_bps = 200 (2%)

# Reduce staleness tolerance
max_staleness_seconds = 1800 (30 minutes)
```

2. **Increase Monitoring**
```bash
# More frequent price checks
# Manual price verification
# Consider temporary pause
```

## Security Considerations

### Access Control

1. **Admin Key Security**
   - Use multi-sig when possible
   - Store admin key securely
   - Rotate admin keys periodically
   - Limit admin key usage

2. **Oracle Key Security**
   - Separate keys for each oracle
   - Regular key rotation
   - Secure key storage
   - Access logging

### Validation Security

1. **Price Deviation Checks**
   - Always enforce deviation limits
   - Consider market conditions
   - Monitor for manipulation attempts
   - Alert on suspicious changes

2. **Staleness Protection**
   - Regular staleness checks
   - Fallback oracle activation
   - Manual intervention capability
   - Time synchronization

### Operational Security

1. **Provider Diversity**
   - Multiple independent sources
   - Geographic distribution
   - Different API providers
   - Failover mechanisms

2. **Rate Limiting**
   - Respect provider limits
   - Implement backoff strategies
   - Monitor API usage
   - Prevent abuse

## Monitoring and Alerting

### Key Metrics

1. **Price Update Frequency**
   - Time between updates
   - Update success rate
   - Failed update attempts
   - Provider response times

2. **Price Quality**
   - Deviation from expected
   - Cross-provider consistency
   - Staleness duration
   - Validation failures

3. **System Health**
   - Oracle availability
   - Provider status
   - Error rates
   - Performance metrics

### Alert Conditions

1. **Critical Alerts**
   - Oracle update failures > 5 minutes
   - Price deviation exceedance
   - Stale price detection
   - Configuration changes

2. **Warning Alerts**
   - High latency responses
   - Provider degradation
   - Near-limit rate usage
   - Unusual price patterns

## Testing Procedures

### Configuration Testing

1. **Unit Tests**
   - Parameter validation
   - Authorization checks
   - Edge case handling
   - Error conditions

2. **Integration Tests**
   - End-to-end flows
   - Provider switching
   - Failover scenarios
   - Performance testing

3. **Security Tests**
   - Unauthorized access attempts
   - Manipulation resistance
   - Parameter boundary testing
   - Role separation verification

### Operational Testing

1. **Disaster Recovery**
   - Oracle failure simulation
   - Provider outage testing
   - Configuration rollback
   - Emergency procedures

2. **Load Testing**
   - High update frequency
   - Multiple asset support
   - Concurrent operations
   - Resource limits

## Best Practices

### Configuration Management

1. **Version Control**
   - Track configuration changes
   - Document change reasons
   - Maintain change history
   - Rollback capability

2. **Review Process**
   - Multi-person review
   - Risk assessment
   - Testing requirements
   - Approval workflow

### Operational Excellence

1. **Gradual Changes**
   - Phase parameter adjustments
   - Monitor impact
   - Rollback capability
   - Communication plan

2. **Documentation**
   - Configuration rationale
   - Operational procedures
   - Emergency contacts
   - Troubleshooting guides

## Troubleshooting

### Common Issues

1. **Price Update Failures**
   - Check oracle authorization
   - Verify price deviation limits
   - Confirm staleness thresholds
   - Review provider status

2. **Configuration Problems**
   - Validate parameter ranges
   - Check admin authorization
   - Verify contract state
   - Review recent changes

3. **Performance Issues**
   - Monitor provider latency
   - Check rate limiting
   - Review cache settings
   - Analyze update frequency

### Diagnostic Commands

```bash
# Check oracle configuration
contract.get_oracle_config()

# Verify oracle registration
contract.get_primary_oracle(asset_address)
contract.get_fallback_oracle(asset_address)

# Check price feed status
contract.get_price(asset_address)

# System health check
contract.health_check()
```

## Compliance and Audit

### Audit Requirements

1. **Configuration Changes**
   - Change timestamps
   - Authorized users
   - Parameter values
   - Change justification

2. **Price Updates**
   - Update timestamps
   - Oracle addresses
   - Price values
   - Validation results

### Reporting

1. **Regular Reports**
   - Configuration status
   - Oracle performance
   - Security metrics
   - Compliance status

2. **Incident Reports**
   - Security events
   - System failures
   - Configuration issues
   - Resolution actions

## Oracle Failure Modes

This section documents how the protocol behaves under adversarial oracle conditions, based on the adversarial test suite in `contracts/lending/src/oracle_adversarial_test.rs`.

### Sudden Price Jumps and Crashes

| Scenario | Protocol Response |
|----------|-------------------|
| 10× collateral price increase | Health factor improves proportionally; position remains healthy |
| 10× collateral price crash | Health factor drops immediately; position becomes liquidatable when below threshold |
| Debt asset price spike | Health factor worsens (debt value grows); position may become liquidatable |

**Key invariant**: View functions (`get_health_factor`, `get_collateral_value`, `get_max_liquidatable_amount`) reflect price changes immediately with no lag. No state change is required from the user.

### Stale Feed Handling

| Scenario | Resolution Order | Result |
|----------|-----------------|--------|
| Primary feed fresh | Primary → (done) | Fresh primary price used |
| Primary stale, fallback fresh | Primary stale → Fallback → (done) | Fallback price used transparently |
| Primary missing, fallback fresh | No primary → Fallback → (done) | Fallback price used |
| Primary stale, fallback stale | Both checked → error | `StalePrice` error returned |
| No feed configured | No primary, no fallback | `NoPriceFeed` error returned |

**Staleness definition**: A price is stale if `current_timestamp - last_updated > max_staleness_seconds` (default 3600s). Future timestamps (`last_updated > current_timestamp`) are **also treated as stale** as a clock-skew manipulation guard.

### Behaviour When No Price Is Available

When the oracle module cannot provide a fresh price, **all values default to 0** rather than reverting:

- `get_collateral_value()` → `0`
- `get_debt_value()` → `0`
- `get_health_factor()` → `0` (when user has debt but no oracle; `HEALTH_FACTOR_NO_DEBT` when no debt)
- `get_max_liquidatable_amount()` → `0` (position treated as non-liquidatable under missing oracle)

This "fail-safe to zero" design prevents panics but means external monitors should treat `health_factor == 0` as an oracle outage signal rather than a healthy position.

### Unauthorised Price Writes (Cache Poisoning)

The oracle enforces three-tier slot isolation:

1. **Admin** can write to the primary slot for any asset.
2. **Registered primary oracle** can write to the primary slot for its registered asset only.
3. **Registered fallback oracle** can write to the fallback slot only — it **cannot overwrite the primary slot**.
4. **All other addresses** are rejected with `OracleError::Unauthorized`.

An attacker injecting a far-future timestamp via storage cannot extend feed freshness: future timestamps are immediately treated as stale by `is_stale()`.

Zero and negative prices are always rejected (`OracleError::InvalidPrice`) regardless of the caller's role.

### Health Factor Boundary

The liquidation threshold boundary is:

```
health_factor = (collateral_value × liquidation_threshold_bps / 10000) × 10000 / debt_value
```

| `health_factor` value | Meaning |
|-----------------------|---------|
| ≥ `HEALTH_FACTOR_SCALE` (10000) | Position is healthy; not liquidatable |
| < `HEALTH_FACTOR_SCALE` | Position is liquidatable |
| `HEALTH_FACTOR_NO_DEBT` (100_000_000) | No debt; trivially healthy |
| `0` | Oracle unavailable with active debt; cannot compute |

**Exact boundary**: A position with `health_factor == 10000` is **not** liquidatable (`get_max_liquidatable_amount` returns 0).

### Oracle Pause Mode

When `set_oracle_paused(admin, true)` is called:
- **New price updates are blocked** (`OracleError::OraclePaused`)
- **Existing prices remain readable** until they become stale under the normal staleness window
- Pause is intended as a short-term emergency circuit-breaker; prolonged pause causes all feeds to go stale and views to return 0

### Cross-Asset Independence

Oracle state for each asset is completely independent:

- Staleness of Asset A's feed has **no effect** on Asset B's price reads
- A stale collateral oracle causes `collateral_value → 0`; the debt value is still correctly computed from a fresh debt oracle (and vice versa)
- Price manipulation of one asset in a cross-asset position affects only the relevant value, not all positions

### Attack Resistance Summary

| Attack Vector | Mitigation |
|---------------|------------|
| Non-authorized price write | `require_auth()` + role check on every `update_price_feed` call |
| Fallback oracle overwrites primary | Slot routing: fallback oracle can only write to `FallbackFeed` key |
| Far-future timestamp injection | `is_stale()` treats `now < last_updated` as stale |
| Zero/negative price injection | `validate_price()` rejects `price ≤ 0` before any storage write |
| Price feed poisoning via protocol pause | Oracle pause blocks writes; existing prices expire naturally |
| Self-referential oracle registration | `set_primary_oracle` / `set_fallback_oracle` reject `oracle == contract_address` |

---

## Conclusion

Effective oracle configuration management is critical for the security and reliability of the StellarLend protocol. This guide provides the procedures and considerations necessary for maintaining a robust oracle system while ensuring proper role separation and security controls.

Regular review of configurations, continuous monitoring, and adherence to security best practices are essential for maintaining system integrity and protecting user assets.
