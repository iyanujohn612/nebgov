extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token, Env, Symbol, TryIntoVal,
};

const MIN_LOCK_DURATION: u32 = 100;
const MAX_LOCK_DURATION: u32 = 10_000;
const MAX_MULTIPLIER_BPS: u32 = 25_000; // 2.5x multiplier at max duration

struct Fixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    token: Address,
    user: Address,
    user2: Address,
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let user2 = Address::generate(&env);
    let token = Address::generate(&env);
    let contract_id = env.register(VoteEscrowContract, ());

    // Initialize contract
    env.as_contract(&contract_id, || {
        VoteEscrowContract::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            MIN_LOCK_DURATION,
            MAX_LOCK_DURATION,
            MAX_MULTIPLIER_BPS,
        );
    });

    Fixture {
        env,
        contract_id,
        admin,
        token,
        user,
        user2,
    }
}

#[test]
fn test_initialize() {
    let f = setup();
    f.env.as_contract(&f.contract_id, || {
        let stored_token: Address = f
            .env
            .storage()
            .instance()
            .get(&DataKey::LockedToken)
            .unwrap();
        assert_eq!(stored_token, f.token);

        let stored_min: u32 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::MinLockDuration)
            .unwrap();
        assert_eq!(stored_min, MIN_LOCK_DURATION);

        let stored_max: u32 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::MaxLockDuration)
            .unwrap();
        assert_eq!(stored_max, MAX_LOCK_DURATION);

        let total_locked: i128 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        assert_eq!(total_locked, 0);
        assert_eq!(VoteEscrowContract::get_total_locked(f.env.clone()), 0);
        assert_eq!(VoteEscrowContract::get_admin(f.env.clone()), f.admin);
    });
}

#[test]
fn test_create_lock_basic() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        assert_eq!(lock.owner, f.user);
        assert_eq!(lock.amount, amount);
        assert!(!lock.withdrawn);
        assert!(lock.initial_voting_power >= amount);
    });
}

#[test]
fn test_create_lock_updates_total() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let total_locked: i128 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        assert_eq!(total_locked, amount);
    });
}

#[test]
fn test_get_past_total_supply_checkpoint_boundaries() {
    let f = setup();
    let checkpoints: Vec<(u32, i128)> =
        Vec::from_array(&f.env, [(10, 100), (20, 200), (30, 300)]);

    f.env.as_contract(&f.contract_id, || {
        f.env
            .storage()
            .instance()
            .set(&DataKey::GlobalCheckpoints, &checkpoints);

        assert_eq!(VoteEscrowContract::get_past_total_supply(f.env.clone(), 20), 200);
        assert_eq!(VoteEscrowContract::get_past_total_supply(f.env.clone(), 25), 200);
        assert_eq!(VoteEscrowContract::get_past_total_supply(f.env.clone(), 5), 0);
        assert_eq!(VoteEscrowContract::get_past_total_supply(f.env.clone(), 35), 300);
    });
}

#[test]
fn test_active_lock_survives_persistent_ttl_and_withdraws() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = sac.address();
    let contract_id = env.register(VoteEscrowContract, ());
    token::StellarAssetClient::new(&env, &token).mint(&user, &1_000);

    env.as_contract(&contract_id, || {
        VoteEscrowContract::initialize(
            env.clone(),
            admin,
            token,
            MIN_LOCK_DURATION,
            MAX_LOCK_DURATION,
            MAX_MULTIPLIER_BPS,
        );
        env.storage()
            .instance()
            .extend_ttl(MAX_LOCK_DURATION + 1, MAX_LOCK_DURATION + 1);
    });

    let lock = env.as_contract(&contract_id, || {
        VoteEscrowContract::create_lock(env.clone(), user.clone(), 1_000, MAX_LOCK_DURATION)
    });
    env.ledger().set_sequence_number(lock.start_ledger + 5_000);

    env.as_contract(&contract_id, || {
        assert!(VoteEscrowContract::get_lock(env.clone(), user.clone()).is_some());
    });

    env.ledger().set_sequence_number(lock.end_ledger);
    env.as_contract(&contract_id, || {
        assert_eq!(VoteEscrowContract::withdraw(env.clone(), user), 1_000);
    });
}

#[test]
fn test_create_lock_min_duration() {
    let f = setup();
    let amount = 1_000;
    let duration = MIN_LOCK_DURATION;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // At minimum duration, voting power should equal amount
        assert_eq!(lock.initial_voting_power, amount);
    });
}

