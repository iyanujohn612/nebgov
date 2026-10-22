//! Drives a real governor/timelock/token-votes/treasury stack (registered
//! natively in an in-process [`Env`], not compiled to WASM) through a
//! [`Scenario`]'s steps, recording a [`SimulationReport`].
//!
//! Contract calls that are expected to fail (e.g. a double-vote, or a
//! proposal below quorum being queued) panic inside the Soroban test host
//! rather than returning a `Result` — the runner wraps each step in
//! `catch_unwind` to turn that into a `StepResult { success: false, .. }`
//! instead of aborting the whole simulation.
//!
//! The runner supports both governor-level lifecycle steps (`Propose`,
//! `Vote`, `Queue`, `Execute`, etc.) and direct timelock operations
//! (`ScheduleBatch`, `ExecuteBatch`, `ValidateDag`) for testing DAG
//! dependency features.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{
    token, Address, Bytes, BytesN, Env, IntoVal, String as SorobanString, Symbol, Val,
    Vec as SorobanVec,
};

use sorogov_co_sponsorship::{CoSponsorshipContract, CoSponsorshipContractClient};
use sorogov_conviction_voting::{ConvictionVotingContract, ConvictionVotingContractClient};
use sorogov_governor::{
    GovernorContract, GovernorContractClient, GovernorSettings, ProposalState, VoteSupport,
    VoteType,
};
use sorogov_liquidity::{LiquidityContract, LiquidityContractClient, Pool};
use sorogov_proposal_bonds::{BondState, ProposalBondsContract, ProposalBondsContractClient};
use sorogov_signal_anchor::{SignalAnchorContract, SignalAnchorContractClient};
use sorogov_timelock::{TimelockContract, TimelockContractClient};
use sorogov_token_votes::{TokenVotesContract, TokenVotesContractClient};
use sorogov_treasury::{TreasuryContract, TreasuryContractClient};
use sorogov_treasury_strategies::adapters::mock_adapter::MockAdapter;
use sorogov_treasury_strategies::{TreasuryStrategiesContract, TreasuryStrategiesContractClient};
use sorogov_vote_escrow::{VoteEscrowContract, VoteEscrowContractClient};
use sorogov_voting_rewards::{merkle, VotingRewardsContract, VotingRewardsContractClient};

use crate::report::{SimulationReport, StepResult};
use crate::scenario::{
    ActorRole, Scenario, SimBondState, SimGovernorSettings, SimPoolAsset, SimProposalState,
    SimStep, SimVoteSupport, SimVoteType,
};

/// A no-op target contract used to resolve `SimStep::Propose` targets that
/// don't name a special contract (`"treasury"`, `"governor"`) — this tool's
/// job is to exercise the governor's proposal lifecycle and rate limits,
/// not to be a general arbitrary-calldata executor (that's already covered
/// by `contracts/governor/src/tests/integration.rs`). Scenario JSON should
/// use `fn_names: ["noop"]` when targeting an arbitrary/placeholder
/// contract.
#[soroban_sdk::contract]
pub struct SimTargetContract;

#[soroban_sdk::contractimpl]
impl SimTargetContract {
    pub fn noop(_env: Env) {}
}

/// Must match `GovernorContract::SECONDS_PER_LEDGER` in
/// `contracts/governor/src/lib.rs`.
const SECONDS_PER_LEDGER: u64 = 5;

/// Outcome ids of the harness's single liquidity pool: asset A is outcome 0
/// (the pool's `reserve_a`), asset B is outcome 1 (`reserve_b`).
const POOL_OUTCOME_A: u32 = 0;
const POOL_OUTCOME_B: u32 = 1;

/// Voting-rewards epoch length. Epoch 0 opens at genesis (ledger 1), so it
/// covers ledgers 1..21, epoch 1 covers 21..41, and so on.
const REWARDS_EPOCH_LEDGERS: u32 = 20;

pub struct SimulationRunner {
    env: Env,
    scenario: Scenario,
    governor: GovernorContractClient<'static>,
    timelock: TimelockContractClient<'static>,
    token_votes: TokenVotesContractClient<'static>,
    #[allow(dead_code)]
    treasury: TreasuryContractClient<'static>,
    #[allow(dead_code)]
    co_sponsorship: CoSponsorshipContractClient<'static>,
    #[allow(dead_code)]
    conviction_voting: ConvictionVotingContractClient<'static>,
    proposal_bonds: ProposalBondsContractClient<'static>,
    vote_escrow: VoteEscrowContractClient<'static>,
    signal_anchor: SignalAnchorContractClient<'static>,
    liquidity: LiquidityContractClient<'static>,
    pool_token_a: Address,
    pool_token_b: Address,
    /// Pool state captured just before the most recent `AddLiquidity`,
    /// `RemoveLiquidity` or `Swap`, compared against by `ExpectPoolInvariant`.
    pool_before_last_change: Option<Pool>,
    voting_rewards: VotingRewardsContractClient<'static>,
    reward_token: Address,
    /// Each successfully published epoch's `(claimant, amount)` leaves, in
    /// tree order, so `ClaimReward` can rebuild a claimant's proof.
    reward_allocations: HashMap<u64, Vec<(Address, i128)>>,
    token: Address,
    target: Address,
    treasury_addr: Address,
    treasury_strategies: TreasuryStrategiesContractClient<'static>,
    treasury_strategies_admin: Address,
    strategy_adapter: Address,
    proposal_bonds_addr: Address,
    actors: HashMap<String, Address>,
    report: SimulationReport,
    batch_op_ids: Vec<Option<soroban_sdk::Bytes>>,
}

