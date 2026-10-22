//! Tests for the Proposer Reputation System (Issue #771).
//!
//! Most tests here call `crate::reputation`'s functions directly inside
//! `env.as_contract(...)`, mirroring the existing precedent in
//! `tests/mod.rs::governor_upgraded_event_helper_emits_expected_topic` — this
//! lets pure scoring/threshold-math behavior be tested without needing a full
//! propose→vote→queue→execute cycle for every case. A handful of tests do
//! drive the real `GovernorContractClient` end-to-end (via `propose()`,
//! `state()`, `queue()`) specifically to verify the wiring at each of those
//! call sites actually invokes the reputation module.

use crate::{reputation, GovernorContract, GovernorContractClient, ProposalState, VoteSupport, VoteType};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger as _},
    Address, Bytes, Env, String, Symbol,
};
use sorogov_timelock::{TimelockContract, TimelockContractClient};

#[contracttype]
#[derive(Clone)]
enum VotesKey {
    Votes(Address),
    TotalSupply,
}

/// Minimal configurable votes mock: lets each test set exact per-address
/// voting power and total supply so quorum/participation/threshold math is
/// fully deterministic.
#[contract]
pub struct ReputationTestVotesContract;

#[contractimpl]
impl ReputationTestVotesContract {
    pub fn set_votes(env: Env, account: Address, votes: i128) {
        env.storage().instance().set(&VotesKey::Votes(account), &votes);
    }

    pub fn set_total_supply(env: Env, supply: i128) {
        env.storage().instance().set(&VotesKey::TotalSupply, &supply);
    }

    pub fn get_votes(env: Env, account: Address) -> i128 {
        env.storage()
            .instance()
            .get(&VotesKey::Votes(account))
            .unwrap_or(0)
    }

    pub fn get_past_votes(env: Env, account: Address, _ledger: u32) -> i128 {
        Self::get_votes(env, account)
    }

    pub fn get_past_total_supply(env: Env, _ledger: u32) -> i128 {
        env.storage()
            .instance()
            .get(&VotesKey::TotalSupply)
            .unwrap_or(0)
    }

    pub fn token(env: Env) -> Address {
        Address::generate(&env)
    }
}

struct Harness {
    env: Env,
    governor: GovernorContractClient<'static>,
    governor_id: Address,
    votes: ReputationTestVotesContractClient<'static>,
    guardian: Address,
}

/// Total supply is fixed at 10,000 across all tests; individual voter/
/// proposer voting power is set per-test via `Harness::votes.set_votes`.
fn setup(
    voting_delay: u32,
    voting_period: u32,
    quorum_numerator: u32,
    proposal_threshold: i128,
    proposal_grace_period: u32,
) -> Harness {
    setup_with_min_delay(
        voting_delay,
        voting_period,
        quorum_numerator,
        proposal_threshold,
        proposal_grace_period,
        1,
    )
}

/// Same as `setup`, but with a configurable timelock `min_delay` (seconds).
/// The veto window for `cancel_queued` is `min_delay / 5` ledgers, so tests
/// that cancel a queued proposal need a delay longer than one ledger.
fn setup_with_min_delay(
    voting_delay: u32,
    voting_period: u32,
    quorum_numerator: u32,
    proposal_threshold: i128,
    proposal_grace_period: u32,
    min_delay: u64,
) -> Harness {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let votes_id = env.register(ReputationTestVotesContract, ());
    let votes = ReputationTestVotesContractClient::new(&env, &votes_id);
    votes.set_total_supply(&10_000i128);

    let timelock_id = env.register(TimelockContract, ());
    let governor_id = env.register(GovernorContract, ());
    let timelock = TimelockContractClient::new(&env, &timelock_id);
    let governor = GovernorContractClient::new(&env, &governor_id);

    timelock.initialize(&admin, &governor_id, &min_delay, &1_209_600u64);

    let guardian = Address::generate(&env);
    governor.initialize(
        &admin,
        &votes_id,
        &timelock_id,
        &voting_delay,
        &voting_period,
        &quorum_numerator,
        &proposal_threshold,
        &guardian,
        &VoteType::Extended,
        &proposal_grace_period,
    );

    Harness {
        env,
        governor,
        governor_id,
        votes,
        guardian,
    }
}