#[test]
fn test_create_lock_max_duration() {
    let f = setup();
    let amount = 1_000;
    let duration = MAX_LOCK_DURATION;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // At maximum duration with 2.5x multiplier, voting power should be 3.5x amount
        // (amount + 2.5x boost)
        assert_eq!(lock.initial_voting_power, amount * 350 / 100);
    });
}

#[test]
fn test_create_lock_insufficient_duration() {
    let f = setup();
    let amount = 1_000;
    let duration = MIN_LOCK_DURATION - 1;

    f.env.as_contract(&f.contract_id, || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::create_lock(
                f.env.clone(),
                f.user.clone(),
                amount,
                duration,
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_create_lock_excessive_duration() {
    let f = setup();
    let amount = 1_000;
    let duration = MAX_LOCK_DURATION + 1;

    f.env.as_contract(&f.contract_id, || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::create_lock(
                f.env.clone(),
                f.user.clone(),
                amount,
                duration,
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_create_lock_invalid_amount() {
    let f = setup();
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::create_lock(
                f.env.clone(),
                f.user.clone(),
                0,
                duration,
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_increase_lock_amount() {
    let f = setup();
    let amount = 1_000;
    let additional_amount = 500;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock1 = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let lock2 = VoteEscrowContract::increase_lock_amount(
            f.env.clone(),
            f.user.clone(),
            additional_amount,
        );

        assert_eq!(lock2.amount, amount + additional_amount);
        assert!(lock2.initial_voting_power > lock1.initial_voting_power);
    });
}

#[test]
fn test_increase_lock_amount_updates_total() {
    let f = setup();
    let amount = 1_000;
    let additional_amount = 500;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        VoteEscrowContract::increase_lock_amount(
            f.env.clone(),
            f.user.clone(),
            additional_amount,
        );

        let total_locked: i128 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        assert_eq!(total_locked, amount + additional_amount);
    });
}

#[test]
fn test_increase_lock_amount_invalid_amount() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::increase_lock_amount(
                f.env.clone(),
                f.user.clone(),
                0,
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_increase_lock_amount_non_existent_lock() {
    let f = setup();
    let additional_amount = 500;

    f.env.as_contract(&f.contract_id, || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::increase_lock_amount(
                f.env.clone(),
                f.user.clone(),
                additional_amount,
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_extend_lock() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;
    let _new_duration = 2_000;

    f.env.as_contract(&f.contract_id, || {
        let lock1 = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let new_end_ledger = lock1.end_ledger.checked_add(1_000).unwrap();
        let lock2 = VoteEscrowContract::extend_lock(
            f.env.clone(),
            f.user.clone(),
            new_end_ledger,
        );

        assert_eq!(lock2.end_ledger, new_end_ledger);
        assert!(lock2.initial_voting_power > lock1.initial_voting_power);
    });
}

#[test]
fn test_extend_lock_invalid_end_ledger() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::extend_lock(
                f.env.clone(),
                f.user.clone(),
                lock.end_ledger,
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_extend_lock_to_min_duration() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let new_duration = lock.end_ledger.checked_add(MIN_LOCK_DURATION).unwrap();
        let extended_lock = VoteEscrowContract::extend_lock(
            f.env.clone(),
            f.user.clone(),
            new_duration,
        );

        assert_eq!(extended_lock.end_ledger, new_duration);
    });
}

#[test]
fn test_withdraw() {
    let f = setup();
    let amount = 1_000;
    let duration = 100; // short duration

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // Advance ledger past lock maturity
        f.env.ledger().set_sequence_number(lock.end_ledger + 1);

        let withdrawn_amount = VoteEscrowContract::withdraw(
            f.env.clone(),
            f.user.clone(),
        );

        assert_eq!(withdrawn_amount, amount);

        // Verify lock is marked withdrawn
        let stored_lock: Option<Lock> = f
            .env
            .storage()
            .persistent()
            .get(&DataKey::Lock(f.user.clone()));
        assert!(stored_lock.is_none());
    });
}

#[test]
fn test_withdraw_updates_total_checked() {
    let f = setup();
    let amount = 1_000;
    let duration = 100;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let total_before: i128 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        assert_eq!(total_before, amount);

        // Advance ledger past lock maturity
        let lock: Lock = f
            .env
            .storage()
            .persistent()
            .get(&DataKey::Lock(f.user.clone()))
            .unwrap();
        f.env.ledger().set_sequence_number(lock.end_ledger + 1);

        VoteEscrowContract::withdraw(
            f.env.clone(),
            f.user.clone(),
        );

        let total_after: i128 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        assert_eq!(total_after, 0);
    });
}

#[test]
fn test_withdraw_before_maturity() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // Try to withdraw before maturity
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::withdraw(
                f.env.clone(),
                f.user.clone(),
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_withdraw_already_withdrawn() {
    let f = setup();
    let amount = 1_000;
    let duration = 100;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // Advance ledger past lock maturity
        f.env.ledger().set_sequence_number(lock.end_ledger + 1);

        VoteEscrowContract::withdraw(
            f.env.clone(),
            f.user.clone(),
        );

        // Try to withdraw again
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::withdraw(
                f.env.clone(),
                f.user.clone(),
            );
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_get_votes_active_lock() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let votes = VoteEscrowContract::get_votes(f.env.clone(), f.user.clone());
        assert_eq!(votes, lock.initial_voting_power);
    });
}