impl SimulationRunner {
    pub fn new(scenario: &Scenario) -> Self {
        let env = Env::default();
        // Governance execution can authorize a nested governor → timelock →
        // target call. Match the repository's cross-contract integration
        // tests by allowing mocked authorization below the root invocation.
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_sequence_number(1);

        // Pick the on-chain admin/pauser: prefer an actor explicitly cast as
        // Admin, then Pauser (initialize() makes admin the initial pauser),
        // else synthesize one not tied to any named actor.
        let admin_actor = scenario
            .actors
            .iter()
            .find(|a| a.role == ActorRole::Admin)
            .or_else(|| scenario.actors.iter().find(|a| a.role == ActorRole::Pauser));

        let mut actors: HashMap<String, Address> = HashMap::new();
        for actor in &scenario.actors {
            actors.insert(actor.name.clone(), Address::generate(&env));
        }
        let admin = admin_actor
            .map(|a| actors[&a.name].clone())
            .unwrap_or_else(|| Address::generate(&env));

        let guardian_actor = scenario
            .actors
            .iter()
            .find(|a| a.role == ActorRole::Guardian);
        let guardian = guardian_actor
            .map(|a| actors[&a.name].clone())
            .unwrap_or_else(|| admin.clone());

        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        let token_votes_id = env.register(TokenVotesContract, ());
        let token_votes = TokenVotesContractClient::new(&env, &token_votes_id);
        token_votes.initialize(&admin, &token);

        let governor_id = env.register(GovernorContract, ());
        let timelock_id = env.register(TimelockContract, ());

        let timelock = TimelockContractClient::new(&env, &timelock_id);
        timelock.initialize(&admin, &governor_id, &1u64, &60u64);

        let governor = GovernorContractClient::new(&env, &governor_id);
        governor.initialize(
            &admin,
            &token_votes_id,
            &timelock_id,
            &scenario.governor_settings.voting_delay,
            &scenario.governor_settings.voting_period,
            &scenario.governor_settings.quorum_numerator,
            &scenario.governor_settings.proposal_threshold,
            &guardian,
            &sim_vote_type(scenario.governor_settings.vote_type),
            &scenario.governor_settings.proposal_grace_period,
        );
        governor.set_initial_config(
            &admin,
            &scenario.governor_settings.max_calldata_size,
            &scenario.governor_settings.proposal_cooldown,
            &scenario.governor_settings.max_proposals_per_period,
        );

        let treasury_owners = SorobanVec::from_array(&env, [admin.clone()]);
        let treasury_id = env.register(TreasuryContract, ());
        let treasury = TreasuryContractClient::new(&env, &treasury_id);
        treasury.initialize(&treasury_owners, &1u32, &governor_id);

        // Strategies are admin-registered and treasury-funded, so the admin
        // here is the harness admin and the treasury is the multisig above.
        let treasury_strategies_id = env.register(TreasuryStrategiesContract, ());
        let treasury_strategies =
            TreasuryStrategiesContractClient::new(&env, &treasury_strategies_id);
        treasury_strategies.initialize(&admin, &treasury_id);
        let strategy_adapter = env.register(MockAdapter, ());

        let co_sponsorship_id = env.register(CoSponsorshipContract, ());
        let co_sponsorship = CoSponsorshipContractClient::new(&env, &co_sponsorship_id);
        co_sponsorship.initialize(
            &admin,
            &governor_id,
            &token_votes_id,
            &7200u32, // draft expiry: 1 hour (12 ledgers of 5s each)
            &10u32,   // max co-sponsors: 10
        );
        let mut governor_settings = governor.get_settings();
        governor_settings.co_sponsorship_registry = Some(co_sponsorship_id);
        governor.update_config(&governor_settings);

        let conviction_voting_id = env.register(ConvictionVotingContract, ());
        let conviction_voting = ConvictionVotingContractClient::new(&env, &conviction_voting_id);
        conviction_voting.initialize(
            &admin,
            &token_votes_id,
            &5000u32,        // decay_bps: 50%
            &5000u32,        // max_ratio_bps: 50%
            &10_000_000i128, // min_threshold_conviction
            &100u32,         // weight_bps: 1%
        );

        let proposal_bonds_id = env.register(ProposalBondsContract, ());
        let proposal_bonds = ProposalBondsContractClient::new(&env, &proposal_bonds_id);
        proposal_bonds.initialize(&admin, &token, &100i128, &governor_id, &2u32, &10_000u32);

        let vote_escrow_id = env.register(VoteEscrowContract, ());
        let vote_escrow = VoteEscrowContractClient::new(&env, &vote_escrow_id);
        vote_escrow.initialize(
            &admin,
            &token,
            &100u32,  // min_lock_duration
            &1100u32, // max_lock_duration
            &10_000u32, // max_multiplier_bps: 100% boost (2x) at max duration
        );

        let signal_anchor_id = env.register(SignalAnchorContract, ());
        let signal_anchor = SignalAnchorContractClient::new(&env, &signal_anchor_id);
        signal_anchor.initialize(&admin);

        let liquidity_id = env.register(LiquidityContract, ());
        let liquidity = LiquidityContractClient::new(&env, &liquidity_id);
        liquidity.initialize(&governor_id);
        let pool_token_a = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let pool_token_b = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        // The governor is the admin, matching the intended deployment where
        // publishing a root is itself a governance-executed action.
        let reward_token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let voting_rewards_id = env.register(VotingRewardsContract, ());
        let voting_rewards = VotingRewardsContractClient::new(&env, &voting_rewards_id);
        voting_rewards.initialize(&governor_id, &reward_token, &REWARDS_EPOCH_LEDGERS);

        let target = env.register(SimTargetContract, ());

        let sac = token::StellarAssetClient::new(&env, &token);
        for actor in &scenario.actors {
            let addr = &actors[&actor.name];
            if actor.initial_balance > 0 {
                sac.mint(addr, &actor.initial_balance);
            }
            let delegatee = actor
                .delegate_to
                .as_ref()
                .map(|d| actors[d].clone())
                .unwrap_or_else(|| addr.clone());
            token_votes.delegate(addr, &delegatee);
        }

        let report = SimulationReport::new(scenario.name.clone());

        Self {
            env,
            scenario: scenario.clone(),
            governor,
            timelock,
            token_votes,
            treasury,
            co_sponsorship,
            conviction_voting,
            proposal_bonds,
            vote_escrow,
            signal_anchor,
            liquidity,
            pool_token_a,
            pool_token_b,
            pool_before_last_change: None,
            voting_rewards,
            reward_token,
            reward_allocations: HashMap::new(),
            token,
            target,
            treasury_addr: treasury_id,
            treasury_strategies,
            treasury_strategies_admin: admin,
            strategy_adapter,
            proposal_bonds_addr: proposal_bonds_id,
            actors,
            report,
            batch_op_ids: Vec::new(),
        }
    }

    pub fn get_actor(&self, name: &str) -> &Address {
        self.actors
            .get(name)
            .unwrap_or_else(|| panic!("unknown actor '{}'", name))
    }

    pub fn get_proposal_state(&self, proposal_id: u64) -> SimProposalState {
        from_proposal_state(self.governor.state(&proposal_id))
    }

    /// Advance both ledger sequence and timestamp to stay consistent with
    /// `SECONDS_PER_LEDGER` in `contracts/governor/src/lib.rs` — the
    /// timelock's readiness check is timestamp-based, so advancing sequence
    /// alone would leave queued proposals permanently un-executable.
    pub fn advance_to_ledger(&mut self, target: u32) {
        let current = self.env.ledger().sequence();
        if target > current {
            let delta_seconds = (target - current) as u64 * SECONDS_PER_LEDGER;
            self.env.ledger().set_sequence_number(target);
            self.env
                .ledger()
                .set_timestamp(self.env.ledger().timestamp() + delta_seconds);
        }
    }

