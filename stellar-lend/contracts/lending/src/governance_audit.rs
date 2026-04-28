//! # Governance Audit Log Module
//!
//! Provides a standardized audit log for all governance and admin actions
//! including oracle updates, pause toggles, risk parameter changes, caps,
//! and upgrade proposals/executions.
//!
//! ## Features
//! - Stable event schema for all governance actions
//! - Bounded storage for recent actions (gas-efficient querying)
//! - Comprehensive action types with extensible payload structure
//! - Security-focused design for incident response and compliance
//!
//! ## Security
//! - All audit entries are immutable once written
//! - Actions are logged atomically with the governance operation
//! - No sensitive data is stored in audit logs (only addresses and enum values)

use soroban_sdk::{contractevent, contracttype, Address, Env, Vec};

// ─────────────────────────────────────────────────────────────────────────────
// Governance Action Types
// ─────────────────────────────────────────────────────────────────────────────

/// Types of governance actions that can be audited.
///
/// This enum is designed to be stable and extensible. New variants can be added
/// without breaking existing audit log consumers.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceAction {
    /// Protocol initialization
    Initialize = 0,
    /// Admin address change
    SetAdmin = 1,
    /// Pause state change for specific operation
    SetPause = 2,
    /// Guardian address configuration
    SetGuardian = 3,
    /// Emergency shutdown trigger
    EmergencyShutdown = 4,
    /// Recovery mode start
    StartRecovery = 5,
    /// Recovery completion
    CompleteRecovery = 6,
    /// Oracle address configuration
    SetOracle = 7,
    /// Oracle parameters configuration
    ConfigureOracle = 8,
    /// Primary oracle address set for asset
    SetPrimaryOracle = 9,
    /// Fallback oracle address set for asset
    SetFallbackOracle = 10,
    /// Oracle pause state change
    SetOraclePaused = 11,
    /// Price feed update
    UpdatePriceFeed = 12,
    /// Liquidation threshold parameter change
    SetLiquidationThreshold = 13,
    /// Close factor parameter change
    SetCloseFactor = 14,
    /// Liquidation incentive parameter change
    SetLiquidationIncentive = 15,
    /// Borrow settings initialization
    InitializeBorrowSettings = 16,
    /// Deposit settings initialization
    InitializeDepositSettings = 17,
    /// Withdraw settings initialization
    InitializeWithdrawSettings = 18,
    /// Flash loan fee change
    SetFlashLoanFee = 19,
    /// Cross-asset admin initialization
    InitializeCrossAssetAdmin = 20,
    /// Asset parameters configuration
    SetAssetParams = 21,
    /// Upgrade system initialization
    UpgradeInit = 22,
    /// Upgrade approver addition
    UpgradeAddApprover = 23,
    /// Upgrade approver removal
    UpgradeRemoveApprover = 24,
    /// Upgrade proposal
    UpgradePropose = 25,
    /// Upgrade approval
    UpgradeApprove = 26,
    /// Upgrade execution
    UpgradeExecute = 27,
    /// Upgrade rollback
    UpgradeRollback = 28,
    /// Insurance fund credit
    CreditInsuranceFund = 29,
    /// Bad debt offset
    OffsetBadDebt = 30,
    /// Data store writer grant
    GrantDataWriter = 31,
    /// Data store writer revoke
    RevokeDataWriter = 32,
    /// Data backup
    DataBackup = 33,
    /// Data restore
    DataRestore = 34,
    /// Data migration
    DataMigrate = 35,
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload Types
// ─────────────────────────────────────────────────────────────────────────────

/// Payload data for governance actions.
///
/// Uses a flexible Vec<Val> approach to accommodate different action types
/// while maintaining a stable event schema. Each action type has a defined
/// payload structure that consumers should follow.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GovernancePayload {
    /// Action-specific data (addresses, amounts, parameters, etc.)
    pub data: Vec<soroban_sdk::Val>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Storage Types
// ─────────────────────────────────────────────────────────────────────────────

/// Storage keys for audit log data.
#[contracttype]
#[derive(Clone)]
pub enum AuditLogKey {
    /// Total count of audit entries
    Count,
    /// Audit entry at specific index (0-based)
    Entry(u64),
}

/// Audit log entry stored in contract storage.
///
/// Designed to be gas-efficient while providing comprehensive audit information.
/// Entries are stored in a bounded circular buffer to control gas costs.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AuditEntry {
    /// Sequential ID of the audit entry
    pub id: u64,
    /// Type of governance action
    pub action: GovernanceAction,
    /// Address that performed the action
    pub caller: Address,
    /// Block timestamp when action occurred
    pub timestamp: u64,
    /// Action-specific payload data
    pub payload: GovernancePayload,
}

// ─────────────────────────────────────────────────────────────────────────────
// Events
// ─────────────────────────────────────────────────────────────────────────────

/// Event emitted for every governance action.
///
/// This is the primary event that off-chain monitors should subscribe to
/// for real-time governance tracking and compliance monitoring.
#[contractevent]
#[derive(Clone, Debug)]
pub struct GovernanceAuditEvent {
    /// Sequential ID of the audit entry
    pub id: u64,
    /// Type of governance action
    pub action: GovernanceAction,
    /// Address that performed the action
    pub caller: Address,
    /// Block timestamp when action occurred
    pub timestamp: u64,
    /// Action-specific payload data
    pub payload: GovernancePayload,
}

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum number of audit entries to store in contract storage.
///
/// This bounds the gas cost of querying recent actions while maintaining
/// sufficient history for compliance and incident response.
/// 1000 entries should cover several months of typical governance activity.
pub const MAX_AUDIT_ENTRIES: u64 = 1000;