#[test]
fn test_get_votes_decayed() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // Advance to midpoint
        let midpoint = lock.start_ledger + (duration / 2);
        f.env.ledger().set_sequence_number(midpoint);

        let votes = VoteEscrowContract::get_votes(f.env.clone(), f.user.clone());
        assert!(votes < lock.initial_voting_power);
        assert!(votes > amount);
    });
}

#[test]
fn test_get_votes_expired() {
    let f = setup();
    let amount = 1_000;
    let duration = 100;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // Advance past maturity
        f.env.ledger().set_sequence_number(lock.end_ledger + 1);

        let votes = VoteEscrowContract::get_votes(f.env.clone(), f.user.clone());
        assert_eq!(votes, amount);
    });
}

#[test]
fn test_get_votes_no_lock() {
    let f = setup();

    f.env.as_contract(&f.contract_id, || {
        let votes = VoteEscrowContract::get_votes(f.env.clone(), f.user.clone());
        assert_eq!(votes, 0);
    });
}

#[test]
fn test_get_past_votes_during_lock() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let votes = VoteEscrowContract::get_past_votes(
            f.env.clone(),
            f.user.clone(),
            lock.start_ledger + 500,
        );
        assert!(votes > 0);
        assert!(votes <= lock.initial_voting_power);
    });
}

#[test]
fn test_get_past_votes_before_lock() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        let votes = VoteEscrowContract::get_past_votes(
            f.env.clone(),
            f.user.clone(),
            lock.start_ledger - 1,
        );
        assert_eq!(votes, 0);
    });
}

#[test]
fn test_get_lock_history() {
    let f = setup();
    let amount = 1_000;
    let duration = 100;

    f.env.as_contract(&f.contract_id, || {
        let lock1 = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );

        // Withdraw first lock
        f.env.ledger().set_sequence_number(lock1.end_ledger + 1);
        VoteEscrowContract::withdraw(f.env.clone(), f.user.clone());

        // Create second lock
        f.env.ledger().set_sequence_number(lock1.end_ledger + 2);
        let lock2 = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount * 2,
            duration,
        );

        // Withdraw second lock
        f.env.ledger().set_sequence_number(lock2.end_ledger + 1);
        VoteEscrowContract::withdraw(f.env.clone(), f.user.clone());

        let history = VoteEscrowContract::get_lock_history(
            f.env.clone(),
            f.user.clone(),
            0,
            10,
        );
        assert_eq!(history.len(), 2);
    });
}

#[test]
fn test_multiple_users_independent_locks() {
    let f = setup();
    let amount1 = 1_000;
    let amount2 = 2_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount1,
            duration,
        );

        VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user2.clone(),
            amount2,
            duration,
        );

        let total_locked: i128 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0);
        assert_eq!(total_locked, amount1 + amount2);

        let votes1 = VoteEscrowContract::get_votes(f.env.clone(), f.user.clone());
        let votes2 = VoteEscrowContract::get_votes(f.env.clone(), f.user2.clone());

        assert!(votes1 > amount1);
        assert!(votes2 > amount2);
        assert!(votes2 > votes1); // More locked amount = more votes
    });
}

#[test]
fn test_update_escrow_config() {
    let f = setup();
    let new_min = 50;
    let new_max = 5_000;
    let new_multiplier = 10_000;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::update_escrow_config(
            f.env.clone(),
            f.admin.clone(),
            new_min,
            new_max,
            new_multiplier,
        );

        let stored_min: u32 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::MinLockDuration)
            .unwrap();
        assert_eq!(stored_min, new_min);

        let stored_max: u32 = f
            .env
            .storage()
            .instance()
            .get(&DataKey::MaxLockDuration)
            .unwrap();
        assert_eq!(stored_max, new_max);

        // Create lock with new minimum duration
        VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            1_000,
            new_min,
        );
    });
}