    /// Resolve a `SimStep::Propose` target string to a real deployed
    /// address: `"treasury"` / `"governor"` name the corresponding
    /// contract, `"target"` (or any other unrecognized name) resolves to
    /// the harness's [`SimTargetContract`], and anything matching a
    /// declared actor resolves to that actor's address.
    fn resolve_target(&self, name: &str) -> Address {
        match name {
            "treasury" => self.treasury_addr.clone(),
            "governor" => self.governor.address.clone(),
            "proposal-bonds" => self.proposal_bonds_addr.clone(),
            "target" => self.target.clone(),
            other => self
                .actors
                .get(other)
                .cloned()
                .unwrap_or_else(|| self.target.clone()),
        }
    }

    /// Run every step in the scenario in order, returning the final report.
    /// Individual step failures are recorded, not propagated — a scenario
    /// documenting an expected failure (via a following `ExpectError` step)
    /// should keep running.
    pub fn run(&mut self) -> SimulationReport {
        self.batch_op_ids = vec![None; self.scenario.steps.len()];
        for i in 0..self.scenario.steps.len() {
            let step = self.scenario.steps[i].clone();
            let result = self.run_step_indexed(i, &step);
            self.report.record(result);
        }
        self.report.final_ledger = self.env.ledger().sequence();
        self.report.clone()
    }

    /// Run a single step outside of [`Self::run`]'s scenario loop — part of
    /// the granular step-by-step control surface (used by tests that need
    /// to assert on intermediate state between steps; `cargo clippy`'s
    /// non-test bin target can't see those `#[cfg(test)]` call sites).
    #[allow(dead_code)]
    pub fn run_step(&mut self, step: &crate::scenario::SimStep) -> StepResult {
        self.run_step_indexed(self.report.total_steps, step)
    }

    fn run_step_indexed(&mut self, index: usize, step: &SimStep) -> StepResult {
        self.env.cost_estimate().budget().reset_default();
        let start = Instant::now();

        // Ensure batch_op_ids is large enough for this step
        if index >= self.batch_op_ids.len() {
            self.batch_op_ids.resize_with(index + 1, || None);
        }

        let outcome: Result<(), String> = catch_unwind(AssertUnwindSafe(|| {
            self.dispatch(step, index);
        }))
        .map_err(panic_message);

        let duration_ms = start.elapsed().as_millis() as u64;
        let cpu_insns = self.env.cost_estimate().budget().cpu_instruction_cost();
        let mem_bytes = self.env.cost_estimate().budget().memory_bytes_cost();

        let success = outcome.is_ok();
        let error = outcome.err();

        StepResult {
            step_index: index,
            step_type: step.kind().to_string(),
            ledger: self.env.ledger().sequence(),
            success,
            error,
            cpu_insns,
            mem_bytes,
            // The soroban-sdk test host doesn't expose per-call storage
            // read/write counts directly; a rough per-step-kind estimate is
            // used here as a placeholder signal for storage_warnings rather
            // than a precise count.
            storage_entries_read: estimated_storage_touches(step).0,
            storage_entries_written: estimated_storage_touches(step).1,
            duration_ms,
            anticipated_failure: false,
        }
    }

