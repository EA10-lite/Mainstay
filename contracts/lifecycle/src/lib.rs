#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, String, Symbol, Vec};

#[contracttype]
#[derive(Clone)]
pub struct MaintenanceRecord {
    pub asset_id: u64,
    pub task_type: Symbol,
    pub notes: String,
    pub engineer: Address,
    pub timestamp: u64,
}

fn history_key(asset_id: u64) -> (Symbol, u64) {
    (symbol_short!("HIST"), asset_id)
}

fn score_key(asset_id: u64) -> (Symbol, u64) {
    (symbol_short!("SCORE"), asset_id)
}

#[contract]
pub struct Lifecycle;

#[contractimpl]
impl Lifecycle {
    pub fn submit_maintenance(
        env: Env,
        asset_id: u64,
        task_type: Symbol,
        notes: String,
        engineer: Address,
    ) {
        engineer.require_auth();
        let record = MaintenanceRecord {
            asset_id,
            task_type,
            notes,
            engineer,
            timestamp: env.ledger().timestamp(),
        };

        let mut history: Vec<MaintenanceRecord> = env
            .storage()
            .persistent()
            .get(&amp;history_key(asset_id))
            .unwrap_or(Vec::new(&amp;env));
        history.push_back(record);
        env.storage().persistent().set(&amp;history_key(asset_id), &amp;history);

        // increment score (capped at 100)
        let score: u32 = env
            .storage()
            .persistent()
            .get(&amp;score_key(asset_id))
            .unwrap_or(0u32);
        let new_score = (score + 5).min(100);
        env.storage().persistent().set(&amp;score_key(asset_id), &amp;new_score);
    }

    pub fn get_maintenance_history(env: Env, asset_id: u64, page: u32, page_size: u32) -> Vec<MaintenanceRecord> {
        let page_size = page_size.min(1000u32);
        let history: Vec<MaintenanceRecord> = env.storage()
            .persistent()
            .get(&amp;history_key(asset_id))
            .unwrap_or(Vec::new(&amp;env));
        let len = history.len();
        let start = (page as usize).saturating_mul(page_size as usize);
        if start >= len {
            return Vec::new(&amp;env);
        }
        let end = (start + page_size as usize).min(len);
        history.slice(start..end)
    }

    pub fn get_last_service(env: Env, asset_id: u64) -> MaintenanceRecord {
        let history: Vec<MaintenanceRecord> = env
            .storage()
            .persistent()
            .get(&amp;history_key(asset_id))
            .expect("no maintenance history");
        history.last().expect("no records")
    }

    pub fn get_collateral_score(env: Env, asset_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&amp;score_key(asset_id))
            .unwrap_or(0)
    }

    pub fn is_collateral_eligible(env: Env, asset_id: u64) -> bool {
        Self::get_collateral_score(env, asset_id) >= 50
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{symbol_short, testutils::Address as _, Env, String};

    #[test]
    fn test_submit_and_score() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Lifecycle, ());
        let client = LifecycleClient::new(&amp;env, &amp;contract_id);

        let engineer = Address::generate(&amp;env);

        for _ in 0..10 {
            client.submit_maintenance(
                &amp;1u64,
                &amp;symbol_short!("OIL_CHG"),
                &amp;String::from_str(&amp;env, "Routine oil change"),
                &amp;engineer,
            );
        }

        assert_eq!(client.get_collateral_score(&amp;1u64), 50);
        assert!(client.is_collateral_eligible(&amp;1u64));
        assert_eq!(client.get_maintenance_history(&amp;1u64, &amp;0u32, &amp;10u32).len(), 10);
    }

    #[test]
    fn test_paginated_history() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Lifecycle, ());
        let client = LifecycleClient::new(&amp;env, &amp;contract_id);

        let engineer = Address::generate(&amp;env);
        let asset_id = 1u64;

        // Submit 25 records
        for _ in 0..25 {
            client.submit_maintenance(
                &amp;asset_id,
                &amp;symbol_short!("OIL_CHG"),
                &amp;String::from_str(&amp;env, "Oil change"),
                &amp;engineer,
            );
        }

        // Test page 0, size 10
        let page0 = client.get_maintenance_history(&amp;asset_id, &amp;0u32, &amp;10u32);
        assert_eq!(page0.len(), 10);

        // Test page 1, size 10
        let page1 = client.get_maintenance_history(&amp;asset_id, &amp;1u32, &amp;10u32);
        assert_eq!(page1.len(), 10);

        // Test page 2, size 10 (last 5)
        let page2 = client.get_maintenance_history(&amp;asset_id, &amp;2u32, &amp;10u32);
        assert_eq!(page2.len(), 5);

        // Test beyond end
        let page3 = client.get_maintenance_history(&amp;asset_id, &amp;3u32, &amp;10u32);
        assert_eq!(page3.len(), 0);

        // Test page_size 0
        let empty = client.get_maintenance_history(&amp;asset_id, &amp;0u32, &amp;0u32);
        assert_eq!(empty.len(), 0);
    }
}
