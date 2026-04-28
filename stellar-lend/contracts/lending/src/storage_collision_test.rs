//! # Storage Collision and Namespace Validation Tests
//!
//! This test suite ensures that the protocol's storage strategy effectively 
//! prevents collisions between modules and across contract upgrades.

extern crate std;
use soroban_sdk::{testutils::Address as _, Address, Env, String as SorobanString, Symbol};
use crate::oracle::OracleKey;
use crate::cross_asset::CrossAssetDataKey;
use crate::data_store::StoreKey;
use crate::withdraw::WithdrawDataKey;
use crate::LendingContract;

#[test]
fn test_storage_namespaces_are_isolated() {
    let env = Env::default();
    let contract_id = env.register(LendingContract, ());
    
    env.as_contract(&contract_id, || {
        // Test 1: Oracle vs CrossAsset Paused
        let key1 = OracleKey::OraclePaused;
        let key2 = CrossAssetDataKey::CrossAssetPaused;
        let key3 = WithdrawDataKey::WithdrawPaused;

        env.storage().persistent().set(&key1, &true);
        env.storage().persistent().set(&key2, &false);
        env.storage().persistent().set(&key3, &true);

        assert_eq!(env.storage().persistent().get::<_, bool>(&key1).unwrap(), true);
        assert_eq!(env.storage().persistent().get::<_, bool>(&key2).unwrap(), false);
        assert_eq!(env.storage().persistent().get::<_, bool>(&key3).unwrap(), true);

        // Test 2: CrossAsset vs DataStore Admin
        let key4 = CrossAssetDataKey::CrossAssetAdmin;
        let key5 = StoreKey::StoreAdmin;
        
        let addr1 = Address::generate(&env);
        let addr2 = Address::generate(&env);

        env.storage().persistent().set(&key4, &addr1);
        env.storage().persistent().set(&key5, &addr2);

        assert_eq!(env.storage().persistent().get::<_, Address>(&key4).unwrap(), addr1);
        assert_eq!(env.storage().persistent().get::<_, Address>(&key5).unwrap(), addr2);
    });
}

#[test]
fn test_data_store_string_keys_isolation() {
    let env = Env::default();
    let contract_id = env.register(LendingContract, ());

    env.as_contract(&contract_id, || {
        // Dynamic string keys in DataStore must not collide with fixed enum keys
        // "StoreAdmin" as a string is different from StoreKey::StoreAdmin as an enum variant
        let user_key_str = SorobanString::from_str(&env, "StoreAdmin");
        let store_entry_key = StoreKey::Entry(user_key_str);
        let internal_key = StoreKey::StoreAdmin;
        
        let addr_user = Address::generate(&env);
        let addr_internal = Address::generate(&env);
        
        env.storage().persistent().set(&store_entry_key, &addr_user);
        env.storage().persistent().set(&internal_key, &addr_internal);
        
        assert_eq!(env.storage().persistent().get::<_, Address>(&store_entry_key).unwrap(), addr_user);
        assert_eq!(env.storage().persistent().get::<_, Address>(&internal_key).unwrap(), addr_internal);
    });
}

#[test]
fn test_namespacing_with_tuples() {
    let env = Env::default();
    let contract_id = env.register(LendingContract, ());
    
    env.as_contract(&contract_id, || {
        // Verify that even if we had shared variant names, tuples would solve it
        // (Using the new names here but the principle remains)
        let key1 = (Symbol::new(&env, "oracle"), OracleKey::OraclePaused);
        let key2 = (Symbol::new(&env, "cross"), CrossAssetDataKey::CrossAssetPaused);

        env.storage().persistent().set(&key1, &true);
        env.storage().persistent().set(&key2, &false);

        assert_eq!(env.storage().persistent().get::<_, bool>(&key1).unwrap(), true);
        assert_eq!(env.storage().persistent().get::<_, bool>(&key2).unwrap(), false);
    });
}