    fn dispatch(&mut self, step: &SimStep, step_index: usize) {
        match step {
            SimStep::AdvanceLedger { ledgers } => {
                let target = self.env.ledger().sequence() + ledgers;
                self.advance_to_ledger(target);
            }
            SimStep::Propose {
                actor,
                targets,
                fn_names,
                description,
            } => {
                let proposer = self.get_actor(actor).clone();
                let targets: SorobanVec<Address> = {
                    let mut v = SorobanVec::new(&self.env);
                    for t in targets {
                        v.push_back(self.resolve_target(t));
                    }
                    v
                };
                let fn_names_vec: SorobanVec<Symbol> = {
                    let mut v = SorobanVec::new(&self.env);
                    for f in fn_names {
                        v.push_back(Symbol::new(&self.env, f));
                    }
                    v
                };
                let mut calldatas: SorobanVec<Bytes> = SorobanVec::new(&self.env);
                for _ in 0..targets.len() {
                    calldatas.push_back(Bytes::new(&self.env));
                }
                let desc = SorobanString::from_str(&self.env, description);
                let desc_hash = description_hash(&self.env, description);
                let metadata_uri = SorobanString::from_str(&self.env, "");
                self.governor.propose(
                    &proposer,
                    &desc,
                    &desc_hash,
                    &metadata_uri,
                    &targets,
                    &fn_names_vec,
                    &calldatas,
                );
            }
            SimStep::Vote {
                actor,
                proposal_id,
                support,
            } => {
                let voter = self.get_actor(actor).clone();
                self.governor
                    .cast_vote(&voter, proposal_id, &sim_vote_support(*support));
                self.report.total_votes_cast += 1;
            }
            SimStep::Queue { proposal_id, .. } => {
                self.governor.queue(proposal_id);
            }
            SimStep::Execute { proposal_id, .. } => {
                self.governor.execute(proposal_id);
                self.report.proposals_executed += 1;
            }
            SimStep::Cancel { actor, proposal_id } => {
                let caller = self.get_actor(actor).clone();
                self.governor.cancel(&caller, proposal_id);
            }
            SimStep::Delegate { actor, delegatee } => {
                let delegator = self.get_actor(actor).clone();
                let delegatee_addr = self.get_actor(delegatee).clone();
                self.token_votes.delegate(&delegator, &delegatee_addr);
            }
            SimStep::DelegateSplit { actor, splits } => {
                let delegator = self.get_actor(actor).clone();
                let split_delegations: SorobanVec<sorogov_token_votes::SplitDelegation> =
                    SorobanVec::from_slice(
                        &self.env,
                        &splits
                            .iter()
                            .map(|(delegatee_name, weight_bps)| {
                                sorogov_token_votes::SplitDelegation {
                                    delegatee: self.get_actor(delegatee_name).clone(),
                                    weight_bps: *weight_bps,
                                }
                            })
                            .collect::<Vec<_>>(),
                    );
                self.token_votes.delegate_split(&delegator, &split_delegations);
            }
            SimStep::MintTokens { actor, amount } => {
                let addr = self.get_actor(actor).clone();
                token::StellarAssetClient::new(&self.env, &self.token)
                    .mint(&addr, &(*amount as i128));
                let balance = token::TokenClient::new(&self.env, &self.token).balance(&addr);
                self.token_votes.checkpoint(&addr, &balance);
            }
            SimStep::BurnTokens { actor, amount } => {
                let addr = self.get_actor(actor).clone();
                token::StellarAssetClient::new(&self.env, &self.token)
                    .clawback(&addr, &(*amount as i128));
                let balance = token::TokenClient::new(&self.env, &self.token).balance(&addr);
                self.token_votes.checkpoint(&addr, &balance);
            }
            SimStep::UpdateConfig { settings, .. } => {
                // `update_config` requires `env.current_contract_address().require_auth()`
                // — satisfied automatically by `env.mock_all_auths()` from setup.
                let current = self.governor.get_settings();
                let new_settings = merge_settings(&current, settings);
                self.governor.update_config(&new_settings);
            }
            SimStep::PauseContract { actor } => {
                let caller = self.get_actor(actor).clone();
                self.governor.pause(&caller);
            }
            SimStep::UnpauseContract { actor } => {
                let caller = self.get_actor(actor).clone();
                self.governor.unpause(&caller);
            }
            SimStep::ExpectState {
                proposal_id,
                expected_state,
            } => {
                let actual = self.get_proposal_state(*proposal_id);
                if actual != *expected_state {
                    panic!(
                        "expected proposal #{} to be {:?}, was {:?}",
                        proposal_id, expected_state, actual
                    );
                }
            }
            SimStep::ExpectError { .. } => {
                // Evaluated post-hoc by the caller (see `check_expect_errors`)
                // against the recorded result of `step_index`, since it
                // needs to inspect a *different* step's outcome rather than
                // perform an action of its own.
            }
            SimStep::TakeAnalyticsSnapshot => {
                self.report.proposals_created = self.governor.proposal_count();
                // Exercises `proposals_count_by_state()`'s O(proposal_count)
                // iteration so its cost shows up in this step's recorded
                // cpu_insns — see `scenarios/compute_stress.json`.
                let counts = self.governor.proposals_count_by_state();
                self.report.proposals_executed = counts.executed;
                self.report.proposals_defeated = counts.defeated;
            }
            SimStep::AssertParticipation {
                proposal_id,
                min_bps,
            } => {
                let proposal = self.governor.get_proposal(proposal_id);
                let total_cast =
                    proposal.votes_for + proposal.votes_against + proposal.votes_abstain;
                let supply = self
                    .token_votes
                    .get_past_total_supply(&proposal.start_ledger);
                let bps = if supply > 0 {
                    (total_cast.saturating_mul(10_000) / supply) as u32
                } else {
                    0
                };
                if bps < *min_bps {
                    panic!(
                        "participation {}bps below minimum {}bps for proposal #{}",
                        bps, min_bps, proposal_id
                    );
                }
            }
            SimStep::AssertQuorumReached { proposal_id } => {
                if !self.governor.is_quorum_reached(proposal_id) {
                    panic!("quorum not reached for proposal #{}", proposal_id);
                }
            }
            SimStep::AssertReputationScore {
                actor,
                min_score,
                max_score,
            } => {
                let proposer = self.get_actor(actor).clone();
                let rep = self.governor.get_proposer_reputation(&proposer);
                if let Some(min) = min_score {
                    if rep.reputation_score < *min {
                        panic!(
                            "reputation score {} below minimum {} for actor {}",
                            rep.reputation_score, min, actor
                        );
                    }
                }
                if let Some(max) = max_score {
                    if rep.reputation_score > *max {
                        panic!(
                            "reputation score {} above maximum {} for actor {}",
                            rep.reputation_score, max, actor
                        );
                    }
                }
            }
            SimStep::CreateDraft {
                actor,
                targets,
                fn_names,
                description,
            } => {
                let creator = self.get_actor(actor).clone();
                let targets_vec: SorobanVec<Address> = {
                    let mut v = SorobanVec::new(&self.env);
                    for t in targets {
                        v.push_back(self.resolve_target(t));
                    }
                    v
                };
                let fn_names_vec: SorobanVec<Symbol> = {
                    let mut v = SorobanVec::new(&self.env);
                    for f in fn_names {
                        v.push_back(Symbol::new(&self.env, f));
                    }
                    v
                };
                let mut calldatas: SorobanVec<Bytes> = SorobanVec::new(&self.env);
                for _ in 0..targets_vec.len() {
                    calldatas.push_back(Bytes::new(&self.env));
                }
                let desc = SorobanString::from_str(&self.env, description);
                let desc_hash = self.env.crypto().sha256(&Bytes::new(&self.env)).into();
                let metadata_uri = SorobanString::from_str(&self.env, "");
                self.co_sponsorship.create_draft(
                    &creator,
                    &desc,
                    &desc_hash,
                    &metadata_uri,
                    &targets_vec,
                    &fn_names_vec,
                    &calldatas,
                );
            }
            SimStep::CoSponsorDraft { actor, draft_id } => {
                let co_sponsor = self.get_actor(actor).clone();
                self.co_sponsorship.co_sponsor(&co_sponsor, draft_id);
            }
            SimStep::FinalizeDraft { actor, draft_id } => {
                let caller = self.get_actor(actor).clone();
                self.co_sponsorship.finalize_draft(&caller, draft_id);
            }
            SimStep::CancelDraft { actor, draft_id } => {
                let caller = self.get_actor(actor).clone();
                self.co_sponsorship.cancel_draft(&caller, draft_id);
            }
            SimStep::ScheduleBatch {
                actor: _,
                targets,
                fn_names,
                delay,
            } => {
                let gov_addr = self.governor.address.clone();
                let targets_vec: SorobanVec<Address> = {
                    let mut v = SorobanVec::new(&self.env);
                    for t in targets {
                        v.push_back(self.resolve_target(t));
                    }
                    v
                };
                let fn_names_vec: SorobanVec<Symbol> = {
                    let mut v = SorobanVec::new(&self.env);
                    for f in fn_names {
                        v.push_back(Symbol::new(&self.env, f));
                    }
                    v
                };
                let calldatas: SorobanVec<Bytes> = {
                    let mut v = SorobanVec::new(&self.env);
                    for _ in 0..targets_vec.len() {
                        v.push_back(Bytes::new(&self.env));
                    }
                    v
                };
                let empty_bytes = Bytes::new(&self.env);
                let batch_op_id = self.timelock.schedule_batch(
                    &gov_addr,
                    &targets_vec,
                    &calldatas,
                    &fn_names_vec,
                    delay,
                    &empty_bytes,
                    &empty_bytes,
                );
                if step_index < self.batch_op_ids.len() {
                    self.batch_op_ids[step_index] = Some(batch_op_id);
                }
            }
            SimStep::ExecuteBatch {
                actor: _,
                schedule_step_index,
            } => {
                let gov_addr = self.governor.address.clone();
                let batch_op_id = self
                    .batch_op_ids
                    .get(*schedule_step_index)
                    .and_then(|opt| opt.clone())
                    .unwrap_or_else(|| {
                        panic!(
                            "step {} references schedule_step_index {} which has no batch_op_id",
                            step_index, schedule_step_index
                        )
                    });
                self.timelock.execute_batch(&gov_addr, &batch_op_id);
            }
            SimStep::ValidateDag {
                schedule_step_indices,
            } => {
                let op_ids: SorobanVec<Bytes> = {
                    let mut v = SorobanVec::new(&self.env);
                    for idx in schedule_step_indices {
                        let batch_op_id = self
                            .batch_op_ids
                            .get(*idx)
                            .and_then(|opt| opt.clone())
                            .unwrap_or_else(|| {
                                panic!(
                                    "ValidateDag step {} references schedule_step_index {} which has no batch_op_id",
                                    step_index, idx
                                )
                            });
                        v.push_back(batch_op_id);
                    }
                    v
                };
                let result = self.timelock.validate_dependency_dag(&op_ids);
                if !result.valid {
                    panic!(
                        "dependency DAG validation failed: cycle detected in {:?}",
                        result.cycle_path
                    );
                }
            }
            SimStep::ConvictionCreateProposal {
                actor,
                target,
                fn_name,
                calldata,
                requested_amount,
            } => {
                let proposer = self.get_actor(actor).clone();
                let target_addr = self.resolve_target(target);
                let fn_symbol = Symbol::new(&self.env, fn_name);
                let calldata_bytes = if let Some(cd) = calldata {
                    Bytes::from_slice(&self.env, cd.as_bytes())
                } else {
                    Bytes::new(&self.env)
                };
                self.conviction_voting.create_proposal(
                    &proposer,
                    &target_addr,
                    &fn_symbol,
                    &calldata_bytes,
                    &(*requested_amount as i128),
                );
            }
            SimStep::ConvictionStake {
                actor,
                proposal_id,
                amount,
            } => {
                let staker = self.get_actor(actor).clone();
                self.conviction_voting
                    .stake(&staker, proposal_id, &(*amount as i128));
            }
            SimStep::ConvictionWithdrawStake { actor } => {
                let staker = self.get_actor(actor).clone();
                self.conviction_voting.withdraw_stake(&staker);
            }
            SimStep::ConvictionCheckpoint { proposal_id } => {
                self.conviction_voting.checkpoint_conviction(proposal_id);
            }
            SimStep::LockProposalBond { actor, description } => {
                let proposer = self.get_actor(actor).clone();
                let hash = description_hash(&self.env, description);
                self.proposal_bonds.lock_bond(&proposer, &hash);
            }
            SimStep::RefundProposalBond {
                actor,
                description,
                proposal_id,
            } => {
                let caller = self.get_actor(actor).clone();
                let hash = description_hash(&self.env, description);
                self.proposal_bonds.refund_bond(&caller, &hash, proposal_id);
            }
            SimStep::ProposeBondSlash {
                actor,
                bonded_description,
                recipient,
                description,
            } => {
                let proposer = self.get_actor(actor).clone();
                let recipient = self.get_actor(recipient).clone();
                let bonded_hash = description_hash(&self.env, bonded_description);
                let mut args: SorobanVec<Val> = SorobanVec::new(&self.env);
                args.push_back(self.governor.address.clone().into_val(&self.env));
                args.push_back(bonded_hash.into_val(&self.env));
                args.push_back(recipient.into_val(&self.env));

                let targets = SorobanVec::from_array(&self.env, [self.proposal_bonds_addr.clone()]);
                let fn_names = SorobanVec::from_array(&self.env, [Symbol::new(&self.env, "slash")]);
                let calldatas = SorobanVec::from_array(&self.env, [args.to_xdr(&self.env)]);
                let description_hash = description_hash(&self.env, description);
                let description = SorobanString::from_str(&self.env, description);
                let metadata_uri = SorobanString::from_str(&self.env, "");
                self.governor.propose(
                    &proposer,
                    &description,
                    &description_hash,
                    &metadata_uri,
                    &targets,
                    &fn_names,
                    &calldatas,
                );
            }
            SimStep::ExpectBondState {
                description,
                expected_state,
            } => {
                let hash = description_hash(&self.env, description);
                let actual = self
                    .proposal_bonds
                    .get_bond(&hash)
                    .unwrap_or_else(|| panic!("no bond for description '{}'", description))
                    .state;
                let expected = match expected_state {
                    SimBondState::Locked => BondState::Locked,
                    SimBondState::Refunded => BondState::Refunded,
                    SimBondState::Slashed => BondState::Slashed,
                };
                if actual != expected {
                    panic!(
                        "expected bond for '{}' to be {:?}, was {:?}",
                        description, expected, actual
                    );
                }
            }
            SimStep::CreateVoteEscrowLock {
                actor,
                amount,
                duration_ledgers,
            } => {
                let owner = self.get_actor(actor).clone();
                self.vote_escrow
                    .create_lock(&owner, &(*amount as i128), duration_ledgers);
            }
            SimStep::IncreaseVoteEscrowLock {
                actor,
                additional_amount,
            } => {
                let owner = self.get_actor(actor).clone();
                self.vote_escrow
                    .increase_lock_amount(&owner, &(*additional_amount as i128));
            }
            SimStep::ExtendVoteEscrowLock {
                actor,
                new_end_ledger,
            } => {
                let owner = self.get_actor(actor).clone();
                self.vote_escrow.extend_lock(&owner, new_end_ledger);
            }
            SimStep::WithdrawVoteEscrowLock { actor } => {
                let owner = self.get_actor(actor).clone();
                self.vote_escrow.withdraw(&owner);
            }
            SimStep::ExpectVotingPower {
                actor,
                expected_power,
            } => {
                let owner = self.get_actor(actor).clone();
                let actual = self.vote_escrow.get_votes(&owner);
                if actual != *expected_power {
                    panic!(
                        "expected voting power for '{}' to be {}, was {}",
                        actor, expected_power, actual
                    );
                }
            }
            SimStep::ExpectPastVotingPower {
                actor,
                ledger,
                expected_power,
            } => {
                let owner = self.get_actor(actor).clone();
                let actual = self.vote_escrow.get_past_votes(&owner, ledger);
                if actual != *expected_power {
                    panic!(
                        "expected past voting power for '{}' at ledger {} to be {}, was {}",
                        actor, ledger, expected_power, actual
                    );
                }
            }
            SimStep::ExpectPastTotalSupply {
                ledger,
                expected_total,
            } => {
                let actual = self.vote_escrow.get_past_total_supply(ledger);
                if actual != *expected_total {
                    panic!(
                        "expected past total supply at ledger {} to be {}, was {}",
                        ledger, expected_total, actual
                    );
                }
            }
            SimStep::AnchorResult {
                actor,
                poll_id,
                result_seed,
            } => {
                let anchorer = self.get_actor(actor).clone();
                let hash = description_hash(&self.env, result_seed);
                self.signal_anchor.anchor_result(&anchorer, poll_id, &hash);
            }
            SimStep::ExpectAnchor {
                poll_id,
                result_seed,
            } => {
                let expected_hash = description_hash(&self.env, result_seed);
                let record = self
                    .signal_anchor
                    .get_anchor(poll_id)
                    .unwrap_or_else(|| panic!("no anchor found for poll_id {}", poll_id));
                if record.result_hash != expected_hash {
                    panic!(
                        "anchored result_hash for poll_id {} did not match expected seed '{}'",
                        poll_id, result_seed
                    );
                }
            }
            SimStep::CreateLiquidityPool { fee_bps } => {
                let governor = self.governor.address.clone();
                self.liquidity.create_pool(
                    &governor,
                    &POOL_OUTCOME_A,
                    &POOL_OUTCOME_B,
                    &self.pool_token_a,
                    &self.pool_token_b,
                );
                self.liquidity
                    .initialize_pool(&governor, &POOL_OUTCOME_A, &POOL_OUTCOME_B, fee_bps);
            }
            SimStep::MintPoolTokens {
                actor,
                amount_a,
                amount_b,
            } => {
                let addr = self.get_actor(actor).clone();
                token::StellarAssetClient::new(&self.env, &self.pool_token_a).mint(&addr, amount_a);
                token::StellarAssetClient::new(&self.env, &self.pool_token_b).mint(&addr, amount_b);
            }
            SimStep::AddLiquidity {
                actor,
                amount_a,
                amount_b,
                min_lp_tokens_out,
            } => {
                let provider = self.get_actor(actor).clone();
                self.pool_before_last_change = self.current_pool();
                self.liquidity.add_liquidity(
                    &provider,
                    &POOL_OUTCOME_A,
                    &POOL_OUTCOME_B,
                    amount_a,
                    amount_b,
                    min_lp_tokens_out,
                );
            }
            SimStep::RemoveLiquidity { actor, lp_tokens } => {
                let provider = self.get_actor(actor).clone();
                self.pool_before_last_change = self.current_pool();
                self.liquidity
                    .remove_liquidity(&provider, &POOL_OUTCOME_A, &POOL_OUTCOME_B, lp_tokens);
            }
            SimStep::Swap {
                actor,
                asset_in,
                amount_in,
                min_amount_out,
            } => {
                let trader = self.get_actor(actor).clone();
                let (outcome_in, outcome_out) = match asset_in {
                    SimPoolAsset::A => (POOL_OUTCOME_A, POOL_OUTCOME_B),
                    SimPoolAsset::B => (POOL_OUTCOME_B, POOL_OUTCOME_A),
                };
                self.pool_before_last_change = self.current_pool();
                self.liquidity
                    .swap(&trader, &outcome_in, &outcome_out, amount_in, min_amount_out);
            }
            SimStep::UpdatePoolFee { fee_bps } => {
                let governor = self.governor.address.clone();
                self.liquidity
                    .update_pool_fee(&governor, &POOL_OUTCOME_A, &POOL_OUTCOME_B, fee_bps);
            }
            SimStep::ExpectPool {
                reserve_a,
                reserve_b,
                total_lp_supply,
                fee_bps,
            } => {
                let actual = self.liquidity.get_pool(&POOL_OUTCOME_A, &POOL_OUTCOME_B);
                let expected = Pool {
                    reserve_a: *reserve_a,
                    reserve_b: *reserve_b,
                    total_lp_supply: *total_lp_supply,
                    fee_bps: *fee_bps,
                };
                if actual != expected {
                    panic!("expected pool {:?}, was {:?}", expected, actual);
                }
            }
            SimStep::ExpectLpShares { actor, lp_tokens } => {
                let provider = self.get_actor(actor).clone();
                let actual =
                    self.liquidity
                        .get_lp_position(&provider, &POOL_OUTCOME_A, &POOL_OUTCOME_B);
                if actual != *lp_tokens {
                    panic!(
                        "expected '{}' to hold {} LP shares, held {}",
                        actor, lp_tokens, actual
                    );
                }
            }
            SimStep::ExpectPoolTokenBalance {
                actor,
                balance_a,
                balance_b,
            } => {
                let addr = self.get_actor(actor).clone();
                let actual_a = token::TokenClient::new(&self.env, &self.pool_token_a).balance(&addr);
                let actual_b = token::TokenClient::new(&self.env, &self.pool_token_b).balance(&addr);
                if (actual_a, actual_b) != (*balance_a, *balance_b) {
                    panic!(
                        "expected '{}' to hold ({}, {}) of pool assets (A, B), held ({}, {})",
                        actor, balance_a, balance_b, actual_a, actual_b
                    );
                }
            }
            SimStep::ExpectPoolInvariant { expect_growth } => {
                let before = self.pool_before_last_change.clone().unwrap_or_else(|| {
                    panic!("ExpectPoolInvariant needs a prior AddLiquidity, RemoveLiquidity or Swap")
                });
                let after = self.liquidity.get_pool(&POOL_OUTCOME_A, &POOL_OUTCOME_B);
                check_pool_invariant(&before, &after, *expect_growth);
            }
            SimStep::FundRewardsPool { actor, amount } => {
                let funder = self.get_actor(actor).clone();
                token::StellarAssetClient::new(&self.env, &self.reward_token).mint(&funder, amount);
                self.voting_rewards.fund_pool(&funder, amount);
            }
            SimStep::StartNextRewardsEpoch => {
                self.voting_rewards.start_next_epoch();
            }
            SimStep::PublishRewardsRoot {
                epoch_id,
                total_reward_amount,
                allocations,
            } => {
                let leaves: Vec<(Address, i128)> = allocations
                    .iter()
                    .map(|a| (self.get_actor(&a.actor).clone(), a.amount))
                    .collect();
                let hashes = reward_leaves(&self.env, *epoch_id, &leaves);
                let root = merkle_root(&self.env, &hashes);
                let governor = self.governor.address.clone();
                self.voting_rewards
                    .publish_epoch_root(&governor, epoch_id, &root, total_reward_amount);
                // Only reached if the contract accepted the root, so a
                // rejected re-publish can't replace the stored allocation.
                self.reward_allocations.insert(*epoch_id, leaves);
            }
            SimStep::ClaimReward {
                actor,
                epoch_id,
                amount,
            } => {
                let claimant = self.get_actor(actor).clone();
                let mut proof = SorobanVec::new(&self.env);
                if let Some(leaves) = self.reward_allocations.get(epoch_id) {
                    if let Some(index) = leaves.iter().position(|(addr, _)| *addr == claimant) {
                        let hashes = reward_leaves(&self.env, *epoch_id, leaves);
                        proof = merkle_proof(&self.env, &hashes, index);
                    }
                }
                self.voting_rewards.claim(&claimant, epoch_id, amount, &proof);
            }
            SimStep::ExpectRewardsEpoch {
                epoch_id,
                start_ledger,
                end_ledger,
                total_reward_amount,
                claimed_amount,
                finalized,
            } => {
                let epoch = self
                    .voting_rewards
                    .get_epoch(epoch_id)
                    .unwrap_or_else(|| panic!("no rewards epoch {}", epoch_id));
                let actual = (
                    epoch.start_ledger,
                    epoch.end_ledger,
                    epoch.total_reward_amount,
                    epoch.claimed_amount,
                    epoch.finalized,
                    epoch.merkle_root.is_some(),
                );
                let expected = (
                    *start_ledger,
                    *end_ledger,
                    *total_reward_amount,
                    *claimed_amount,
                    *finalized,
                    *finalized,
                );
                if actual != expected {
                    panic!(
                        "expected rewards epoch {} (start, end, total, claimed, finalized, has_root) = {:?}, was {:?}",
                        epoch_id, expected, actual
                    );
                }
            }
            SimStep::ExpectCurrentRewardsEpoch { epoch_id } => {
                let actual = self.voting_rewards.get_current_epoch_id();
                if actual != *epoch_id {
                    panic!("expected current rewards epoch {}, was {}", epoch_id, actual);
                }
            }
            SimStep::ExpectAvailableRewardsPool { amount } => {
                let actual = self.voting_rewards.get_available_pool();
                if actual != *amount {
                    panic!("expected available rewards pool {}, was {}", amount, actual);
                }
            }
            SimStep::ExpectRewardClaimed {
                actor,
                epoch_id,
                claimed,
            } => {
                let claimant = self.get_actor(actor).clone();
                let actual = self.voting_rewards.has_claimed(epoch_id, &claimant);
                if actual != *claimed {
                    panic!(
                        "expected has_claimed('{}', epoch {}) to be {}, was {}",
                        actor, epoch_id, claimed, actual
                    );
                }
            }
            SimStep::ExpectRewardBalance { actor, balance } => {
                let addr = self.get_actor(actor).clone();
                let actual = token::TokenClient::new(&self.env, &self.reward_token).balance(&addr);
                if actual != *balance {
                    panic!(
                        "expected '{}' to hold {} of the reward asset, held {}",
                        actor, balance, actual
                    );
                }
            }
            SimStep::RegisterTreasuryStrategy {
                max_allocation_bps,
                withdrawal_cooldown_ledgers,
            } => {
                let admin = self.treasury_strategies_admin.clone();
                self.treasury_strategies.register_strategy(
                    &admin,
                    &self.strategy_adapter,
                    &self.token,
                    max_allocation_bps,
                    withdrawal_cooldown_ledgers,
                );
            }
            SimStep::FundTreasury { amount } => {
                token::StellarAssetClient::new(&self.env, &self.token)
                    .mint(&self.treasury_addr, amount);
            }
            SimStep::DepositToTreasuryStrategy { amount } => {
                let treasury = self.treasury_addr.clone();
                self.treasury_strategies
                    .deposit(&treasury, &self.token, amount);
            }
            SimStep::RequestTreasuryWithdrawal {
                strategy_id,
                amount,
            } => {
                let treasury = self.treasury_addr.clone();
                self.treasury_strategies
                    .request_withdrawal(&treasury, strategy_id, amount);
            }
            SimStep::ClaimTreasuryWithdrawal { withdrawal_id } => {
                self.treasury_strategies.claim_withdrawal(withdrawal_id);
            }
            SimStep::ExpectTreasuryAllocation {
                strategy_id,
                amount,
            } => {
                let actual = self.treasury_strategies.get_allocation(strategy_id).amount;
                if actual != *amount {
                    panic!(
                        "expected strategy {} to hold an allocation of {}, was {}",
                        strategy_id, amount, actual
                    );
                }
            }
            SimStep::ExpectTreasuryBalance { balance } => {
                let actual =
                    token::TokenClient::new(&self.env, &self.token).balance(&self.treasury_addr);
                if actual != *balance {
                    panic!(
                        "expected the treasury to hold {} of the token, held {}",
                        balance, actual
                    );
                }
            }
            SimStep::ExpectTreasuryTotalValue { total } => {
                let actual = self.treasury_strategies.get_total_value(&self.token);
                if actual != *total {
                    panic!(
                        "expected treasury strategies to report a total value of {}, was {}",
                        total, actual
                    );
                }
            }
        }
    }

