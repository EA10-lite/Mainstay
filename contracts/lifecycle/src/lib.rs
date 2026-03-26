#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, contracterror, panic_with_error, symbol_short, Address, Env, String, Symbol, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ContractError {
    NoMaintenanceHistory  = 1,
    UnauthorizedEngineer  = 2,
}

#[contracttype]
#[derive(Clone)]
pub struct MaintenanceRecord {
    pub asset_id: u64,
    pub task_type: Symbol,
    pub notes: String,
    pub engineer: Address,
    pub timestamp: u64,
}

const ENG_REGISTRY: Symbol = symbol_short!("ENG_REG");

fn history_key(asset_id: u64) -> (Symbol, u64) {
    (symbol_short!("HIST"), asset_id)
}

fn score_key(asset_id: u64) -> (Symbol, u64) {
    (symbol_short!("SCORE"), asset_id)
}

fn registry_key() -> Symbol {
    symbol_short!("REGISTRY")
}

fn max_history_key() -> Symbol {
    symbol_short!("MAX_HIST")
}

const DEFAULT_MAX_HISTORY: u32 = 200;

// Minimal client interface for cross-contract call to EngineerRegistry
mod engineer_registry {
    use soroban_sdk::{contractclient, Address, Env};
    #[contractclient(name = "EngineerRegistryClient")]
    pub trait EngineerRegistry {
        fn verify_engineer(env: Env, engineer: Address) -> bool;
    }
}

#[contract]
pub struct Lifecycle;