fn propose_dummy(
    env: &Env,
    governor: &GovernorContractClient,
    proposer: &Address,
    seed: &[u8],
) -> u64 {
    let target = Address::generate(env);
    let fn_name = Symbol::new(env, "exec");
    let calldata = Bytes::new(env);
    let description = String::from_str(env, "Reputation test proposal");
    let description_hash = env.crypto().sha256(&Bytes::from_slice(env, seed)).into();
    let metadata_uri = String::from_str(env, "ipfs://reputation-test");

    let mut targets = soroban_sdk::Vec::new(env);
    targets.push_back(target);
    let mut fn_names = soroban_sdk::Vec::new(env);
    fn_names.push_back(fn_name);
    let mut calldatas = soroban_sdk::Vec::new(env);
    calldatas.push_back(calldata);

    governor.propose(
        proposer,
        &description,
        &description_hash,
        &metadata_uri,
        &targets,
        &fn_names,
        &calldatas,
    )
}

#[test]
fn test_score_increments_on_successful_proposal() {
    let h = setup(10, 20, 20, 0, 120_960);
    let proposer = Address::generate(&h.env);
    h.votes.set_votes(&proposer, &100_000i128);
    let voter_a = Address::generate(&h.env);
    let voter_b = Address::generate(&h.env);
    h.votes.set_votes(&voter_a, &1_000i128);
    h.votes.set_votes(&voter_b, &1_000i128);

    let proposal_id = propose_dummy(&h.env, &h.governor, &proposer, b"succeed-1");

    h.env.ledger().with_mut(|l| l.sequence_number = 11);
    h.governor.cast_vote(&voter_a, &proposal_id, &VoteSupport::For);
    h.governor.cast_vote(&voter_b, &proposal_id, &VoteSupport::For);

    h.env.ledger().with_mut(|l| l.sequence_number = 31);
    assert_eq!(h.governor.state(&proposal_id), ProposalState::Succeeded);

    // queue() re-verifies quorum/vote direction and is the point at which a
    // Succeeded outcome is recorded (see lib.rs::queue()).
    h.governor.queue(&proposal_id);

    let rep = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(rep.consecutive_successful, 1);
    // 2 voters * 1000 / 10_000 supply = 2000 bps participation: below the
    // 5000 bps high-participation bonus threshold, so just score_for_succeed.
    assert_eq!(rep.reputation_score, 100);
}

#[test]
fn test_score_decrements_on_defeated_proposal() {
    let h = setup(10, 20, 20, 0, 120_960);
    let proposer = Address::generate(&h.env);
    h.votes.set_votes(&proposer, &100_000i128);
    let voter = Address::generate(&h.env);
    h.votes.set_votes(&voter, &1_000i128); // below the 2000 quorum requirement

    let proposal_id = propose_dummy(&h.env, &h.governor, &proposer, b"defeat-1");

    h.env.ledger().with_mut(|l| l.sequence_number = 11);
    h.governor.cast_vote(&voter, &proposal_id, &VoteSupport::For);

    h.env.ledger().with_mut(|l| l.sequence_number = 31);
    assert_eq!(h.governor.state(&proposal_id), ProposalState::Defeated);

    let rep = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(rep.consecutive_failed, 1);
    assert_eq!(rep.reputation_score, -30);
}

#[test]
fn test_score_decrements_on_expired_proposal() {
    // Small grace period so the proposal can lapse into Expired quickly.
    let h = setup(10, 20, 20, 0, 5);
    let proposer = Address::generate(&h.env);
    h.votes.set_votes(&proposer, &100_000i128);
    let voter_a = Address::generate(&h.env);
    let voter_b = Address::generate(&h.env);
    h.votes.set_votes(&voter_a, &1_000i128);
    h.votes.set_votes(&voter_b, &1_000i128);

    let proposal_id = propose_dummy(&h.env, &h.governor, &proposer, b"expire-1");

    h.env.ledger().with_mut(|l| l.sequence_number = 11);
    h.governor.cast_vote(&voter_a, &proposal_id, &VoteSupport::For);
    h.governor.cast_vote(&voter_b, &proposal_id, &VoteSupport::For);

    // end_ledger = 10 + 20 = 30; grace_end = 35. Never queue — advance past
    // the grace period so the proposal lapses from Succeeded into Expired.
    h.env.ledger().with_mut(|l| l.sequence_number = 40);
    assert_eq!(h.governor.state(&proposal_id), ProposalState::Expired);

    let rep = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(rep.consecutive_failed, 1);
    assert_eq!(rep.reputation_score, -60);
}