    /// The pool's current state, or `None` before `CreateLiquidityPool`.
    fn current_pool(&self) -> Option<Pool> {
        self.liquidity
            .get_pool_safe(&POOL_OUTCOME_A, &POOL_OUTCOME_B)
    }

    /// Post-process `ExpectError` steps against already-recorded results —
    /// they assert on a *prior* step's outcome, so they can't be resolved
    /// during `dispatch` (which only sees the current step).
    pub fn check_expect_errors(&mut self) {
        let checks: std::vec::Vec<(usize, usize, String)> = self
            .scenario
            .steps
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                SimStep::ExpectError {
                    step_index,
                    expected_error,
                } => Some((i, *step_index, expected_error.clone())),
                _ => None,
            })
            .collect();

        for (check_step_idx, target_step_idx, expected) in checks {
            let Some(target_result) = self.report.step_results.get(target_step_idx) else {
                continue;
            };
            let target_error = target_result.error.clone();
            let target_was_failure = !target_result.success;
            let matches = target_error
                .as_ref()
                .map(|e| e.contains(&expected))
                .unwrap_or(false);

            if matches && target_was_failure {
                // The target step's failure was anticipated and validated —
                // exclude it from the failure tally (its `success` field
                // stays false, since it factually did fail).
                if let Some(target) = self.report.step_results.get_mut(target_step_idx) {
                    if !target.anticipated_failure {
                        target.anticipated_failure = true;
                        self.report.failed_steps = self.report.failed_steps.saturating_sub(1);
                        self.report.passed_steps += 1;
                    }
                }
            }

            if let Some(check_result) = self.report.step_results.get_mut(check_step_idx) {
                if !matches {
                    check_result.success = false;
                    check_result.error = Some(format!(
                        "expected step {} to fail with an error containing '{}', got {:?}",
                        target_step_idx, expected, target_error
                    ));
                    self.report.failed_steps += 1;
                    self.report.passed_steps = self.report.passed_steps.saturating_sub(1);
                }
            }
        }
    }

    pub fn get_report(&self) -> &SimulationReport {
        &self.report
    }
}