#[contractimpl]
impl Lifecycle {
    /// Must be called once after deployment to set the asset-registry contract address.
    /// Optionally set a max_history cap (pass 0 to use the default of 200).
    pub fn initialize(env: Env, asset_registry: Address, max_history: u32) {
        env.storage().instance().set(&registry_key(), &asset_registry);
        let cap = if max_history == 0 { DEFAULT_MAX_HISTORY } else { max_history };
        env.storage().instance().set(&max_history_key(), &cap);
    /// Must be called once after deployment to bind the engineer registry.
    pub fn initialize(env: Env, engineer_registry: Address) {
        env.storage().instance().set(&ENG_REGISTRY, &engineer_registry);
    }

    pub fn submit_maintenance(
        env: Env,
        asset_id: u64,
        task_type: Symbol,
        notes: String,
        engineer: Address,
    ) {
        engineer.require_auth();

        // Validate asset exists in the registry (panics with "asset not found" if not)
        let registry: Address = env
            .storage()
            .instance()
            .get(&registry_key())
            .expect("registry not set");
        let registry_client = asset_registry::AssetRegistryClient::new(&env, &registry);
        registry_client.get_asset(&asset_id);

        let mut history: Vec<MaintenanceRecord> = env
            .storage()
            .persistent()
            .get(&history_key(asset_id))
            .unwrap_or(Vec::new(&env));

        let cap: u32 = env
            .storage()
            .instance()
            .get(&max_history_key())
            .unwrap_or(DEFAULT_MAX_HISTORY);

        if history.len() >= cap {
            panic!("history cap reached");
        // Cross-check engineer credential
        let registry_id: Address = env.storage().instance().get(&ENG_REGISTRY)
            .unwrap_or_else(|| panic_with_error!(&env, ContractError::UnauthorizedEngineer));
        let registry = engineer_registry::EngineerRegistryClient::new(&env, &registry_id);
        if !registry.verify_engineer(&engineer) {
            panic_with_error!(&env, ContractError::UnauthorizedEngineer);
        }

        let record = MaintenanceRecord {
            asset_id,
            task_type,
            notes,
            engineer,
            timestamp: env.ledger().timestamp(),
        };

        history.push_back(record);
        env.storage().persistent().set(&history_key(asset_id), &history);
        
        // Extend TTL for persistent storage entries to prevent data loss
        env.storage().persistent().extend_ttl(&history_key(asset_id), 518400, 518400); // 30 days

        let score: u32 = env
            .storage()
            .persistent()
            .get(&score_key(asset_id))
            .unwrap_or(0u32);
        let new_score = (score + 5).min(100);
        env.storage().persistent().set(&score_key(asset_id), &new_score);
        
        // Extend TTL for persistent storage entries to prevent data loss
        env.storage().persistent().extend_ttl(&score_key(asset_id), 518400, 518400); // 30 days
        
        // Emit maintenance submission event
        env.events().publish(
            (symbol_short!("MAINT"), asset_id),
            (task_type, engineer.clone(), env.ledger().timestamp())
        );
    }

    pub fn get_maintenance_history(env: Env, asset_id: u64) -> Vec<MaintenanceRecord> {
        env.storage()
            .persistent()
            .get(&history_key(asset_id))
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_last_service(env: Env, asset_id: u64) -> MaintenanceRecord {
        let history: Vec<MaintenanceRecord> = env
            .storage()
            .persistent()
            .get(&history_key(asset_id))
            .unwrap_or_else(|| panic_with_error!(&env, ContractError::NoMaintenanceHistory));
        history.last().unwrap_or_else(|| panic_with_error!(&env, ContractError::NoMaintenanceHistory))
    }

    pub fn get_collateral_score(env: Env, asset_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&score_key(asset_id))
            .unwrap_or(0)
    }

    pub fn is_collateral_eligible(env: Env, asset_id: u64) -> bool {
        let threshold = env
            .storage()
            .instance()
            .get(&CONFIG)
            .map(|c: Config| c.collateral_threshold)
            .unwrap_or(50);
        Self::get_collateral_score(env, asset_id) >= threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_registry::{AssetRegistry, AssetRegistryClient};
    use soroban_sdk::{symbol_short, testutils::Address as _, Env, String};
    use soroban_sdk::{symbol_short, testutils::Address as _, BytesN, Env, String};
    use crate::engineer_registry::EngineerRegistryClient;
    use engineer_registry_contract::EngineerRegistry;

    mod engineer_registry_contract {
        soroban_sdk::contractimport!(
            file = "../../target/wasm32-unknown-unknown/release/engineer_registry.wasm"
        );
    }

    fn setup(env: &Env) -> (LifecycleClient, EngineerRegistryClient) {
        let eng_reg_id = env.register(EngineerRegistry, ());
        let lifecycle_id = env.register(Lifecycle, ());
        let lifecycle = LifecycleClient::new(env, &lifecycle_id);
        lifecycle.initialize(&eng_reg_id);
        (lifecycle, EngineerRegistryClient::new(env, &eng_reg_id))
    }

    fn setup(env: &Env, max_history: u32) -> (LifecycleClient<'_>, AssetRegistryClient<'_>) {
    fn setup(env: &Env) -> (LifecycleClient<'_>, AssetRegistryClient<'_>) {
        let registry_id = env.register(AssetRegistry, ());
        let registry_client = AssetRegistryClient::new(env, &registry_id);

        let lifecycle_id = env.register(Lifecycle, ());
        let client = LifecycleClient::new(env, &lifecycle_id);
        client.initialize(&registry_id, &max_history);
        client.initialize(&registry_id);

        (client, registry_client)
    }

    fn register_asset<'a>(env: &Env, registry_client: &AssetRegistryClient<'a>) -> u64 {
        let owner = Address::generate(env);
        registry_client.register_asset(
            &symbol_short!("GENSET"),
            &String::from_str(env, "Caterpillar 3516"),
            &owner,
        )
    }

    #[test]
    fn test_submit_and_score() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, registry_client) = setup(&env, 0);
        let asset_id = register_asset(&env, &registry_client);

        let engineer = Address::generate(&env);
        let (client, eng_client) = setup(&env);

    #[test]
    fn test_submit_and_score() {
        let (env, _, client) = setup();
        let engineer = Address::generate(&env);
        let issuer = Address::generate(&env);
        let hash = BytesN::from_array(&env, &[1u8; 32]);
        eng_client.register_engineer(&engineer, &hash, &issuer);

        let engineer = Address::generate(&env);
        for _ in 0..10 {
            client.submit_maintenance(
                &asset_id,
                &symbol_short!("OIL_CHG"),
                &String::from_str(&env, "Routine oil change"),
                &engineer,
            );
        }

        assert_eq!(client.get_collateral_score(&asset_id), 50);
        assert!(client.is_collateral_eligible(&asset_id));
        assert_eq!(client.get_maintenance_history(&asset_id).len(), 10);
    }

    #[test]
    #[should_panic(expected = "asset not found")]
    fn test_submit_maintenance_nonexistent_asset() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup(&env, 0);

        let engineer = Address::generate(&env);
        let (client, _) = setup(&env);

        let engineer = Address::generate(&env);
        // asset_id 999 was never registered — must panic
        client.submit_maintenance(
            &999u64,
            &symbol_short!("OIL_CHG"),
            &String::from_str(&env, "Should fail"),
            &engineer,
        );
    }

    #[test]
    #[should_panic(expected = "history cap reached")]
    fn test_history_cap_enforced() {
        let env = Env::default();
        env.mock_all_auths();
        // Set a small cap of 3 for this test
        let (client, registry_client) = setup(&env, 3);
        let asset_id = register_asset(&env, &registry_client);

        let engineer = Address::generate(&env);
        // Fill to cap
        for _ in 0..3 {
            client.submit_maintenance(
                &asset_id,
                &symbol_short!("OIL_CHG"),
                &String::from_str(&env, "ok"),
                &engineer,
            );
        }
        // This 4th submission must panic
        client.submit_maintenance(
            &asset_id,
            &symbol_short!("OIL_CHG"),
            &String::from_str(&env, "over cap"),
            &engineer,
        );
    }

    #[test]
    fn test_unregistered_engineer_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup(&env);

        let unregistered = Address::generate(&env);
        let result = client.try_submit_maintenance(
            &1u64,
            &symbol_short!("OIL_CHG"),
            &String::from_str(&env, "Should fail"),
            &unregistered,
        );
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                ContractError::UnauthorizedEngineer as u32
            )))
        );
    }

    #[test]
    fn test_get_last_service_no_history() {
        let env = Env::default();
        let contract_id = env.register(Lifecycle, ());
        let client = LifecycleClient::new(&env, &contract_id);
        let result = client.try_get_last_service(&999u64);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                ContractError::NoMaintenanceHistory as u32
            )))
        );
    }

    #[test]
    fn test_submit_maintenance_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, eng_client) = setup(&env);
        
        let engineer = Address::generate(&env);
        let issuer = Address::generate(&env);
        let hash = BytesN::from_array(&env, &[1u8; 32]);
        eng_client.register_engineer(&engineer, &hash, &issuer);

        client.submit_maintenance(
            &1u64,
            &symbol_short!("OIL_CHG"),
            &String::from_str(&env, "Routine maintenance"),
            &engineer,
        );

        // Verify maintenance event was emitted
        let events = env.events().all();
        assert!(events.len() > 0);
    }

    #[test]
    fn test_ttl_extended_on_maintenance_submission() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, eng_client) = setup(&env);
        
        let engineer = Address::generate(&env);
        let issuer = Address::generate(&env);
        let hash = BytesN::from_array(&env, &[1u8; 32]);
        eng_client.register_engineer(&engineer, &hash, &issuer);

        let asset_id = 1u64;
        client.submit_maintenance(
            &asset_id,
            &symbol_short!("OIL_CHG"),
            &String::from_str(&env, "Routine maintenance"),
            &engineer,
        );

        // Verify TTL is set for history storage entry
        let history_ttl = env.storage().persistent().get_ttl(&history_key(asset_id));
        assert!(history_ttl > 0, "History TTL should be extended");

        // Verify TTL is set for score storage entry
        let score_ttl = env.storage().persistent().get_ttl(&score_key(asset_id));
        assert!(score_ttl > 0, "Score TTL should be extended");
    }
}
