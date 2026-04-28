use soroban_sdk::{contracttype, Address, Env};
use crate::borrow::BorrowError;

#[contracttype]
#[derive(Clone)]
pub enum RegistryKey {
    AssetRegistry(Address),
}

pub fn is_registered(env: &Env, asset: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&RegistryKey::AssetRegistry(asset.clone()))
        .unwrap_or(false)
}

pub fn require_registered_asset(env: &Env, asset: &Address) -> Result<(), BorrowError> {
    if !is_registered(env, asset) {
        return Err(BorrowError::AssetNotSupported);
    }
    Ok(())
}

pub fn register(env: &Env, asset: &Address) -> Result<(), BorrowError> {
    if is_registered(env, asset) {
        return Err(BorrowError::InvalidAmount);
    }
    env.storage()
        .persistent()
        .set(&RegistryKey::AssetRegistry(asset.clone()), &true);
    Ok(())
}

pub fn deregister(env: &Env, asset: &Address) -> Result<(), BorrowError> {
    env.storage()
        .persistent()
        .remove(&RegistryKey::AssetRegistry(asset.clone()));
    Ok(())
}