fn panic_message(e: std::boxed::Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = e.downcast_ref::<std::string::String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn description_hash(env: &Env, description: &str) -> BytesN<32> {
    env.crypto()
        .sha256(&Bytes::from_slice(env, description.as_bytes()))
        .into()
}

fn sim_vote_type(t: SimVoteType) -> VoteType {
    match t {
        SimVoteType::Simple => VoteType::Simple,
        SimVoteType::Extended => VoteType::Extended,
        SimVoteType::Quadratic => VoteType::Quadratic,
    }
}

fn sim_vote_support(s: SimVoteSupport) -> VoteSupport {
    match s {
        SimVoteSupport::Against => VoteSupport::Against,
        SimVoteSupport::For => VoteSupport::For,
        SimVoteSupport::Abstain => VoteSupport::Abstain,
    }
}

fn from_proposal_state(s: ProposalState) -> SimProposalState {
    match s {
        ProposalState::Pending => SimProposalState::Pending,
        ProposalState::Active => SimProposalState::Active,
        ProposalState::Defeated => SimProposalState::Defeated,
        ProposalState::Succeeded => SimProposalState::Succeeded,
        ProposalState::Queued => SimProposalState::Queued,
        ProposalState::Executed => SimProposalState::Executed,
        ProposalState::Cancelled => SimProposalState::Cancelled,
        ProposalState::Expired => SimProposalState::Expired,
    }
}

fn reward_leaves(env: &Env, epoch_id: u64, leaves: &[(Address, i128)]) -> Vec<BytesN<32>> {
    leaves
        .iter()
        .map(|(addr, amount)| merkle::compute_leaf(env, addr, epoch_id, *amount))
        .collect()
}

/// `sha256(min(a, b) || max(a, b))` — mirrors the crate-private
/// `merkle::hash_pair` in `contracts/voting-rewards`. A mismatch would make
/// every valid `ClaimReward` fail with `InvalidProof`.
fn merkle_hash_pair(env: &Env, a: &BytesN<32>, b: &BytesN<32>) -> BytesN<32> {
    let (first, second) = if a.to_array() <= b.to_array() {
        (a, b)
    } else {
        (b, a)
    };
    let mut buf = Bytes::from_array(env, &first.to_array());
    buf.append(&Bytes::from_array(env, &second.to_array()));
    env.crypto().sha256(&buf).into()
}

/// One tree level up: hash adjacent pairs, promoting an odd trailing node
/// unchanged (the same rule as `backend/src/voting-rewards/merkle.ts` and
/// the contract's own reference tree in `tests.rs`).
fn merkle_parent_level(env: &Env, level: &[BytesN<32>]) -> Vec<BytesN<32>> {
    level
        .chunks(2)
        .map(|pair| match pair {
            [a, b] => merkle_hash_pair(env, a, b),
            [a] => a.clone(),
            _ => unreachable!(),
        })
        .collect()
}

fn merkle_root(env: &Env, leaves: &[BytesN<32>]) -> BytesN<32> {
    if leaves.is_empty() {
        // No leaves: publish an all-zero root nothing can prove against.
        return BytesN::from_array(env, &[0; 32]);
    }
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        level = merkle_parent_level(env, &level);
    }
    level.remove(0)
}