// ─────────────────────────────────────────────────────────────────────────────
// Public Interface
// ─────────────────────────────────────────────────────────────────────────────

/// Log a governance action to the audit trail.
///
/// This function should be called atomically with every governance action
/// to ensure complete audit coverage. The entry is stored in a circular
/// buffer and an event is emitted for off-chain monitoring.
///
/// # Arguments
/// * `env` - The contract environment
/// * `action` - Type of governance action
/// * `caller` - Address performing the action
/// * `payload` - Action-specific data
///
/// # Security
/// This function is read-only with respect to authorization - it should
/// only be called after the caller has been properly authorized for the
/// specific governance action being audited.
pub fn log_governance_action(
    env: &Env,
    action: GovernanceAction,
    caller: Address,
    payload: GovernancePayload,
) {
    let count_key = AuditLogKey::Count;
    let mut count: u64 = env.storage().persistent().get(&count_key).unwrap_or(0);
    count += 1;

    // Create audit entry
    let entry = AuditEntry {
        id: count,
        action,
        caller: caller.clone(),
        timestamp: env.ledger().timestamp(),
        payload: payload.clone(),
    };

    // Store in circular buffer
    let storage_index = count % MAX_AUDIT_ENTRIES;
    env.storage()
        .persistent()
        .set(&AuditLogKey::Entry(storage_index), &entry);

    // Update count
    env.storage().persistent().set(&count_key, &count);

    // Emit event for off-chain monitoring
    let event = GovernanceAuditEvent {
        id: count,
        action,
        caller,
        timestamp: entry.timestamp,
        payload,
    };
    event.publish(env);
}

/// Get recent governance audit entries.
///
/// Returns up to `limit` most recent audit entries in reverse chronological
/// order (newest first). The function is bounded to ensure predictable gas
/// costs regardless of total audit history.
///
/// # Arguments
/// * `env` - The contract environment
/// * `limit` - Maximum number of entries to return (must be <= 100)
///
/// # Returns
/// Vector of audit entries ordered from newest to oldest
///
/// # Security
/// This function is read-only and requires no authorization. It only returns
/// non-sensitive audit information (addresses, enum values, timestamps).
pub fn get_recent_audit_entries(env: &Env, limit: u32) -> Vec<AuditEntry> {
    if limit == 0 || limit > 100 {
        // Enforce reasonable limits to prevent gas issues
        return Vec::new(env);
    }

    let count_key = AuditLogKey::Count;
    let count: u64 = env.storage().persistent().get(&count_key).unwrap_or(0);
    
    if count == 0 {
        return Vec::new(env);
    }

    let mut entries = Vec::new(env);
    let limit_u64 = limit as u64;
    let start_index = if count > limit_u64 { count - limit_u64 } else { 0 };

    for i in start_index..count {
        let storage_index = i % MAX_AUDIT_ENTRIES;
        if let Some(entry) = env.storage().persistent().get(&AuditLogKey::Entry(storage_index)) {
            if entry.id == i + 1 { // Verify we have the correct entry
                entries.push_back(entry);
            }
        }
    }

    // Reverse to get newest first
    entries.reverse();
    entries
}

/// Get the total count of audit entries.
///
/// Returns the total number of governance actions that have been logged
/// since contract deployment. This can be used for pagination.
pub fn get_audit_count(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&AuditLogKey::Count)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper Functions for Payload Construction
// ─────────────────────────────────────────────────────────────────────────────

/// Create a payload with a single address.
pub fn payload_address(env: &Env, address: Address) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with an address and boolean.
pub fn payload_address_bool(env: &Env, address: Address, value: bool) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into_val(env));
    data.push_back(value.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with an address and u64 value.
pub fn payload_address_u64(env: &Env, address: Address, value: u64) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into_val(env));
    data.push_back(value.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with an address and i128 value.
pub fn payload_address_i128(env: &Env, address: Address, value: i128) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into_val(env));
    data.push_back(value.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with two addresses.
pub fn payload_two_addresses(env: &Env, addr1: Address, addr2: Address) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(addr1.into_val(env));
    data.push_back(addr2.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with an address, asset, and i128 value.
pub fn payload_address_asset_i128(
    env: &Env,
    address: Address,
    asset: Address,
    value: i128,
) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(address.into_val(env));
    data.push_back(asset.into_val(env));
    data.push_back(value.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with just an i128 value.
pub fn payload_i128(env: &Env, value: i128) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with a u64 value.
pub fn payload_u64(env: &Env, value: u64) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with two u64 values.
pub fn payload_two_u64(env: &Env, value1: u64, value2: u64) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value1.into_val(env));
    data.push_back(value2.into_val(env));
    GovernancePayload { data }
}

/// Create a payload with a string value.
pub fn payload_string(env: &Env, value: soroban_sdk::String) -> GovernancePayload {
    let mut data = Vec::new(env);
    data.push_back(value.into_val(env));
    GovernancePayload { data }
}

/// Create an empty payload for actions that don't need additional data.
pub fn payload_empty(env: &Env) -> GovernancePayload {
    GovernancePayload {
        data: Vec::new(env),
    }
}
