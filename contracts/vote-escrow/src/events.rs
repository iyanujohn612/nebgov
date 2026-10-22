use soroban_sdk::{contracttype, Address, Env, Symbol};

pub const LOCK_CREATED_TOPIC: &str = "LockCreated";
pub const LOCK_INCREASED_TOPIC: &str = "LockIncreased";
pub const LOCK_EXTENDED_TOPIC: &str = "LockExtended";
pub const LOCK_WITHDRAWN_TOPIC: &str = "LockWithdrawn";
pub const ESCROW_CONFIG_UPDATED_TOPIC: &str = "EscrowConfigUpdated";

#[contracttype]
#[derive(Clone, Debug)]
pub struct LockCreatedEvent {
    pub owner: Address,
    pub amount: i128,
    pub end_ledger: u32,
    pub initial_voting_power: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LockIncreasedEvent {
    pub owner: Address,
    pub added_amount: i128,
    pub new_voting_power: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LockExtendedEvent {
    pub owner: Address,
    pub old_end_ledger: u32,
    pub new_end_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LockWithdrawnEvent {
    pub owner: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowConfigUpdatedEvent {
    pub old_min_lock_duration: u32,
    pub new_min_lock_duration: u32,
    pub old_max_lock_duration: u32,
    pub new_max_lock_duration: u32,
    pub old_max_multiplier_bps: u32,
    pub new_max_multiplier_bps: u32,
}

pub fn emit_lock_created(env: &Env, owner: &Address, amount: i128, end_ledger: u32, initial_voting_power: i128) {
    env.events().publish(
        (Symbol::new(env, LOCK_CREATED_TOPIC), owner.clone()),
        LockCreatedEvent {
            owner: owner.clone(),
            amount,
            end_ledger,
            initial_voting_power,
        },
    );
}

pub fn emit_lock_increased(env: &Env, owner: &Address, added_amount: i128, new_voting_power: i128) {
    env.events().publish(
        (Symbol::new(env, LOCK_INCREASED_TOPIC), owner.clone()),
        LockIncreasedEvent {
            owner: owner.clone(),
            added_amount,
            new_voting_power,
        },
    );
}

pub fn emit_lock_extended(env: &Env, owner: &Address, old_end_ledger: u32, new_end_ledger: u32) {
    env.events().publish(
        (Symbol::new(env, LOCK_EXTENDED_TOPIC), owner.clone()),
        LockExtendedEvent {
            owner: owner.clone(),
            old_end_ledger,
            new_end_ledger,
        },
    );
}

pub fn emit_lock_withdrawn(env: &Env, owner: &Address, amount: i128) {
    env.events().publish(
        (Symbol::new(env, LOCK_WITHDRAWN_TOPIC), owner.clone()),
        LockWithdrawnEvent {
            owner: owner.clone(),
            amount,
        },
    );
}

pub fn emit_escrow_config_updated(env: &Env, event: EscrowConfigUpdatedEvent) {
    env.events()
        .publish((Symbol::new(env, ESCROW_CONFIG_UPDATED_TOPIC),), event);
}
