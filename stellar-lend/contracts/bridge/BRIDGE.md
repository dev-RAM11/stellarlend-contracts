# Bridge Registry Contract

A Soroban smart contract that manages cross-chain bridge configurations for the
StellarLend protocol. It acts as a **registry and accounting layer**: it records
deposit and withdrawal events and tracks cumulative totals, but does **not** hold
or transfer tokens directly. Actual cross-chain asset movement is performed by an
off-chain relayer that reads the events emitted by this contract.

---

## Table of Contents

- [Overview](#overview)
- [Roles and Trust Boundaries](#roles-and-trust-boundaries)
- [Storage Layout](#storage-layout)
- [Public API](#public-api)
- [Fee Calculation](#fee-calculation)
- [Network ID and Replay Protection](#network-id-and-replay-protection)
- [Canonical Withdrawal Message IDs](#canonical-withdrawal-message-ids)
- [Security Notes](#security-notes)
- [Error Reference](#error-reference)
- [Running Tests](#running-tests)

---

## Overview

```
┌──────────────┐    bridge_deposit    ┌──────────────────────┐
│  Any caller  │ ──────────────────▶  │                      │
└──────────────┘                      │   Bridge Registry    │
                                      │   (this contract)    │
┌──────────────┐   bridge_withdraw    │                      │
│   Relayer    │ ──────────────────▶  │  Accounting only –   │
└──────────────┘                      │  no token custody    │
                                      │                      │
┌──────────────┐  register / config   │                      │
│    Admin     │ ──────────────────▶  │                      │
└──────────────┘                      └──────────────────────┘
```

---

## Roles and Trust Boundaries

| Role     | How to identify                          | Permitted actions |
|----------|------------------------------------------|-------------------|
| **Admin**   | Address stored at `ADMIN` instance key  | `register_bridge`, `set_bridge_fee`, `set_bridge_active`, `set_relayer`, `transfer_admin`, all upgrade operations |
| **Relayer** | Address stored at `RELAYER` instance key (optional) | `bridge_withdraw` only |
| **Any caller** | Any authenticated Stellar address    | `bridge_deposit` |

The admin and relayer are **strictly separated**:
- The relayer can only record outbound withdrawals; it cannot modify bridge
  configuration, assign a new relayer, or transfer admin rights.
- When no relayer is configured, only the admin may call `bridge_withdraw`.

---

## Storage Layout

| Storage type | Key | Value |
|---|---|---|
| Instance | `ADMIN` (symbol) | `Address` — protocol admin |
| Instance | `RELAYER` (symbol) | `Address` — designated relayer (optional) |
| Instance | `DataKey::BridgeList` | `Vec<String>` — ordered list of bridge IDs |
| Persistent | `DataKey::Bridge(id)` | `BridgeConfig` — per-bridge configuration |

Storage keys are defined as a typed `DataKey` enum (`#[contracttype]`), which
Soroban serialises automatically. Future migrations that change `BridgeConfig`
fields should introduce a new versioned variant.

---

## Public API

### `init(admin: Address) → Result<(), ContractError>`

Initialise the registry. Must be called exactly once, in the same transaction as
deployment, to prevent front-running.

### `register_bridge(caller, bridge_id, network_id, fee_bps, min_amount) → Result<(), ContractError>`

Admin-only. Register a new bridge endpoint. The bridge starts active with zero
cumulative totals.

| Parameter | Type | Constraints |
|---|---|---|
| `bridge_id` | `String` | 1–64 bytes, `[a-zA-Z0-9_-]` only |
| `network_id` | `u32` | Remote chain identifier (e.g. EVM chain ID) |
| `fee_bps` | `u64` | 0 – 1 000 (0% – 10%) |
| `min_amount` | `i128` | ≥ 0 |

### `set_bridge_fee(caller, bridge_id, fee_bps) → Result<(), ContractError>`

Admin-only. Update the fee for an existing bridge. Takes effect immediately.

### `set_bridge_active(caller, bridge_id, active) → Result<(), ContractError>`

Admin-only. Pause (`false`) or resume (`true`) deposits on a bridge.
Withdrawals remain allowed while inactive so in-flight transfers can settle.

### `bridge_deposit(sender, bridge_id, amount) → Result<i128, ContractError>`

Any authenticated caller. Records an inbound deposit and returns the net amount
after fee deduction. The off-chain relayer uses the net amount to credit the
recipient on the destination chain.

### `bridge_withdraw(caller, bridge_id, message_id, recipient, amount) → Result<(), ContractError>`

Admin or relayer. Records an outbound withdrawal and emits a
`BridgeWithdrawalEvent` that the off-chain relayer uses to execute the actual
token transfer on the destination chain. Allowed even when the bridge is inactive.
Each withdrawal must provide a unique 32-byte `message_id`; reusing one is rejected.

### `set_relayer(caller, relayer) → Result<(), ContractError>`

Admin-only. Designate an address that may call `bridge_withdraw`. Overwrites the
previous relayer.

### `transfer_admin(caller, new_admin) → Result<(), ContractError>`

Admin-only. Transfer admin rights. The previous admin loses all privileges
immediately.

### `get_bridge_config(bridge_id) → Result<BridgeConfig, ContractError>`
### `list_bridges() → Vec<String>`
### `get_admin() → Result<Address, ContractError>`
### `get_relayer() → Option<Address>`
### `compute_fee(amount, fee_bps) → i128`

---

## Fee Calculation

```
fee = floor(amount × fee_bps / 10_000)
net = amount − fee
```

Intermediate arithmetic uses `I256` (256-bit integers) to prevent overflow even
at `i128::MAX`. The result is floored (rounds toward zero).

**Example:** 100 000 tokens at 50 bps → fee = 500, net = 99 500.

The maximum fee rate is **1 000 bps (10%)**, enforced at registration and update
time.

---

## Network ID and Replay Protection

Each `BridgeConfig` stores a `network_id: u32` that identifies the remote
blockchain this bridge connects to (e.g. Ethereum mainnet = `1`, BSC = `56`).

Every `BridgeDepositEvent` and `BridgeWithdrawalEvent` includes `network_id`.
Off-chain relayers **must** verify that the `network_id` in the event matches the
intended destination chain before executing any token transfer. This prevents a
withdrawal message intended for chain A from being replayed on chain B.

For withdrawals, the contract also stores every processed `message_id` under
`DataKey::ProcessedWithdrawal`. A second call with the same `message_id` fails
with `ContractError::MessageAlreadyProcessed`, which prevents duplicate
recording of the same bridge instruction even if the caller retries with the
same payload.

## Canonical Withdrawal Message IDs

`message_id` is treated on-chain as an opaque `BytesN<32>`, but operators should
derive it from immutable source-chain facts so uniqueness does not rely on
human coordination. A recommended format is:

```text
message_id = hash(
  source_chain_id ||
  source_tx_hash ||
  source_log_index ||
  bridge_id ||
  recipient ||
  amount
)
```

The exact hash function may follow the remote bridge stack, but the encoding
must be canonical and deterministic across all relayers. Two relayers observing
the same remote withdrawal intent should compute the same `message_id`.

Trust assumption: this contract enforces one-time use of a `message_id`, but it
cannot prove that a relayer chose the *correct* ID. Off-chain infrastructure
must therefore reject any withdrawal request whose `message_id` is not derived
from the canonical source event.

---

## Security Notes

### Authorization
- Every state-mutating function verifies caller identity via Soroban's
  `require_auth()` before reading or writing storage.
- `require_admin` compares the authenticated caller against the stored admin
  address; matching is strict (no role inheritance outside the relayer path).

### Reentrancy
Soroban contracts execute atomically within a single transaction. This contract
makes no cross-contract calls, making reentrancy structurally impossible.

### Arithmetic
- Fee calculation: `I256` prevents overflow in `amount × fee_bps`.
- Cumulative accounting: `checked_add` / `checked_sub`; overflows return
  `ContractError::Overflow` rather than wrapping silently.
- Release builds enable `overflow-checks = true` at the workspace level.

### Token custody
This contract does **not** hold or transfer tokens. It is a bookkeeping layer
only. Operators are responsible for ensuring that the off-chain bridge protocol
correctly matches on-chain event data before moving funds.

### Relayer trust model
- `bridge_deposit` remains user-authorized and is unique at the Stellar
  transaction level; relayers should still reconcile deposits against their
  escrow or lockbox flow before crediting another chain.
- `bridge_withdraw` is replay-resistant on-chain only when relayers submit the
  canonical `message_id` for the source withdrawal event.
- A malicious or buggy relayer cannot reuse an already-processed `message_id`,
  but could still create inconsistent requests if operators do not validate the
  source-chain event that produced the ID.

### Bridge ID validation
Bridge IDs are validated for length (1–64 bytes) and character set
(`[a-zA-Z0-9_-]`) before any storage write. This prevents storage-key injection
and ensures deterministic key serialisation.

---

## Error Reference

| Code | Name | Meaning |
|------|------|---------|
| 1 | `AlreadyInitialised` | `init` called more than once |
| 2 | `NotInitialised` | Contract not yet initialised |
| 3 | `Unauthorised` | Caller lacks required role |
| 4 | `BridgeAlreadyExists` | Duplicate bridge ID |
| 5 | `BridgeNotFound` | Unknown bridge ID |
| 6 | `BridgeInactive` | Deposit rejected; bridge is paused |
| 7 | `FeeTooHigh` | `fee_bps` > 1 000 |
| 8 | `InvalidBridgeIdLen` | ID is empty or > 64 bytes |
| 9 | `InvalidBridgeIdChar` | ID contains characters outside `[a-zA-Z0-9_-]` |
| 10 | `NegativeMinAmount` | `min_amount` < 0 |
| 11 | `AmountNotPositive` | Amount ≤ 0 |
| 12 | `AmountBelowMinimum` | Amount < bridge `min_amount` |
| 13 | `Overflow` | Accounting integer overflow |
| 14 | `MessageAlreadyProcessed` | Withdrawal `message_id` was already used |

---

## Running Tests

```bash
# Unit tests (fast, no WASM build)
cargo test -p bridge

# Full WASM build (verify exports and binary size)
stellar contract build --package bridge
```

Expected output: bridge unit tests complete with replay-resistance, fee-rounding,
authorization, and arithmetic coverage and no failures.