/// Finds the first event published by `contract_id` whose first topic is
/// `topic`, and returns its full topic list and payload. Panics if none
/// matches, so a missing/renamed event fails the test loudly.
fn find_event(env: &Env, contract_id: &Address, topic: &str) -> (Vec<soroban_sdk::Val>, soroban_sdk::Val) {
    let topic_symbol = Symbol::new(env, topic);
    let events = env.events().all();
    let (_, topics, data) = events
        .iter()
        .find(|(addr, topics, _)| {
            *addr == *contract_id
                && topics.get(0).is_some_and(|t| {
                    let first: Result<Symbol, _> = t.try_into_val(env);
                    first.is_ok() && first.unwrap() == topic_symbol
                })
        })
        .unwrap_or_else(|| panic!("expected a {} event", topic));
    (topics, data)
}

#[test]
fn test_create_lock_emits_lock_created_event() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(f.env.clone(), f.user.clone(), amount, duration);

        let (topics, data) = find_event(&f.env, &f.contract_id, LOCK_CREATED_TOPIC);
        let owner_topic: Address = topics.get(1).unwrap().try_into_val(&f.env).unwrap();
        assert_eq!(owner_topic, f.user);

        let decoded: LockCreatedEvent = data.try_into_val(&f.env).unwrap();
        assert_eq!(decoded.owner, f.user);
        assert_eq!(decoded.amount, amount);
        assert_eq!(decoded.end_ledger, lock.end_ledger);
        assert_eq!(decoded.initial_voting_power, lock.initial_voting_power);
    });
}

#[test]
fn test_increase_lock_amount_emits_lock_increased_event() {
    let f = setup();
    let amount = 1_000;
    let additional_amount = 500;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::create_lock(f.env.clone(), f.user.clone(), amount, duration);

        let lock2 = VoteEscrowContract::increase_lock_amount(
            f.env.clone(),
            f.user.clone(),
            additional_amount,
        );

        let (topics, data) = find_event(&f.env, &f.contract_id, LOCK_INCREASED_TOPIC);
        let owner_topic: Address = topics.get(1).unwrap().try_into_val(&f.env).unwrap();
        assert_eq!(owner_topic, f.user);

        let decoded: LockIncreasedEvent = data.try_into_val(&f.env).unwrap();
        assert_eq!(decoded.owner, f.user);
        assert_eq!(decoded.added_amount, additional_amount);
        assert_eq!(decoded.new_voting_power, lock2.initial_voting_power);
    });
}

#[test]
fn test_extend_lock_emits_lock_extended_event() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock1 = VoteEscrowContract::create_lock(f.env.clone(), f.user.clone(), amount, duration);
        let new_end_ledger = lock1.end_ledger.checked_add(1_000).unwrap();

        VoteEscrowContract::extend_lock(f.env.clone(), f.user.clone(), new_end_ledger);

        let (topics, data) = find_event(&f.env, &f.contract_id, LOCK_EXTENDED_TOPIC);
        let owner_topic: Address = topics.get(1).unwrap().try_into_val(&f.env).unwrap();
        assert_eq!(owner_topic, f.user);

        let decoded: LockExtendedEvent = data.try_into_val(&f.env).unwrap();
        assert_eq!(decoded.owner, f.user);
        assert_eq!(decoded.old_end_ledger, lock1.end_ledger);
        assert_eq!(decoded.new_end_ledger, new_end_ledger);
    });
}

#[test]
fn test_withdraw_emits_lock_withdrawn_event() {
    let f = setup();
    let amount = 1_000;
    let duration = 100;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(f.env.clone(), f.user.clone(), amount, duration);
        f.env.ledger().set_sequence_number(lock.end_ledger + 1);

        VoteEscrowContract::withdraw(f.env.clone(), f.user.clone());

        let (topics, data) = find_event(&f.env, &f.contract_id, LOCK_WITHDRAWN_TOPIC);
        let owner_topic: Address = topics.get(1).unwrap().try_into_val(&f.env).unwrap();
        assert_eq!(owner_topic, f.user);

        let decoded: LockWithdrawnEvent = data.try_into_val(&f.env).unwrap();
        assert_eq!(decoded.owner, f.user);
        assert_eq!(decoded.amount, amount);
    });
}