#[test]
fn test_effective_threshold_reduced_for_high_score_proposer() {
    let h = setup(10, 20, 0, 1_000, 120_960);
    let proposer = Address::generate(&h.env);

    h.env.as_contract(&h.governor_id, || {
        for _ in 0..5 {
            reputation::record_proposal_created(&h.env, &proposer);
            reputation::record_proposal_terminal(
                &h.env,
                &proposer,
                reputation::ReputationOutcome::Succeeded,
                Some(5_000),
            );
        }
    });

    let effective = h.governor.get_effective_threshold(&proposer);
    assert!(effective < 1_000, "expected a discount, got {effective}");
}

#[test]
fn test_effective_threshold_increased_for_low_score_proposer() {
    let h = setup(10, 20, 0, 1_000, 120_960);
    let proposer = Address::generate(&h.env);

    h.env.as_contract(&h.governor_id, || {
        reputation::record_proposal_created(&h.env, &proposer);
        reputation::record_proposal_terminal(
            &h.env,
            &proposer,
            reputation::ReputationOutcome::Defeated,
            None,
        );
    });

    let effective = h.governor.get_effective_threshold(&proposer);
    assert!(effective > 1_000, "expected a penalty, got {effective}");
}

#[test]
fn test_reputation_decay_moves_score_toward_zero() {
    let h = setup(10, 20, 0, 0, 120_960);
    let proposer = Address::generate(&h.env);

    h.env.as_contract(&h.governor_id, || {
        reputation::record_proposal_created(&h.env, &proposer);
        reputation::record_proposal_terminal(
            &h.env,
            &proposer,
            reputation::ReputationOutcome::Executed,
            None,
        );
    });
    let before = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(before.reputation_score, 50);

    h.env.ledger().with_mut(|l| l.sequence_number += 2_000);
    h.governor.apply_reputation_decay(&proposer);

    let after = h.governor.get_proposer_reputation(&proposer);
    assert!(after.reputation_score < before.reputation_score);
    assert!(after.reputation_score >= 0);
}

#[test]
fn test_high_participation_bonus_applied() {
    let h = setup(10, 20, 0, 0, 120_960);
    let low_participation = Address::generate(&h.env);
    let high_participation = Address::generate(&h.env);

    h.env.as_contract(&h.governor_id, || {
        reputation::record_proposal_created(&h.env, &low_participation);
        reputation::record_proposal_terminal(
            &h.env,
            &low_participation,
            reputation::ReputationOutcome::Succeeded,
            Some(1_000), // 10% participation: below the 50% bonus threshold
        );

        reputation::record_proposal_created(&h.env, &high_participation);
        reputation::record_proposal_terminal(
            &h.env,
            &high_participation,
            reputation::ReputationOutcome::Succeeded,
            Some(6_000), // 60% participation: above the 50% bonus threshold
        );
    });

    let low = h.governor.get_proposer_reputation(&low_participation);
    let high = h.governor.get_proposer_reputation(&high_participation);
    assert_eq!(low.reputation_score, 100);
    assert_eq!(high.reputation_score, 130);
}

#[test]
fn test_score_clamped_at_max_and_min() {
    let h = setup(10, 20, 0, 0, 120_960);
    let good_proposer = Address::generate(&h.env);
    let bad_proposer = Address::generate(&h.env);
    let config = reputation::default_config();

    h.env.as_contract(&h.governor_id, || {
        // 21 Executed outcomes (+50 each) sum to 1050, but must clamp at
        // the default max_score of 1000.
        for _ in 0..21 {
            reputation::record_proposal_created(&h.env, &good_proposer);
            reputation::record_proposal_terminal(
                &h.env,
                &good_proposer,
                reputation::ReputationOutcome::Executed,
                None,
            );
        }

        // 17 Expired outcomes (-60 each) sum to -1020, but must clamp at
        // the default min_score of -1000.
        for _ in 0..17 {
            reputation::record_proposal_created(&h.env, &bad_proposer);
            reputation::record_proposal_terminal(
                &h.env,
                &bad_proposer,
                reputation::ReputationOutcome::Expired,
                None,
            );
        }
    });

    let good = h.governor.get_proposer_reputation(&good_proposer);
    let bad = h.governor.get_proposer_reputation(&bad_proposer);
    assert_eq!(good.reputation_score, config.max_score);
    assert_eq!(bad.reputation_score, config.min_score);
}

