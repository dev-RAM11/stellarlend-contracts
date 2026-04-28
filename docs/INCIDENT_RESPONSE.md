# Incident Response & Pause Mechanisms

The StellarLend protocol provides several layers of protection to handle security incidents, market volatility, or technical issues. These mechanisms allow the protocol administrators to halt or restrict operations to protect user funds.

## 1. Pause Mechanisms Overview

| Mechanism | Scope | Impact | Recommended Use Case |
|-----------|-------|--------|----------------------|
| **Per-Operation Pause** | Specific function (e.g., Deposit) | Only the specific operation is disabled. Others remain active. | Minor issues in specific modules, maintenance. |
| **Emergency Pause** | Global | ALL mutating operations are disabled. View functions remain available. | Major security breach, critical bug discovery. |
| **Read-Only Mode** | Global (Highest Precedence) | ALL state-changing operations (including admin config) are disabled. View functions remain available. | Investigation of complex incidents where even admin state changes might be risky. |

## 2. Read-Only Mode

Read-Only Mode is the most restrictive state of the protocol. When enabled, it ensures that no state transitions can occur within the contract, providing a "frozen" snapshot for investigation.

### Impact of Read-Only Mode
- **Mutating Operations Disabled:** `deposit`, `withdraw`, `borrow`, `repay`, `liquidate`, and `flash_loan` will all fail with a `ReadOnlyMode` error.
- **Admin Operations Disabled:** `set_risk_params`, `update_interest_rate_config`, and other configuration updates are blocked.
- **View Functions Available:** All `get_*` functions and analytics reporting remain fully functional.
- **Exceptions:** Only `set_read_only_mode` itself can be called by the admin to toggle the mode.

### Precedence Matrix
If multiple pause mechanisms are active simultaneously, the most restrictive one takes precedence:
1. **Read-Only Mode** (Overrides everything)
2. **Emergency Pause** (Overrides per-operation switches)
3. **Per-Operation Pause** (Lowest precedence)

## 3. Incident Response Guidance

### Minor Bug or Maintenance
If a bug is identified in a specific operation (e.g., a display error in deposits), use **Per-Operation Pause** for that specific function:
```sh
soroban contract invoke --id <ID> --fn set_pause_switch --arg caller=<ADMIN> --arg operation=pause_deposit --arg paused=true
```

### Suspected Security Breach
If a security breach is suspected but its extent is unknown, immediately trigger the **Emergency Pause**:
```sh
soroban contract invoke --id <ID> --fn set_emergency_pause --arg caller=<ADMIN> --arg paused=true
```

### Critical Incident / Forensic Investigation
If a critical exploit has occurred or the protocol state must be preserved exactly for forensic analysis, enable **Read-Only Mode**:
```sh
soroban contract invoke --id <ID> --fn set_read_only_mode --arg caller=<ADMIN> --arg enabled=true
```

## 4. Security Notes & Limitations

- **Authorization:** Only the designated `Admin` address can toggle these switches.
- **Persistence:** All pause states are stored in persistent storage and remain active across ledger updates until explicitly disabled.
- **View-Only Guarantee:** While state-changing operations are blocked, view functions continue to read from current storage. Note that if interest accrual is triggered by a view function (if any), it will not be persisted in read-only mode.
- **Off-Chain Indexers:** Indexers should monitor for `PauseStateChanged` and `ReadOnlyMode` events to update their UI/state accordingly.