#[test]
fn test_update_escrow_config_emits_escrow_config_updated_event() {
    let f = setup();
    let new_min = 50;
    let new_max = 5_000;
    let new_multiplier = 10_000;

    f.env.as_contract(&f.contract_id, || {
        VoteEscrowContract::update_escrow_config(
            f.env.clone(),
            f.admin.clone(),
            new_min,
            new_max,
            new_multiplier,
        );

        let (topics, data) = find_event(&f.env, &f.contract_id, ESCROW_CONFIG_UPDATED_TOPIC);
        assert_eq!(topics.len(), 1);

        let decoded: EscrowConfigUpdatedEvent = data.try_into_val(&f.env).unwrap();
        assert_eq!(decoded.old_min_lock_duration, MIN_LOCK_DURATION);
        assert_eq!(decoded.new_min_lock_duration, new_min);
        assert_eq!(decoded.old_max_lock_duration, MAX_LOCK_DURATION);
        assert_eq!(decoded.new_max_lock_duration, new_max);
        assert_eq!(decoded.old_max_multiplier_bps, MAX_MULTIPLIER_BPS);
        assert_eq!(decoded.new_max_multiplier_bps, new_multiplier);
    });
}

#[test]
fn test_create_lock_overflowing_boost_computation_panics() {
    let f = setup();
    // Chosen so `amount * MAX_MULTIPLIER_BPS` overflows i128 inside
    // compute_initial_voting_power's boost calculation.
    let amount = i128::MAX / (MAX_MULTIPLIER_BPS as i128) + 1;
    let duration = MAX_LOCK_DURATION;

    f.env.as_contract(&f.contract_id, || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            VoteEscrowContract::create_lock(f.env.clone(), f.user.clone(), amount, duration);
        }));
        assert!(result.is_err());
    });
}

#[test]
fn test_lock_history_is_bounded_across_many_cycles() {
    let f = setup();
    let amount = 100;
    let duration = MIN_LOCK_DURATION;
    let cycles = MAX_LOCK_HISTORY_ENTRIES + 8;

    f.env.as_contract(&f.contract_id, || {
        let mut ledger = 10;
        for _ in 0..cycles {
            f.env.ledger().set_sequence_number(ledger);
let _lock = VoteEscrowContract::create_lock(
                f.env.clone(),
                f.user.clone(),
                amount,
                duration,
            );
            f.env.ledger().set_sequence_number(lock.end_ledger + 1);
            VoteEscrowContract::withdraw(f.env.clone(), f.user.clone());
            ledger = lock.end_ledger + 2;
        }

        // The append counter is monotonic, but only a bounded window is retained.
        let count: u32 = f
            .env
            .storage()
            .persistent()
            .get(&DataKey::LockHistoryCount(f.user.clone()))
            .unwrap_or(0);
        assert_eq!(count, cycles);

        let all =
            VoteEscrowContract::get_lock_history(f.env.clone(), f.user.clone(), 0, 1_000);
        assert_eq!(all.len(), MAX_LOCK_HISTORY_ENTRIES);

        // Pagination is clamped to the retained window instead of walking history.
        let first_page =
            VoteEscrowContract::get_lock_history(f.env.clone(), f.user.clone(), 0, 5);
        assert_eq!(first_page.len(), 5);
        let past_end =
            VoteEscrowContract::get_lock_history(f.env.clone(), f.user.clone(), cycles, 5);
        assert_eq!(past_end.len(), 0);

        // Historical queries still resolve from the bounded window: the most
        // recent withdrawn record covers the ledger we advance past below.
        let last = all.get(all.len() - 1).unwrap();
        let probe = last.end_ledger.saturating_sub(1);
        let votes =
            VoteEscrowContract::get_past_votes(f.env.clone(), f.user.clone(), probe);
        assert!(votes > 0);
    });
}

#[test]
fn test_get_past_votes_reads_bounded_history_after_withdraw() {
    let f = setup();
    let amount = 1_000;
    let duration = 1_000;

    f.env.as_contract(&f.contract_id, || {
        let lock = VoteEscrowContract::create_lock(
            f.env.clone(),
            f.user.clone(),
            amount,
            duration,
        );
        let probe = lock.start_ledger + 250;

        f.env.ledger().set_sequence_number(lock.end_ledger + 1);
        VoteEscrowContract::withdraw(f.env.clone(), f.user.clone());

        // The live record is gone; the historical answer must come from the
        // indexed history entry.
        assert!(VoteEscrowContract::get_lock(f.env.clone(), f.user.clone()).is_none());

        let votes =
            VoteEscrowContract::get_past_votes(f.env.clone(), f.user.clone(), probe);
        assert!(votes > 0);
        assert!(votes <= lock.initial_voting_power);
    });
}