#[test]
fn test_consecutive_success_streak_tracked() {
    let h = setup(10, 20, 0, 0, 120_960);
    let proposer = Address::generate(&h.env);

    h.env.as_contract(&h.governor_id, || {
        for _ in 0..3 {
            reputation::record_proposal_created(&h.env, &proposer);
            reputation::record_proposal_terminal(
                &h.env,
                &proposer,
                reputation::ReputationOutcome::Succeeded,
                None,
            );
        }
    });
    let after_wins = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(after_wins.consecutive_successful, 3);
    assert_eq!(after_wins.consecutive_failed, 0);

    h.env.as_contract(&h.governor_id, || {
        reputation::record_proposal_created(&h.env, &proposer);
        reputation::record_proposal_terminal(
            &h.env,
            &proposer,
            reputation::ReputationOutcome::Defeated,
            None,
        );
    });
    let after_defeat = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(after_defeat.consecutive_successful, 0);
    assert_eq!(after_defeat.consecutive_failed, 1);
}

#[test]
fn test_threshold_multiplier_computed_correctly_at_extremes() {
    let config = reputation::default_config();
    let at_max_score = reputation::compute_threshold_multiplier(config.max_score, &config);
    let at_min_score = reputation::compute_threshold_multiplier(config.min_score, &config);
    assert_eq!(at_max_score, config.min_threshold_multiplier_bps);
    assert_eq!(at_min_score, config.max_threshold_multiplier_bps);
}

#[test]
fn test_new_proposer_has_baseline_multiplier() {
    let h = setup(10, 20, 0, 1_000, 120_960);
    let proposer = Address::generate(&h.env);

    let rep = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(rep.total_proposals, 0);
    assert_eq!(rep.threshold_multiplier_bps, 10_000);

    let effective = h.governor.get_effective_threshold(&proposer);
    assert_eq!(effective, 1_000);
}

#[test]
fn test_propose_uses_effective_threshold_not_flat_threshold() {
    let h = setup(10, 20, 0, 1_000, 120_960);
    let proposer = Address::generate(&h.env);

    h.env.as_contract(&h.governor_id, || {
        for _ in 0..5 {
            reputation::record_proposal_created(&h.env, &proposer);
            reputation::record_proposal_terminal(
                &h.env,
                &proposer,
                reputation::ReputationOutcome::Succeeded,
                Some(5_000),
            );
        }
    });

    let effective = h.governor.get_effective_threshold(&proposer);
    assert!(effective < 1_000);

    // Voting power strictly between the discounted effective threshold and
    // the flat threshold: only the reputation-adjusted check lets this pass.
    let voting_power = (effective + 1_000) / 2;
    assert!(voting_power < 1_000);
    assert!(voting_power > effective);
    h.votes.set_votes(&proposer, &voting_power);

    let proposal_id = propose_dummy(&h.env, &h.governor, &proposer, b"discounted-propose");
    assert_eq!(h.governor.state(&proposal_id), ProposalState::Pending);

    // A newcomer with the same voting power but no reputation history must
    // still be rejected by the flat threshold.
    let newcomer = Address::generate(&h.env);
    h.votes.set_votes(&newcomer, &voting_power);
    let result = h.governor.try_propose(
        &newcomer,
        &String::from_str(&h.env, "d"),
        &h.env.crypto().sha256(&Bytes::new(&h.env)).into(),
        &String::from_str(&h.env, "u"),
        &soroban_sdk::vec![&h.env, Address::generate(&h.env)],
        &soroban_sdk::vec![&h.env, Symbol::new(&h.env, "test")],
        &soroban_sdk::vec![&h.env, Bytes::new(&h.env)],
    );
    assert!(result.is_err());
}