fn merkle_proof(env: &Env, leaves: &[BytesN<32>], mut index: usize) -> SorobanVec<BytesN<32>> {
    let mut proof = SorobanVec::new(env);
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        if let Some(sibling) = level.get(index ^ 1) {
            proof.push_back(sibling.clone());
        }
        level = merkle_parent_level(env, &level);
        index /= 2;
    }
    proof
}

/// Panics unless `reserve_a * reserve_b / total_lp_supply^2` is no lower
/// after the change than before it (strictly higher if `expect_growth`).
/// Cross-multiplied to stay in integers: `k_after * S_before^2` vs
/// `k_before * S_after^2`.
fn check_pool_invariant(before: &Pool, after: &Pool, expect_growth: bool) {
    if after.total_lp_supply == 0 {
        // A fully withdrawn pool must not strand reserves with no LP claim.
        if after.reserve_a != 0 || after.reserve_b != 0 {
            panic!(
                "pool has no LP supply but still holds reserves: {:?}",
                after
            );
        }
        return;
    }
    if before.total_lp_supply == 0 {
        // First deposit: nothing to compare against, only that it is funded.
        if after.reserve_a <= 0 || after.reserve_b <= 0 {
            panic!("first deposit left an unfunded pool: {:?}", after);
        }
        return;
    }

    let mul = |a: i128, b: i128| {
        a.checked_mul(b).unwrap_or_else(|| {
            panic!(
                "overflow checking pool invariant: {:?} -> {:?}",
                before, after
            )
        })
    };
    let k_before = mul(before.reserve_a, before.reserve_b);
    let k_after = mul(after.reserve_a, after.reserve_b);
    let lhs = mul(k_after, mul(before.total_lp_supply, before.total_lp_supply));
    let rhs = mul(k_before, mul(after.total_lp_supply, after.total_lp_supply));

    if lhs < rhs || (expect_growth && lhs == rhs) {
        panic!(
            "pool product per LP share {} (k {} -> {}, LP supply {} -> {})",
            if lhs < rhs {
                "decreased"
            } else {
                "did not grow"
            },
            k_before,
            k_after,
            before.total_lp_supply,
            after.total_lp_supply
        );
    }
}

