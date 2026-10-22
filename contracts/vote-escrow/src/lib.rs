#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Vec};

mod error;
mod events;

pub use error::VoteEscrowError;
pub use events::*;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Lock {
    pub owner: Address,
    pub amount: i128,
    pub start_ledger: u32,
    pub end_ledger: u32,
    pub initial_voting_power: i128,
    pub withdrawn: bool,
}

#[contracttype]
pub enum DataKey {
    Admin,
    LockedToken,
    MinLockDuration,
    MaxLockDuration,
    MaxMultiplierBps,
    Lock(Address),
    /// Monotonic count of historical lock records ever appended for this owner.
    /// The number of *retained* records is `min(count, MAX_LOCK_HISTORY_ENTRIES)`.
    LockHistoryCount(Address),
    /// One slot of the per-owner history ring buffer. Slot `n % MAX_LOCK_HISTORY_ENTRIES`
    /// holds the `n`-th appended (withdrawn) lock record.
    LockHistoryEntry(Address, u32),
    TotalLocked,
    GlobalCheckpoints,
}

/// Maximum number of withdrawn lock records retained per owner.
///
/// `withdraw` appends to a fixed-size ring buffer instead of reading, pushing
/// and rewriting an unbounded `Vec<Lock>`, so storage per owner and work per
/// withdrawal are both bounded. Older records are evicted once the buffer is
/// full; `LockHistoryCount` still tracks the monotonic append position.
pub const MAX_LOCK_HISTORY_ENTRIES: u32 = 32;

#[contract]
pub struct VoteEscrowContract;