#[test]
fn test_queue_then_cancel_queued_applies_cancelled_penalty() {
    // min_delay = 500s => veto window of 100 ledgers after queue().
    let h = setup_with_min_delay(10, 20, 20, 0, 120_960, 500);
    let proposer = Address::generate(&h.env);
    h.votes.set_votes(&proposer, &100_000i128);
    let voter_a = Address::generate(&h.env);
    let voter_b = Address::generate(&h.env);
    h.votes.set_votes(&voter_a, &1_000i128);
    h.votes.set_votes(&voter_b, &1_000i128);

    let proposal_id = propose_dummy(&h.env, &h.governor, &proposer, b"queue-cancel-1");

    h.env.ledger().with_mut(|l| l.sequence_number = 11);
    h.governor.cast_vote(&voter_a, &proposal_id, &VoteSupport::For);
    h.governor.cast_vote(&voter_b, &proposal_id, &VoteSupport::For);

    h.env.ledger().with_mut(|l| l.sequence_number = 31);
    h.governor.queue(&proposal_id);

    // Still inside the veto window (queued at ledger 31, window ends at 131).
    h.env.ledger().with_mut(|l| l.sequence_number = 40);
    h.governor.cancel_queued(&h.guardian, &proposal_id);

    // queue() recorded Succeeded (+100); cancel_queued() adds Cancelled (-20).
    let rep = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(rep.reputation_score, 80);
    assert_eq!(rep.consecutive_failed, 1);
    assert_eq!(rep.consecutive_successful, 0);
}

#[test]
fn test_cancel_by_governance_after_defeat_penalises_only_once() {
    let h = setup(10, 20, 20, 0, 120_960);
    let proposer = Address::generate(&h.env);
    h.votes.set_votes(&proposer, &100_000i128);
    let voter = Address::generate(&h.env);
    h.votes.set_votes(&voter, &1_000i128); // below the 2000 quorum requirement

    let proposal_id = propose_dummy(&h.env, &h.governor, &proposer, b"defeat-then-gov-cancel");

    h.env.ledger().with_mut(|l| l.sequence_number = 11);
    h.governor.cast_vote(&voter, &proposal_id, &VoteSupport::For);

    // Natural outcome: Defeated (-30), recorded once when state() is read.
    h.env.ledger().with_mut(|l| l.sequence_number = 31);
    assert_eq!(h.governor.state(&proposal_id), ProposalState::Defeated);

    // Governance then cancels the already-defeated proposal. It must not
    // apply a second penalty on top of the Defeated one.
    h.governor.cancel_by_governance(&proposal_id);

    let rep = h.governor.get_proposer_reputation(&proposer);
    assert_eq!(rep.reputation_score, -30);
    assert_eq!(rep.consecutive_failed, 1);
}

#[test]
fn test_self_cancel_pending_scored_same_as_guardian_veto() {
    // Documents the current behaviour: a proposer withdrawing their own
    // Pending proposal and a guardian veto of an Active proposal both record
    // the `Cancelled` outcome (-20). Separating the two is a scoring-policy
    // decision; this test pins what ships today so a change is deliberate.
    let h = setup(10, 20, 20, 0, 120_960);

    let self_cancelled = Address::generate(&h.env);
    h.votes.set_votes(&self_cancelled, &100_000i128);
    let pending_id = propose_dummy(&h.env, &h.governor, &self_cancelled, b"self-cancel-pending");
    // start_ledger = 0 + voting_delay(10) => still Pending at ledger 0.
    h.governor.cancel(&self_cancelled, &pending_id);

    let rep = h.governor.get_proposer_reputation(&self_cancelled);
    assert_eq!(rep.reputation_score, -20);
    assert_eq!(rep.consecutive_failed, 1);

    let vetoed = Address::generate(&h.env);
    h.votes.set_votes(&vetoed, &100_000i128);
    let active_id = propose_dummy(&h.env, &h.governor, &vetoed, b"guardian-veto-active");
    h.env.ledger().with_mut(|l| l.sequence_number = 11);
    h.governor.cancel(&h.guardian, &active_id);

    let rep = h.governor.get_proposer_reputation(&vetoed);
    assert_eq!(rep.reputation_score, -20);
    assert_eq!(rep.consecutive_failed, 1);
}