fn merge_settings(current: &GovernorSettings, overrides: &SimGovernorSettings) -> GovernorSettings {
    GovernorSettings {
        voting_delay: overrides.voting_delay,
        voting_period: overrides.voting_period,
        quorum_numerator: overrides.quorum_numerator,
        proposal_threshold: overrides.proposal_threshold,
        guardian: current.guardian.clone(),
        vote_type: sim_vote_type(overrides.vote_type),
        proposal_grace_period: overrides.proposal_grace_period,
        use_dynamic_quorum: current.use_dynamic_quorum,
        reflector_oracle: current.reflector_oracle.clone(),
        min_quorum_usd: current.min_quorum_usd,
        max_calldata_size: overrides.max_calldata_size,
        proposal_cooldown: overrides.proposal_cooldown,
        max_proposals_per_period: overrides.max_proposals_per_period,
        proposal_period_duration: overrides.proposal_period_duration,
        // Not modeled in the scenario DSL (no co-sponsorship or commit-reveal
        // concept in SimGovernorSettings) — carried through unchanged from
        // whatever's currently configured, same treatment as `guardian`/
        // `reflector_oracle`.
        co_sponsorship_registry: current.co_sponsorship_registry.clone(),
        use_commit_reveal: current.use_commit_reveal,
        commit_phase_fraction: current.commit_phase_fraction,
    }
}

fn estimated_storage_touches(step: &SimStep) -> (u32, u32) {
    match step {
        SimStep::Propose { targets, .. } => {
            let target_count = targets.len() as u32;
            let reads = 3;
            let writes = 5 + target_count;
            (reads, writes)
        }
        SimStep::Vote { .. } => (2, 2),
        SimStep::Queue { .. } | SimStep::Execute { .. } | SimStep::Cancel { .. } => (2, 2),
        SimStep::UpdateConfig { .. } => (1, 14),
        SimStep::LockProposalBond { .. }
        | SimStep::RefundProposalBond { .. }
        | SimStep::ExpectBondState { .. } => (2, 2),
        SimStep::ProposeBondSlash { .. } => (3, 6),
        _ => (0, 0),
    }
}