#[contractimpl]
impl VoteEscrowContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        locked_token: Address,
        min_lock_duration: u32,
        max_lock_duration: u32,
        max_multiplier_bps: u32,
    ) {
        admin.require_auth();

        if env.storage().instance().has(&DataKey::Admin) {
            env.panic_with_error(VoteEscrowError::NotInitialized);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::LockedToken, &locked_token);
        env.storage()
            .instance()
            .set(&DataKey::MinLockDuration, &min_lock_duration);
        env.storage()
            .instance()
            .set(&DataKey::MaxLockDuration, &max_lock_duration);
        env.storage()
            .instance()
            .set(&DataKey::MaxMultiplierBps, &max_multiplier_bps);
        env.storage()
            .instance()
            .set(&DataKey::TotalLocked, &0i128);

        let checkpoints: Vec<(u32, i128)> = Vec::new(&env);
        env.storage()
            .instance()
            .set(&DataKey::GlobalCheckpoints, &checkpoints);
    }

    pub fn create_lock(env: Env, owner: Address, amount: i128, duration_ledgers: u32) -> Lock {
        owner.require_auth();

        let min_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinLockDuration)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();
        let max_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxLockDuration)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();

        if duration_ledgers < min_duration || duration_ledgers > max_duration {
            env.panic_with_error(VoteEscrowError::InvalidDuration);
        }

        if amount <= 0 {
            env.panic_with_error(VoteEscrowError::InvalidAmount);
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::Lock(owner.clone()))
        {
            let existing_lock: Lock = env
                .storage()
                .persistent()
                .get(&DataKey::Lock(owner.clone()))
                .unwrap();
            if !existing_lock.withdrawn {
                env.panic_with_error(VoteEscrowError::LockNotFound);
            }
        }

        let locked_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::LockedToken)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();

        let current_ledger = env.ledger().sequence();
        let end_ledger = current_ledger.checked_add(duration_ledgers)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap();

        let max_multiplier_bps: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxMultiplierBps)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();

        let initial_voting_power =
            Self::compute_initial_voting_power(&env, amount, duration_ledgers, min_duration, max_duration, max_multiplier_bps);

        let lock = Lock {
            owner: owner.clone(),
            amount,
            start_ledger: current_ledger,
            end_ledger,
            initial_voting_power,
            withdrawn: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Lock(owner.clone()), &lock);
        Self::refresh_persistent_ttl(&env, &DataKey::Lock(owner.clone()));

        token::Client::new(&env, &locked_token).transfer(&owner, &env.current_contract_address(), &amount);

        let current_total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        let new_total = current_total.checked_add(amount)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap();
        env.storage()
            .instance()
            .set(&DataKey::TotalLocked, &new_total);

        Self::update_global_checkpoint(&env, current_ledger);

        emit_lock_created(&env, &owner, amount, end_ledger, initial_voting_power);

        lock
    }

    pub fn increase_lock_amount(env: Env, owner: Address, additional_amount: i128) -> Lock {
        owner.require_auth();

        if additional_amount <= 0 {
            env.panic_with_error(VoteEscrowError::InvalidAmount);
        }

        let mut lock: Lock = env
            .storage()
            .persistent()
            .get(&DataKey::Lock(owner.clone()))
            .ok_or(VoteEscrowError::LockNotFound)
            .unwrap();

        if lock.withdrawn {
            env.panic_with_error(VoteEscrowError::LockAlreadyWithdrawn);
        }

        let locked_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::LockedToken)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();

        token::Client::new(&env, &locked_token).transfer(&owner, &env.current_contract_address(), &additional_amount);

        let new_amount = lock.amount.checked_add(additional_amount)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap();

        let min_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinLockDuration)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();
        let max_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxLockDuration)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();
        let max_multiplier_bps: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxMultiplierBps)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();

        let remaining_duration = lock.end_ledger.saturating_sub(lock.start_ledger);
        let new_initial_voting_power =
            Self::compute_initial_voting_power(&env, new_amount, remaining_duration, min_duration, max_duration, max_multiplier_bps);

        lock.amount = new_amount;
        lock.initial_voting_power = new_initial_voting_power;

        env.storage()
            .persistent()
            .set(&DataKey::Lock(owner.clone()), &lock);
        Self::refresh_persistent_ttl(&env, &DataKey::Lock(owner.clone()));

        let current_total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        let new_total = current_total.checked_add(additional_amount)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap();
        env.storage()
            .instance()
            .set(&DataKey::TotalLocked, &new_total);

        let current_ledger = env.ledger().sequence();
        Self::update_global_checkpoint(&env, current_ledger);

        emit_lock_increased(&env, &owner, additional_amount, lock.initial_voting_power);

        lock
    }

    pub fn extend_lock(env: Env, owner: Address, new_end_ledger: u32) -> Lock {
        owner.require_auth();

        let mut lock: Lock = env
            .storage()
            .persistent()
            .get(&DataKey::Lock(owner.clone()))
            .ok_or(VoteEscrowError::LockNotFound)
            .unwrap();

        if lock.withdrawn {
            env.panic_with_error(VoteEscrowError::LockAlreadyWithdrawn);
        }

        if new_end_ledger <= lock.end_ledger {
            env.panic_with_error(VoteEscrowError::InvalidEndLedger);
        }

        let min_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinLockDuration)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();
        let max_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxLockDuration)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();
        let max_multiplier_bps: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxMultiplierBps)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();

        let new_duration = new_end_ledger.saturating_sub(lock.start_ledger);

        if new_duration < min_duration || new_duration > max_duration {
            env.panic_with_error(VoteEscrowError::InvalidDuration);
        }

        let old_end_ledger = lock.end_ledger;
        let new_initial_voting_power =
            Self::compute_initial_voting_power(&env, lock.amount, new_duration, min_duration, max_duration, max_multiplier_bps);

        lock.end_ledger = new_end_ledger;
        lock.initial_voting_power = new_initial_voting_power;

        env.storage()
            .persistent()
            .set(&DataKey::Lock(owner.clone()), &lock);
        Self::refresh_persistent_ttl(&env, &DataKey::Lock(owner.clone()));

        let current_ledger = env.ledger().sequence();
        Self::update_global_checkpoint(&env, current_ledger);

        emit_lock_extended(&env, &owner, old_end_ledger, new_end_ledger);

        lock
    }

    pub fn withdraw(env: Env, owner: Address) -> i128 {
        owner.require_auth();

        let mut lock: Lock = env
            .storage()
            .persistent()
            .get(&DataKey::Lock(owner.clone()))
            .ok_or(VoteEscrowError::LockNotFound)
            .unwrap();

        if lock.withdrawn {
            env.panic_with_error(VoteEscrowError::LockAlreadyWithdrawn);
        }

        let current_ledger = env.ledger().sequence();

        if current_ledger < lock.end_ledger {
            env.panic_with_error(VoteEscrowError::LockNotMatured);
        }

        let locked_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::LockedToken)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap();

        token::Client::new(&env, &locked_token).transfer(
            &env.current_contract_address(),
            &owner,
            &lock.amount,
        );

        lock.withdrawn = true;

        Self::append_lock_history(&env, &owner, &lock);

        env.storage()
            .persistent()
            .remove(&DataKey::Lock(owner.clone()));

        let current_total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        let new_total = current_total.checked_sub(lock.amount)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap();
        env.storage()
            .instance()
            .set(&DataKey::TotalLocked, &new_total);

        Self::update_global_checkpoint(&env, current_ledger);

        emit_lock_withdrawn(&env, &owner, lock.amount);

        lock.amount
    }

    pub fn get_votes(env: Env, account: Address) -> i128 {
        let lock_opt: Option<Lock> = env
            .storage()
            .persistent()
            .get(&DataKey::Lock(account));

        match lock_opt {
            Some(lock) => {
                if lock.withdrawn {
                    return 0;
                }
                let current_ledger = env.ledger().sequence();
                Self::compute_decayed_power(&lock, current_ledger)
            }
            None => 0,
        }
    }

    pub fn get_past_votes(env: Env, account: Address, ledger: u32) -> i128 {
        let lock_opt: Option<Lock> = env
            .storage()
            .persistent()
            .get(&DataKey::Lock(account.clone()));

        if let Some(lock) = lock_opt {
            if !lock.withdrawn && ledger >= lock.start_ledger && ledger < lock.end_ledger {
                return Self::compute_decayed_power(&lock, ledger);
            }
            if ledger >= lock.start_ledger && ledger < lock.end_ledger {
                return Self::compute_decayed_power(&lock, ledger);
            }
        }

        // Scan only the bounded history window. The previous implementation
        // walked an unbounded `Vec<Lock>` on every historical query.
        let retained = Self::retained_history_len(&env, &account);
        for index in 0..retained {
            if let Some(historical_lock) = Self::read_lock_history_entry(&env, &account, index) {
                if ledger >= historical_lock.start_ledger && ledger < historical_lock.end_ledger {
                    return Self::compute_decayed_power(&historical_lock, ledger);
                }
            }
        }

        0
    }

    pub fn get_past_total_supply(env: Env, ledger: u32) -> i128 {
        let checkpoints_opt: Option<Vec<(u32, i128)>> = env
            .storage()
            .instance()
            .get(&DataKey::GlobalCheckpoints);

        if let Some(checkpoints) = checkpoints_opt {
            return Self::binary_search_checkpoint(&checkpoints, ledger);
        }

        0
    }

    pub fn token(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::LockedToken)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap()
    }

    pub fn get_total_locked(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap()
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(VoteEscrowError::NotInitialized)
            .unwrap()
    }

    pub fn get_lock(env: Env, owner: Address) -> Option<Lock> {
        env.storage()
            .persistent()
            .get(&DataKey::Lock(owner))
    }

    pub fn get_lock_history(env: Env, owner: Address, offset: u32, limit: u32) -> Vec<Lock> {
        let retained = Self::retained_history_len(&env, &owner);
        let mut result = Vec::new(&env);

        if offset >= retained || limit == 0 {
            return result;
        }

        let end = offset.saturating_add(limit).min(retained);
        for index in offset..end {
            if let Some(entry) = Self::read_lock_history_entry(&env, &owner, index) {
                result.push_back(entry);
            }
        }
        result
    }

    pub fn update_escrow_config(
        env: Env,
        admin: Address,
        min_lock_duration: u32,
        max_lock_duration: u32,
        max_multiplier_bps: u32,
    ) {
        admin.require_auth();

        let old_min_lock_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinLockDuration)
            .unwrap_or_else(|| env.panic_with_error(VoteEscrowError::NotInitialized));
        let old_max_lock_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxLockDuration)
            .unwrap_or_else(|| env.panic_with_error(VoteEscrowError::NotInitialized));
        let old_max_multiplier_bps: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxMultiplierBps)
            .unwrap_or_else(|| env.panic_with_error(VoteEscrowError::NotInitialized));

        env.storage()
            .instance()
            .set(&DataKey::MinLockDuration, &min_lock_duration);
        env.storage()
            .instance()
            .set(&DataKey::MaxLockDuration, &max_lock_duration);
        env.storage()
            .instance()
            .set(&DataKey::MaxMultiplierBps, &max_multiplier_bps);

        emit_escrow_config_updated(
            &env,
            EscrowConfigUpdatedEvent {
                old_min_lock_duration,
                new_min_lock_duration: min_lock_duration,
                old_max_lock_duration,
                new_max_lock_duration: max_lock_duration,
                old_max_multiplier_bps,
                new_max_multiplier_bps: max_multiplier_bps,
            },
        );
    }

    fn compute_initial_voting_power(
        _env: &Env,
        amount: i128,
        duration_ledgers: u32,
        min_duration: u32,
        max_duration: u32,
        max_multiplier_bps: u32,
    ) -> i128 {
        if duration_ledgers <= min_duration {
            return amount;
        }

        let duration_range = max_duration.saturating_sub(min_duration) as i128;
        if duration_range == 0 {
            let boost = amount
                .checked_mul(max_multiplier_bps as i128)
                .ok_or(VoteEscrowError::ArithmeticOverflow)
                .unwrap()
                .checked_div(10000)
                .ok_or(VoteEscrowError::ArithmeticOverflow)
                .unwrap();
            return amount
                .checked_add(boost)
                .ok_or(VoteEscrowError::ArithmeticOverflow)
                .unwrap();
        }

        let applicable_duration = (duration_ledgers.saturating_sub(min_duration)) as i128;
        let applicable_bps = applicable_duration
            .checked_mul(max_multiplier_bps as i128)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap()
            .checked_div(duration_range)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap();

        let boost = amount
            .checked_mul(applicable_bps)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap()
            .checked_div(10000)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap();

        amount
            .checked_add(boost)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap()
    }

    fn compute_decayed_power(lock: &Lock, current_ledger: u32) -> i128 {
        if current_ledger >= lock.end_ledger {
            return lock.amount;
        }

        if current_ledger < lock.start_ledger {
            return 0;
        }

        let duration = lock.end_ledger.saturating_sub(lock.start_ledger) as i128;
        if duration == 0 {
            return lock.amount;
        }

        let remaining = (lock.end_ledger.saturating_sub(current_ledger)) as i128;

        let boost = lock.initial_voting_power.saturating_sub(lock.amount);
        let decayed_boost = boost
            .checked_mul(remaining)
            .ok_or(VoteEscrowError::ArithmeticOverflow)
            .unwrap()
            .checked_div(duration)
            .unwrap_or(0);

        lock.amount
            .checked_add(decayed_boost)
            .unwrap_or(lock.amount)
    }

    /// Number of history records currently retained for `owner`.
    ///
    /// Capped at [`MAX_LOCK_HISTORY_ENTRIES`] so history reads stay bounded no
    /// matter how many lock/withdraw cycles the owner has performed.
    fn retained_history_len(env: &Env, owner: &Address) -> u32 {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LockHistoryCount(owner.clone()))
            .unwrap_or(0);
        count.min(MAX_LOCK_HISTORY_ENTRIES)
    }

    /// Append one withdrawn lock to `owner`'s bounded history ring buffer.
    ///
    /// A constant number of storage writes per withdrawal, regardless of how
    /// long the history is — the previous implementation read the whole
    /// unbounded entry, pushed, and wrote it all back.
    fn append_lock_history(env: &Env, owner: &Address, lock: &Lock) {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LockHistoryCount(owner.clone()))
            .unwrap_or(0);
        let slot = count % MAX_LOCK_HISTORY_ENTRIES;
        env.storage()
            .persistent()
            .set(&DataKey::LockHistoryEntry(owner.clone(), slot), lock);
        env.storage().persistent().set(
            &DataKey::LockHistoryCount(owner.clone()),
            &count.saturating_add(1),
        );
    }

    /// Read the `index`-th retained history record, oldest first.
    ///
    /// Returns `None` when `index` falls outside the retained window.
    fn read_lock_history_entry(env: &Env, owner: &Address, index: u32) -> Option<Lock> {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LockHistoryCount(owner.clone()))
            .unwrap_or(0);
        let retained = count.min(MAX_LOCK_HISTORY_ENTRIES);
        if index >= retained {
            return None;
        }
        // Map the retained logical index through the ring buffer to a slot.
        let logical = count.saturating_sub(retained).saturating_add(index);
        let slot = logical % MAX_LOCK_HISTORY_ENTRIES;
        env.storage()
            .persistent()
            .get(&DataKey::LockHistoryEntry(owner.clone(), slot))
    }

    fn update_global_checkpoint(env: &Env, ledger: u32) {
        let mut checkpoints: Vec<(u32, i128)> = env
            .storage()
            .instance()
            .get(&DataKey::GlobalCheckpoints)
            .unwrap_or_else(|| Vec::new(env));

        let total_locked: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);

        checkpoints.push_back((ledger, total_locked));

        env.storage()
            .instance()
            .set(&DataKey::GlobalCheckpoints, &checkpoints);
    }

    fn refresh_persistent_ttl(env: &Env, key: &DataKey) {
        env.storage().persistent().extend_ttl(key, 100_000, 100_000);
    }

    fn binary_search_checkpoint(checkpoints: &Vec<(u32, i128)>, ledger: u32) -> i128 {
        let mut low = 0;
        let mut high = checkpoints.len();

        while low < high {
            let mid = (low + high) / 2;
            if checkpoints.get(mid).unwrap().0 <= ledger {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        if low == 0 {
            0
        } else {
            checkpoints.get(low - 1).unwrap().1
        }
    }
}

#[cfg(test)]
mod tests;
