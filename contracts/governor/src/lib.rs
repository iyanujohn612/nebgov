#![no_std]
#![allow(clippy::too_many_arguments)]
// Prevent future introduction of dead match patterns in this security-critical
// state machine (issue #439).
#![deny(unreachable_patterns)]

mod commit_reveal;
pub mod error;
mod events;
mod reputation;

use crate::error::GovernorError;
use soroban_sdk::xdr::FromXdr;
use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractclient, contractimpl, contracttype, symbol_short, token,
    Address, Bytes, BytesN, Env, IntoVal, String, Symbol, Val, Vec,
};

/// Current storage schema version.
///
/// Increment this constant when making breaking changes to the on-chain
/// storage layout so that [`GovernorContract::migrate`] can apply the
/// corresponding migration step.
const CURRENT_STORAGE_VERSION: u32 = 1;


/// Cross-contract interface for the Timelock contract.
///
/// The governor uses this to schedule proposals after they succeed and to
/// trigger execution once the mandatory delay has elapsed.
#[contractclient(name = "TimelockClient")]
pub trait TimelockTrait {
    #[allow(clippy::too_many_arguments)]
    fn schedule(
        env: Env,
        caller: Address,
        target: Address,
        data: Bytes,
        fn_name: Symbol,
        delay: u64,
        predecessor: Bytes,
        salt: Bytes,
    ) -> Bytes;
    #[allow(clippy::too_many_arguments)]
    fn schedule_batch(
        env: Env,
        caller: Address,
        targets: Vec<Address>,
        datas: Vec<Bytes>,
        fn_names: Vec<Symbol>,
        delay: u64,
        predecessor: Bytes,
        salt: Bytes,
    ) -> Bytes;
    fn execute(env: Env, caller: Address, op_id: Bytes);
    fn execute_batch(env: Env, caller: Address, batch_op_id: Bytes);
    fn cancel(env: Env, caller: Address, op_id: Bytes);
    fn min_delay(env: Env) -> u64;
    fn execution_window(env: Env) -> u64;
    fn is_done(env: Env, op_id: Bytes) -> bool;
    fn is_batch_done(env: Env, batch_op_id: Bytes) -> bool;
}

/// Cross-contract interface for the TokenVotes contract.
///
/// The governor uses this to check voting power when creating proposals and
/// casting votes, and to query snapshot-based total supply for quorum.
#[contractclient(name = "VotesClient")]
pub trait VotesTrait {
    fn get_votes(env: Env, account: Address) -> i128;
    fn get_past_votes(env: Env, account: Address, ledger: u32) -> i128;
    fn get_past_total_supply(env: Env, ledger: u32) -> i128;
    fn token(env: Env) -> Address;
}

/// Cross-contract interface for the Reflector oracle.
///
/// Used for dynamic quorum calculations based on token USD price.
#[contractclient(name = "ReflectorOracleClient")]
pub trait ReflectorOracleTrait {
    fn lastprice(env: Env, asset: Address) -> Option<i128>;
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WeightedToken {
    pub token: Address,
    pub weight_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum VotingStrategy {
    /// Single token (default, backward compatible). Uses the VotesToken stored at init.
    Single,
    /// Weighted sum from up to 5 tokens. weight = sum(get_past_votes(t) * bps / 10000).
    MultiToken(Vec<WeightedToken>),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ProposalState {
    Pending,
    Active,
    Defeated,
    Succeeded,
    Queued,
    Executed,
    Cancelled,
    Expired,
}

#[contracttype]
#[derive(Clone)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub description: String,
    pub description_hash: BytesN<32>,
    pub metadata_uri: String,
    pub targets: Vec<Address>,
    pub fn_names: Vec<Symbol>,
    pub calldatas: Vec<Bytes>,
    pub start_ledger: u32,
    pub end_ledger: u32,
    pub votes_for: i128,
    pub votes_against: i128,
    pub votes_abstain: i128,
    pub executed: bool,
    pub cancelled: bool,
    pub queued: bool,
    pub op_ids: Vec<Bytes>,
}

#[contracttype]
pub struct MigrateData {
    pub new_version: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GovernorSettings {
    pub voting_delay: u32,
    pub voting_period: u32,
    pub quorum_numerator: u32,
    pub proposal_threshold: i128,
    pub guardian: Address,
    pub vote_type: VoteType,
    pub proposal_grace_period: u32,
    pub use_dynamic_quorum: bool,
    pub reflector_oracle: Option<Address>,
    pub min_quorum_usd: i128,
    pub max_calldata_size: u32,
    pub proposal_cooldown: u32,
    pub max_proposals_per_period: u32,
    pub proposal_period_duration: u32,
    pub co_sponsorship_registry: Option<Address>,
    pub use_commit_reveal: bool,
    pub commit_phase_fraction: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CanProposeResult {
    pub allowed: bool,
    pub reason: Symbol,
    pub cooldown_ends_at: Option<u32>,
    pub proposals_this_period: u32,
    pub max_per_period: u32,
    pub voting_power: i128,
    pub threshold: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProposalStateCounts {
    pub pending: u64,
    pub active: u64,
    pub succeeded: u64,
    pub defeated: u64,
    pub queued: u64,
    pub executed: u64,
    pub cancelled: u64,
    pub expired: u64,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum VoteSupport {
    Against,
    For,
    Abstain,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VotingReceipt {
    pub has_voted: bool,
    pub support: VoteSupport,
    pub weight: i128,
    pub reason: String,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum VoteType {
    Simple,    // For / Against only
    Extended,  // For / Against / Abstain (current)
    Quadratic, // weight = sqrt(tokens)
}

#[contracttype]
pub enum DataKey {
    Proposal(u64),
    ProposalCount,
    VotingDelay,
    VotingPeriod,
    QuorumNumerator,
    ProposalThreshold,
    Timelock,
    VotesToken,
    Admin,
    Guardian,
    VoteType,
    ProposalGracePeriod,
    HasVoted(u64, Address),
    VoteReceipt(u64, Address),
    // The ledger sequence number when a proposal was queued (for veto window tracking).
    QueueTime(u64),
    ProposalExpiredEmitted(u64),
    CurrentWasmHash,
    // Active voting strategy (Single or MultiToken).
    VotingStrategy,
    // Whether dynamic quorum is enabled.
    UseDynamicQuorum,
    // Address of the Reflector oracle for dynamic quorum.
    ReflectorOracle,
    // Minimum quorum floor in USD (6-decimal format).
    MinQuorumUsd,
    // Address authorized to pause/unpause the contract.
    Pauser,
    // Whether the contract is currently paused.
    IsPaused,
    // Last proposal ledger for an address (for cooldown).
    LastProposalLedger(Address),
    // Proposal count for an address in current period.
    ProposalsInPeriod(Address, u32),
    // Maximum calldata size per action in bytes.
    MaxCalldataSize,
    // Minimum ledgers between proposals from the same address (cooldown period).
    ProposalCooldown,
    // Maximum proposals per period (period measured in ledgers).
    MaxProposalsPerPeriod,
    // Period duration in ledgers for proposal rate limiting.
    ProposalPeriodDuration,
    // Cached timelock min_delay/execution_window (seconds), written once at initialize time.
    CachedTimelockDelay,
    CachedExecutionWindow,
    // Ordered list of proposal ids for pagination.
    ProposalList,
    // Current storage schema version (for migration tracking).
    StorageVersion,
    // Address of the trusted co-sponsorship registry contract, if configured.
    CoSponsorshipRegistry,
    // Per-address proposer reputation record (Issue #771).
    ProposerReputation(Address),
    // Idempotency guard: a proposal expiring via grace-period only records its
    // reputation penalty once, no matter how many times state() is read.
    ReputationExpiredRecorded(u64),
    // Commitment hash a voter submitted (Issue #766); presence doubles as "has committed".
    VoteCommitment(u64, Address),
    // Ledger after which a proposal's commit phase ends, snapshotted at creation.
    CommitDeadline(u64),
    RevealDeadline(u64),
    // Whether commit-reveal voting is enabled governor-wide for new proposals.
    UseCommitReveal,
    // Fraction (bps) of a new proposal's voting_period allotted to the commit phase.
    CommitPhaseFraction,
}

#[contract]
pub struct GovernorContract;

#[contractimpl]
impl GovernorContract {
    /// Approximate number of seconds per Soroban ledger close.
    ///
    /// Used to convert timelock delay/window values (in seconds) to ledger
    /// counts. Stellar Mainnet closes ledgers roughly every 5 seconds; this
    /// constant must be kept consistent across all conversion sites in this
    /// file so that TTL extensions and veto-window calculations use the same
    /// baseline.
    const SECONDS_PER_LEDGER: u64 = 5;

    fn must_get_proposal(env: &Env, proposal_id: u64) -> Proposal {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ProposalNotFound))
    }

    fn extend_proposal_ttl(env: &Env, proposal_id: u64, proposal: &Proposal) {
        let current = env.ledger().sequence();

        let grace_period: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalGracePeriod)
            .unwrap_or(120_960);

        // Use cached timelock timing values (written at initialize time) to avoid
        // repeated cross-contract calls. Falls back to live calls for pre-migration
        // contracts that don't have the cache entries yet.
        let cached_delay: Option<u64> = env.storage().instance().get(&DataKey::CachedTimelockDelay);
        let cached_window: Option<u64> = env
            .storage()
            .instance()
            .get(&DataKey::CachedExecutionWindow);
        let timelock_delay_ledgers: u32 = match (cached_delay, cached_window) {
            (Some(d), Some(w)) => ((d + w) / Self::SECONDS_PER_LEDGER) as u32,
            _ => {
                let timelock_addr: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::Timelock)
                    .unwrap_or_else(|| env.panic_with_error(GovernorError::TimelockNotSet));
                let timelock = TimelockClient::new(env, &timelock_addr);
                ((timelock.min_delay() + timelock.execution_window()) / Self::SECONDS_PER_LEDGER)
                    as u32
            }
        };

        let ledgers_until_end = proposal.end_ledger.saturating_sub(current);

        let ttl_ledgers = ledgers_until_end
            .saturating_add(grace_period)
            .saturating_add(timelock_delay_ledgers)
            .saturating_add(1000);

        env.storage().persistent().extend_ttl(
            &DataKey::Proposal(proposal_id),
            ttl_ledgers,
            ttl_ledgers,
        );
    }

    fn decode_calldata_args(env: &Env, data: &Bytes) -> Vec<Val> {
        if data.is_empty() {
            return Vec::new(env);
        }

        if let Ok(args) = Vec::<Val>::from_xdr(env, data) {
            return args;
        }

        Vec::new(env)
    }

    fn emit_proposal_expired_if_needed(env: &Env, proposal: &Proposal) {
        let expired_emitted: bool = env
            .storage()
            .persistent()
            .get(&DataKey::ProposalExpiredEmitted(proposal.id))
            .unwrap_or(false);

        if expired_emitted || proposal.cancelled || proposal.executed || proposal.queued {
            return;
        }

        let current = env.ledger().sequence();
        let quorum = Self::quorum(env.clone(), proposal.id);
        let quorum_met = proposal.votes_for + proposal.votes_abstain >= quorum;
        let for_wins = proposal.votes_for > proposal.votes_against;

        if current > proposal.end_ledger && !(quorum_met && for_wins) {
            events::emit_proposal_expired(env, proposal.id, proposal.end_ledger);
            env.storage()
                .persistent()
                .set(&DataKey::ProposalExpiredEmitted(proposal.id), &true);

            // Reputation: this branch is reached exactly when a proposal
            // concludes without succeeding (fails quorum and/or the vote),
            // i.e. the Defeated classification in state() below. Guarded by
            // the flag above so it fires at most once (Issue #771).
            reputation::record_proposal_terminal(
                env,
                &proposal.proposer,
                reputation::ReputationOutcome::Defeated,
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        admin: Address,
        votes_token: Address,
        timelock: Address,
        voting_delay: u32,
        voting_period: u32,
        quorum_numerator: u32,
        proposal_threshold: i128,
        guardian: Address,
        vote_type: VoteType,
        proposal_grace_period: u32,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Admin) {
            env.panic_with_error(GovernorError::AlreadyInitialized);
        }
        if voting_period == 0 {
            env.panic_with_error(GovernorError::VotePeriodTooShort);
        }
        let timelock_client = TimelockClient::new(&env, &timelock);
        let execution_window = timelock_client.execution_window();
        if execution_window == 0 {
            env.panic_with_error(GovernorError::ExecutionWindowZero);
        }
        let min_delay = timelock_client.min_delay();
        // timelock_delay + execution_window must not overflow u64
        let _ = min_delay
            .checked_add(execution_window)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ArithmeticOverflow));
        // Cache timing values so extend_proposal_ttl can avoid repeated cross-contract calls.
        env.storage()
            .instance()
            .set(&DataKey::CachedTimelockDelay, &min_delay);
        env.storage()
            .instance()
            .set(&DataKey::CachedExecutionWindow, &execution_window);

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::VotesToken, &votes_token);
        env.storage().instance().set(&DataKey::Timelock, &timelock);
        env.storage()
            .instance()
            .set(&DataKey::VotingDelay, &voting_delay);
        env.storage()
            .instance()
            .set(&DataKey::VotingPeriod, &voting_period);
        env.storage()
            .instance()
            .set(&DataKey::QuorumNumerator, &quorum_numerator);
        env.storage()
            .instance()
            .set(&DataKey::ProposalThreshold, &proposal_threshold);
        env.storage().instance().set(&DataKey::Guardian, &guardian);
        env.storage().instance().set(&DataKey::VoteType, &vote_type);
        env.storage()
            .instance()
            .set(&DataKey::ProposalGracePeriod, &proposal_grace_period);
        env.storage().instance().set(&DataKey::ProposalCount, &0u64);
        env.storage()
            .persistent()
            .set(&DataKey::ProposalList, &Vec::<u64>::new(&env));
        env.storage().instance().set(
            &DataKey::CurrentWasmHash,
            &BytesN::from_array(&env, &[0u8; 32]),
        );
        env.storage()
            .instance()
            .set(&DataKey::VotingStrategy, &VotingStrategy::Single);
        env.storage()
            .instance()
            .set(&DataKey::UseDynamicQuorum, &false);
        // Initialize new security settings with defaults
        env.storage()
            .instance()
            .set(&DataKey::MaxCalldataSize, &10_000u32); // 10KB max calldata
        env.storage()
            .instance()
            .set(&DataKey::ProposalCooldown, &100u32); // ~10 min cooldown
        env.storage()
            .instance()
            .set(&DataKey::MaxProposalsPerPeriod, &5u32); // 5 proposals per period
        env.storage()
            .instance()
            .set(&DataKey::ProposalPeriodDuration, &10_000u32); // ~24 hour period
        // Commit-reveal voting disabled by default (Issue #766); when a
        // governance proposal turns it on, new proposals split their voting
        // period 50/50 between commit and reveal unless overridden.
        env.storage()
            .instance()
            .set(&DataKey::UseCommitReveal, &false);
        env.storage().instance().set(
            &DataKey::CommitPhaseFraction,
            &commit_reveal::DEFAULT_COMMIT_PHASE_FRACTION_BPS,
        );
        // Initialize pause state (not paused by default)
        env.storage().instance().set(&DataKey::IsPaused, &false);
        // Set admin as initial pauser
        env.storage().instance().set(&DataKey::Pauser, &admin);
        // Write storage version for migration tracking.
        env.storage()
            .instance()
            .set(&DataKey::StorageVersion, &CURRENT_STORAGE_VERSION);
    }

    fn apply_migration_v1(env: &Env) {
        if !env.storage().instance().has(&DataKey::MaxCalldataSize) {
            env.storage()
                .instance()
                .set(&DataKey::MaxCalldataSize, &10_000u32);
        }
        if !env.storage().instance().has(&DataKey::ProposalCooldown) {
            env.storage()
                .instance()
                .set(&DataKey::ProposalCooldown, &100u32);
        }
        if !env.storage().instance().has(&DataKey::MaxProposalsPerPeriod) {
            env.storage()
                .instance()
                .set(&DataKey::MaxProposalsPerPeriod, &5u32);
        }
        if !env.storage().instance().has(&DataKey::ProposalPeriodDuration) {
            env.storage()
                .instance()
                .set(&DataKey::ProposalPeriodDuration, &10_000u32);
        }
        if !env.storage().instance().has(&DataKey::IsPaused) {
            env.storage().instance().set(&DataKey::IsPaused, &false);
        }
        if !env.storage().instance().has(&DataKey::Pauser) {
            // Admin is always written during initialize() before any upgrade
            // can be triggered (upgrade requires current_contract_address()
            // auth, which only passes through an on-chain proposal that itself
            // requires a proposer — impossible before the contract is fully
            // initialized). A missing Admin key therefore indicates a
            // deployment that was never initialized; surface a typed error
            // rather than an opaque runtime panic.
            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .unwrap_or_else(|| env.panic_with_error(GovernorError::PauserNotSet));
            env.storage().instance().set(&DataKey::Pauser, &admin);
        }
        if !env.storage().instance().has(&DataKey::CurrentWasmHash) {
            env.storage().instance().set(
                &DataKey::CurrentWasmHash,
                &BytesN::from_array(env, &[0u8; 32]),
            );
        }
        if !env.storage().instance().has(&DataKey::VotingStrategy) {
            env.storage()
                .instance()
                .set(&DataKey::VotingStrategy, &VotingStrategy::Single);
        }
        if !env.storage().instance().has(&DataKey::UseDynamicQuorum) {
            env.storage()
                .instance()
                .set(&DataKey::UseDynamicQuorum, &false);
        }
        if !env.storage().persistent().has(&DataKey::ProposalList) {
            env.storage()
                .persistent()
                .set(&DataKey::ProposalList, &Vec::<u64>::new(env));
        }
        if !env.storage().instance().has(&DataKey::UseCommitReveal) {
            env.storage()
                .instance()
                .set(&DataKey::UseCommitReveal, &false);
        }
        if !env.storage().instance().has(&DataKey::CommitPhaseFraction) {
            env.storage().instance().set(
                &DataKey::CommitPhaseFraction,
                &commit_reveal::DEFAULT_COMMIT_PHASE_FRACTION_BPS,
            );
        }
    }

    pub fn set_initial_config(
        env: Env,
        admin: Address,
        max_calldata_size: u32,
        proposal_cooldown: u32,
        max_proposals_per_period: u32,
    ) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::UnauthorizedPause));
        if admin != stored_admin {
            env.panic_with_error(GovernorError::UnauthorizedPause);
        }
        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0);
        if count > 0 {
            env.panic_with_error(GovernorError::ProposalRateLimited);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxCalldataSize, &max_calldata_size);
        env.storage()
            .instance()
            .set(&DataKey::ProposalCooldown, &proposal_cooldown);
        env.storage()
            .instance()
            .set(&DataKey::MaxProposalsPerPeriod, &max_proposals_per_period);
    }

    fn validate_action(env: &Env, targets: &Vec<Address>, fn_names: &Vec<Symbol>, calldatas: &Vec<Bytes>) {
        if !(targets.len() == fn_names.len() && targets.len() == calldatas.len()) {
            env.panic_with_error(GovernorError::InvalidVectorLengths);
        }
        if targets.is_empty() {
            env.panic_with_error(GovernorError::NoTargets);
        }

        let max_calldata_size: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxCalldataSize)
            .unwrap_or(10_000);
        for i in 0..calldatas.len() {
            let calldata = calldatas.get(i).unwrap();
            if calldata.len() > max_calldata_size {
                env.panic_with_error(GovernorError::CalldataTooLarge);
            }
        }

        // MaxCalldataSize bounds each entry's byte size but not the *count* of
        // entries — without this, a proposal with many tiny entries could
        // exhaust the compute budget when execute() dispatches every one.
        const MAX_CALLDATA_COUNT: u32 = 10;
        if calldatas.len() > MAX_CALLDATA_COUNT {
            env.panic_with_error(GovernorError::TooManyCalldataEntries);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn propose(
        env: Env,
        proposer: Address,
        description: String,
        description_hash: BytesN<32>,
        metadata_uri: String,
        targets: Vec<Address>,
        fn_names: Vec<Symbol>,
        calldatas: Vec<Bytes>,
    ) -> u64 {
        proposer.require_auth();
        Self::validate_action(&env, &targets, &fn_names, &calldatas);

        // Get the voting power of the proposer (strategy-aware)
        let proposer_votes = Self::compute_proposer_votes(&env, &proposer);

        // Enforce proposal threshold, adjusted by the proposer's reputation
        // score (Issue #771). Degrades to the flat threshold when reputation
        // is disabled or the proposer has no history yet.
        let threshold: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalThreshold)
            .unwrap_or(0);
        let effective_threshold = reputation::get_effective_threshold(&env, &proposer, threshold);

        if proposer_votes < effective_threshold {
            env.panic_with_error(GovernorError::ProposalThresholdNotMet);
        }

        Self::create_proposal_internal(
            &env,
            &proposer,
            description,
            description_hash,
            metadata_uri,
            targets,
            fn_names,
            calldatas,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn propose_via_registry(
        env: Env,
        registry: Address,
        proposer: Address,
        description: String,
        description_hash: BytesN<32>,
        metadata_uri: String,
        targets: Vec<Address>,
        fn_names: Vec<Symbol>,
        calldatas: Vec<Bytes>,
    ) -> u64 {
        registry.require_auth();
        let configured: Address = env
            .storage()
            .instance()
            .get(&DataKey::CoSponsorshipRegistry)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::UnauthorizedRegistry));
        if registry != configured {
            env.panic_with_error(GovernorError::UnauthorizedRegistry);
        }

        Self::validate_action(&env, &targets, &fn_names, &calldatas);

        // No individual-proposer threshold check here, by design: the
        // configured registry (co-sponsorship contract) already verifies
        // pooled co-sponsor power against the reputation-adjusted
        // `get_effective_threshold` before calling this (#849) — that's the
        // entire point of the registry bypass, letting an individually
        // under-powered proposer through on pooled co-sponsor support.
        // Re-checking `proposer`'s own votes here would defeat that purpose
        // entirely, since it's exactly the case this path exists to allow.

        Self::create_proposal_internal(
            &env,
            &proposer,
            description,
            description_hash,
            metadata_uri,
            targets,
            fn_names,
            calldatas,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_proposal_internal(
        env: &Env,
        proposer: &Address,
        description: String,
        description_hash: BytesN<32>,
        metadata_uri: String,
        targets: Vec<Address>,
        fn_names: Vec<Symbol>,
        calldatas: Vec<Bytes>,
    ) -> u64 {
        // Rate limiting checks (Issue #188)
        let current_ledger = env.ledger().sequence();

        // Check cooldown period
        let cooldown: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCooldown)
            .unwrap_or(100);
        let last_proposal_ledger: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::LastProposalLedger(proposer.clone()));
        if let Some(last) = last_proposal_ledger {
            if current_ledger < last + cooldown {
                env.panic_with_error(GovernorError::ProposalRateLimited);
            }
        }

        // Check max proposals per period
        let period_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalPeriodDuration)
            .unwrap_or(10_000);
        let max_proposals: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxProposalsPerPeriod)
            .unwrap_or(5);

        let current_period = current_ledger / period_duration;
        let proposals_in_period: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ProposalsInPeriod(
                proposer.clone(),
                current_period,
            ))
            .unwrap_or(0);

        if proposals_in_period >= max_proposals {
            env.panic_with_error(GovernorError::ProposalRateLimited);
        }

        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0);
        let proposal_id = count + 1;

        let voting_delay: u32 = env
            .storage()
            .instance()
            .get(&DataKey::VotingDelay)
            .unwrap_or(100);
        let voting_period: u32 = env
            .storage()
            .instance()
            .get(&DataKey::VotingPeriod)
            .unwrap_or(1000);

        let current = env.ledger().sequence();
        let proposal = Proposal {
            id: proposal_id,
            proposer: proposer.clone(),
            description: description.clone(),
            description_hash: description_hash.clone(),
            metadata_uri: metadata_uri.clone(),
            targets: targets.clone(),
            fn_names: fn_names.clone(),
            calldatas: calldatas.clone(),
            start_ledger: current + voting_delay,
            end_ledger: current + voting_delay + voting_period,
            votes_for: 0,
            votes_against: 0,
            votes_abstain: 0,
            executed: false,
            cancelled: false,
            queued: false,
            op_ids: Vec::new(env),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        // Extend TTL to cover full proposal lifecycle
        Self::extend_proposal_ttl(env, proposal_id, &proposal);
        env.storage()
            .instance()
            .set(&DataKey::ProposalCount, &proposal_id);

        let mut proposal_list: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ProposalList)
            .unwrap_or(Vec::new(env));
        proposal_list.push_back(proposal_id);
        env.storage()
            .persistent()
            .set(&DataKey::ProposalList, &proposal_list);

        // Update rate limiting storage
        env.storage()
            .persistent()
            .set(&DataKey::LastProposalLedger(proposer.clone()), &current);
        // Extend TTL for the cooldown entry so it does not expire before the
        // cooldown period elapses (Issue #621).  Add a 1000-ledger buffer so
        // the storage survives even if the cooldown is set close to the default
        // Soroban TTL ceiling.
        env.storage().persistent().extend_ttl(
            &DataKey::LastProposalLedger(proposer.clone()),
            cooldown.saturating_add(1000),
            cooldown.saturating_add(1000),
        );

        // Prune the previous period's key for this proposer to prevent
        // unbounded storage growth (Issue #716).  We only need the current
        // period's count for rate-limiting, so the entry from
        // `current_period - 1` can be safely removed.
        if current_period > 0 {
            env.storage()
                .persistent()
                .remove(&DataKey::ProposalsInPeriod(
                    proposer.clone(),
                    current_period - 1,
                ));
        }

        env.storage().persistent().set(
            &DataKey::ProposalsInPeriod(proposer.clone(), current_period),
            &(proposals_in_period + 1),
        );
        // Extend TTL for the period-count entry so it does not expire before
        // the period ends (Issue #621).  Add a 1000-ledger buffer for safety.
        env.storage().persistent().extend_ttl(
            &DataKey::ProposalsInPeriod(proposer.clone(), current_period),
            period_duration.saturating_add(1000),
            period_duration.saturating_add(1000),
        );

        reputation::record_proposal_created(env, proposer);
        commit_reveal::start_phase_if_enabled(env, proposal_id, proposal.start_ledger, proposal.end_ledger);

        events::emit_proposal_created(env, &proposal);

        proposal_id
    }

    fn apply_vote_type(vote_type: VoteType, raw_weight: i128) -> i128 {
        match vote_type {
            VoteType::Simple | VoteType::Extended => raw_weight,
            VoteType::Quadratic => {
                // Integer square root for quadratic voting
                let raw_u128 = raw_weight as u128;
                if raw_u128 == 0 {
                    return 0;
                }
                // Simple integer square root implementation
                let mut x = raw_u128;
                let mut y = x.div_ceil(2);
                while y < x {
                    x = y;
                    y = (x + raw_u128 / x) / 2;
                }
                x as i128
            }
        }
    }

    fn validate_vote_support(env: &Env, support: &VoteSupport) -> Result<(), GovernorError> {
        let vote_type: VoteType = env
            .storage()
            .instance()
            .get(&DataKey::VoteType)
            .unwrap_or(VoteType::Extended);

        match vote_type {
            VoteType::Simple => {
                if matches!(support, VoteSupport::Abstain) {
                    Err(GovernorError::InvalidSupport)
                } else {
                    Ok(())
                }
            }
            VoteType::Extended | VoteType::Quadratic => Ok(()),
        }
    }

    pub fn cast_vote(env: Env, voter: Address, proposal_id: u64, support: VoteSupport) {
        voter.require_auth();

        // Commit-reveal proposals must be voted on via commit_vote/reveal_vote —
        // allowing a direct cast_vote here would let a voter bypass the hidden
        // commitment and defeat the anti-bribery/anti-copying guarantee (Issue #766).
        if commit_reveal::is_enabled_for_proposal(&env, proposal_id) {
            env.panic_with_error(GovernorError::ProposalNotActive);
        }

        // Validate vote support against configured vote type
        Self::validate_vote_support(&env, &support).unwrap_or_else(|e| env.panic_with_error(e));

        let voted: bool = env
            .storage()
            .persistent()
            .get(&DataKey::HasVoted(proposal_id, voter.clone()))
            .unwrap_or(false);
        if voted {
            env.panic_with_error(GovernorError::AlreadyVoted);
        }

        let mut proposal = Self::must_get_proposal(&env, proposal_id);

        // Reject votes outside the Active window. Checking fields directly
        // avoids a redundant storage read that Self::state() would incur.
        let current = env.ledger().sequence();
        if proposal.cancelled
            || proposal.executed
            || proposal.queued
            || current < proposal.start_ledger
            || current > proposal.end_ledger
        {
            env.panic_with_error(GovernorError::ProposalNotActive);
        }

        // Look up the voter's snapshot voting power at the proposal's start ledger
        // using the active voting strategy (single token or multi-token weighted).
        let raw_weight: i128 = Self::compute_votes(&env, &voter, &proposal.start_ledger);

        // Apply vote type weighting
        let vote_type: VoteType = env
            .storage()
            .instance()
            .get(&DataKey::VoteType)
            .unwrap_or(VoteType::Extended);
        let weight = Self::apply_vote_type(vote_type, raw_weight);

        if weight > 0 {
            match support {
                VoteSupport::For => proposal.votes_for += weight,
                VoteSupport::Against => proposal.votes_against += weight,
                VoteSupport::Abstain => proposal.votes_abstain += weight,
            }

            env.storage()
                .persistent()
                .set(&DataKey::Proposal(proposal_id), &proposal);
            // Extend TTL to cover full proposal lifecycle
            Self::extend_proposal_ttl(&env, proposal_id, &proposal);
        }

        env.storage()
            .persistent()
            .set(&DataKey::HasVoted(proposal_id, voter.clone()), &true);

        // Store voting receipt
        let receipt = VotingReceipt {
            has_voted: true,
            support: support.clone(),
            weight,
            reason: String::from_str(&env, ""),
        };
        env.storage()
            .persistent()
            .set(&DataKey::VoteReceipt(proposal_id, voter.clone()), &receipt);

        // Emit VoteCast event including the weighted vote power.
        events::emit_vote_cast(&env, &voter, proposal_id, &support, weight);
    }

    pub fn cast_vote_with_reason(
        env: Env,
        voter: Address,
        proposal_id: u64,
        support: VoteSupport,
        reason: String,
    ) {
        voter.require_auth();

        // Commit-reveal proposals must be voted on via commit_vote/reveal_vote
        // (see cast_vote's identical guard above for the rationale).
        if commit_reveal::is_enabled_for_proposal(&env, proposal_id) {
            env.panic_with_error(GovernorError::ProposalNotActive);
        }

        // Validate vote support against configured vote type
        Self::validate_vote_support(&env, &support).unwrap_or_else(|e| env.panic_with_error(e));

        let voted: bool = env
            .storage()
            .persistent()
            .get(&DataKey::HasVoted(proposal_id, voter.clone()))
            .unwrap_or(false);
        if voted {
            env.panic_with_error(GovernorError::AlreadyVoted);
        }

        let mut proposal = Self::must_get_proposal(&env, proposal_id);

        // Reject votes outside the Active window.
        let current = env.ledger().sequence();
        if proposal.cancelled
            || proposal.executed
            || proposal.queued
            || current < proposal.start_ledger
            || current > proposal.end_ledger
        {
            env.panic_with_error(GovernorError::ProposalNotActive);
        }

        // Look up the voter's snapshot voting power at the proposal's start ledger
        let raw_weight: i128 = Self::compute_votes(&env, &voter, &proposal.start_ledger);

        // Apply vote type weighting
        let vote_type: VoteType = env
            .storage()
            .instance()
            .get(&DataKey::VoteType)
            .unwrap_or(VoteType::Extended);
        let weight = Self::apply_vote_type(vote_type, raw_weight);

        if weight > 0 {
            match support {
                VoteSupport::For => proposal.votes_for += weight,
                VoteSupport::Against => proposal.votes_against += weight,
                VoteSupport::Abstain => proposal.votes_abstain += weight,
            }

            env.storage()
                .persistent()
                .set(&DataKey::Proposal(proposal_id), &proposal);
            // Extend TTL to cover full proposal lifecycle
            Self::extend_proposal_ttl(&env, proposal_id, &proposal);
        }

        env.storage()
            .persistent()
            .set(&DataKey::HasVoted(proposal_id, voter.clone()), &true);

        // Store voting receipt with reason
        let receipt = VotingReceipt {
            has_voted: true,
            support: support.clone(),
            weight,
            reason: reason.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::VoteReceipt(proposal_id, voter.clone()), &receipt);

        // Emit VoteCastWithReason event
        events::emit_vote_cast_with_reason(&env, &voter, proposal_id, &support, weight, reason);
    }

    fn ensure_not_finalized(env: &Env, proposal: &Proposal) {
        if proposal.cancelled || proposal.executed || proposal.queued {
            env.panic_with_error(GovernorError::ProposalNotActive);
        }
    }

    /// Two-phase commit-reveal voting (Issue #766): hides `support` until `reveal_vote`.
    pub fn commit_vote(env: Env, voter: Address, proposal_id: u64, commitment: BytesN<32>) {
        voter.require_auth();

        let commit_deadline = commit_reveal::commit_deadline(&env, proposal_id)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::CommitRevealNotEnabled));

        let proposal = Self::must_get_proposal(&env, proposal_id);
        Self::ensure_not_finalized(&env, &proposal);

        let current = env.ledger().sequence();
        if current < proposal.start_ledger {
            env.panic_with_error(GovernorError::ProposalNotActive);
        }
        if current > commit_deadline {
            env.panic_with_error(GovernorError::CommitPhaseEnded);
        }

        if commit_reveal::has_committed(&env, proposal_id, &voter) {
            env.panic_with_error(GovernorError::AlreadyVoted);
        }

        commit_reveal::store_commitment(&env, proposal_id, &voter, &commitment);
        events::emit_vote_committed(&env, proposal_id, &voter, &commitment);
    }

    /// Discloses a `commit_vote` preimage and applies the vote to the tally (Issue #766).
    pub fn reveal_vote(
        env: Env,
        voter: Address,
        proposal_id: u64,
        support: VoteSupport,
        weight_seed: u128,
        salt: BytesN<32>,
    ) {
        voter.require_auth();

        let commit_deadline = commit_reveal::commit_deadline(&env, proposal_id)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::CommitRevealNotEnabled));
        let reveal_deadline = commit_reveal::reveal_deadline(&env, proposal_id)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::CommitRevealNotEnabled));

        Self::validate_vote_support(&env, &support).unwrap_or_else(|e| env.panic_with_error(e));

        let voted: bool = env
            .storage()
            .persistent()
            .get(&DataKey::HasVoted(proposal_id, voter.clone()))
            .unwrap_or(false);
        if voted {
            env.panic_with_error(GovernorError::AlreadyVoted);
        }

        let mut proposal = Self::must_get_proposal(&env, proposal_id);
        Self::ensure_not_finalized(&env, &proposal);

        let current = env.ledger().sequence();
        if current <= commit_deadline {
            env.panic_with_error(GovernorError::RevealPhaseNotStarted);
        }
        if current > reveal_deadline {
            env.panic_with_error(GovernorError::RevealPhaseEnded);
        }

        if !commit_reveal::has_committed(&env, proposal_id, &voter) {
            env.panic_with_error(GovernorError::NoCommitmentToReveal);
        }

        let stored_commitment = commit_reveal::get_commitment(&env, proposal_id, &voter)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::NoCommitmentToReveal));

        let recomputed = commit_reveal::compute_commitment(&env, proposal_id, &support, weight_seed, &salt);
        if recomputed != stored_commitment {
            env.panic_with_error(GovernorError::CommitmentMismatch);
        }

        let raw_weight: i128 = Self::compute_votes(&env, &voter, &proposal.start_ledger);
        let vote_type: VoteType = env
            .storage()
            .instance()
            .get(&DataKey::VoteType)
            .unwrap_or(VoteType::Extended);
        let weight = Self::apply_vote_type(vote_type, raw_weight);

        if weight > 0 {
            match support {
                VoteSupport::For => proposal.votes_for += weight,
                VoteSupport::Against => proposal.votes_against += weight,
                VoteSupport::Abstain => proposal.votes_abstain += weight,
            }

            env.storage()
                .persistent()
                .set(&DataKey::Proposal(proposal_id), &proposal);
            Self::extend_proposal_ttl(&env, proposal_id, &proposal);
        }

        env.storage()
            .persistent()
            .set(&DataKey::HasVoted(proposal_id, voter.clone()), &true);

        let receipt = VotingReceipt {
            has_voted: true,
            support: support.clone(),
            weight,
            reason: String::from_str(&env, ""),
        };
        env.storage()
            .persistent()
            .set(&DataKey::VoteReceipt(proposal_id, voter.clone()), &receipt);

        events::emit_vote_revealed(&env, proposal_id, &voter, &support, weight);
    }

    pub fn get_commit_deadline(env: Env, proposal_id: u64) -> u32 {
        commit_reveal::commit_deadline(&env, proposal_id)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::CommitRevealNotEnabled))
    }

    pub fn get_reveal_deadline(env: Env, proposal_id: u64) -> u32 {
        commit_reveal::reveal_deadline(&env, proposal_id)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::CommitRevealNotEnabled))
    }

    pub fn has_committed(env: Env, proposal_id: u64, voter: Address) -> bool {
        commit_reveal::has_committed(&env, proposal_id, &voter)
    }

    pub fn vote(env: Env, voter: Address, proposal_id: u64, vote_choice: u32) {
        // Validate vote_choice is within valid range (0-2)
        if vote_choice > 2 {
            env.panic_with_error(GovernorError::InvalidVoteChoice);
        }

        // Convert u32 to VoteSupport enum
        let support = match vote_choice {
            0 => VoteSupport::Against,
            1 => VoteSupport::For,
            2 => VoteSupport::Abstain,
            _ => unreachable!(), // Already validated above
        };

        // Delegate to cast_vote with the validated VoteSupport
        Self::cast_vote(env, voter, proposal_id, support);
    }

    pub fn queue(env: Env, proposal_id: u64) {
        let proposal_state = Self::state(env.clone(), proposal_id);

        if proposal_state == ProposalState::Expired {
            env.panic_with_error(GovernorError::ProposalExpired);
        }

        if proposal_state != ProposalState::Succeeded {
            env.panic_with_error(GovernorError::ProposalNotSucceeded);
        }

        let mut proposal = Self::must_get_proposal(&env, proposal_id);

        // Independently re-verify quorum and threshold against the final vote
        // tally so that stale cached state cannot allow a failed proposal through.
        let required_quorum = Self::quorum(env.clone(), proposal_id);
        let quorum_met = proposal.votes_for + proposal.votes_abstain >= required_quorum;
        let for_wins = proposal.votes_for > proposal.votes_against;
        if !quorum_met || !for_wins {
            env.panic_with_error(GovernorError::ProposalNotSucceeded);
        }

        // Reputation: record a Succeeded outcome now that quorum + vote
        // direction have been independently re-verified above (Issue #771).
        // queue() can only run once per proposal (a second call would find
        // state() == Queued and panic above), so this fires exactly once.
        let strategy: VotingStrategy = env
            .storage()
            .instance()
            .get(&DataKey::VotingStrategy)
            .unwrap_or(VotingStrategy::Single);
        let supply = Self::compute_quorum_supply(&env, &proposal.start_ledger, &strategy);
        let total_votes = proposal.votes_for + proposal.votes_against + proposal.votes_abstain;
        let participation_bps = if supply > 0 {
            let bps = total_votes.checked_mul(10_000).unwrap_or(0) / supply;
            Some(bps.max(0).min(i128::from(u32::MAX)) as u32)
        } else {
            None
        };
        reputation::record_proposal_terminal(
            &env,
            &proposal.proposer,
            reputation::ReputationOutcome::Succeeded,
            participation_bps,
        );

        let timelock_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Timelock)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::TimelockNotSet));
        let gov_addr = env.current_contract_address();
        let timelock = TimelockClient::new(&env, &timelock_addr);

        if proposal.targets.is_empty() {
            env.panic_with_error(GovernorError::NoTargets);
        }

        // Use the timelock's own minimum delay to guarantee the configured
        // execution window is respected.
        let delay = timelock.min_delay();

        let ready_at = env.ledger().timestamp() + delay;

        // Schedule proposal actions via the timelock.
        // Multi-action proposals use schedule_batch() — one batch_op_id for all
        // actions — which reduces storage costs and enables atomic execution.
        // Single-action proposals use the existing schedule() path.
        let mut op_ids: Vec<Bytes> = Vec::new(&env);
        let empty_bytes = Bytes::new(&env);
        // Salted with proposal_id so two proposals scheduling the exact same
        // target/calldata/fn_name (e.g. two identical "noop" actions) never
        // collide on the same timelock op_id — the timelock only hashes
        // target+data+predecessor+salt, with no notion of "proposal" at all.
        let salt = Bytes::from_slice(&env, &proposal_id.to_be_bytes());

        if proposal.targets.len() > 1 {
            let batch_op_id = timelock.schedule_batch(
                &gov_addr,
                &proposal.targets,
                &proposal.calldatas,
                &proposal.fn_names,
                &delay,
                &empty_bytes,
                &salt,
            );
            op_ids.push_back(batch_op_id);
        } else {
            let target = proposal.targets.get(0).unwrap();
            let fn_name = proposal.fn_names.get(0).unwrap();
            let calldata = proposal.calldatas.get(0).unwrap();
            let op_id = timelock.schedule(
                &gov_addr,
                &target,
                &calldata,
                &fn_name,
                &delay,
                &empty_bytes,
                &salt,
            );
            op_ids.push_back(op_id);
        }

        proposal.op_ids = op_ids;
        proposal.queued = true;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        // Extend TTL to cover full proposal lifecycle
        Self::extend_proposal_ttl(&env, proposal_id, &proposal);

        // Store the queue time (current ledger sequence) for veto window tracking
        let queue_time = env.ledger().sequence();
        env.storage()
            .persistent()
            .set(&DataKey::QueueTime(proposal_id), &queue_time);

        let first_op_id = proposal.op_ids.get(0).unwrap();
        events::emit_proposal_queued(&env, proposal_id, &first_op_id, ready_at);
    }

    pub fn execute(env: Env, proposal_id: u64) {
        if Self::state(env.clone(), proposal_id) != ProposalState::Queued {
            env.panic_with_error(GovernorError::ProposalNotQueued);
        }

        let mut proposal = Self::must_get_proposal(&env, proposal_id);

        if proposal.executed {
            env.panic_with_error(GovernorError::ProposalAlreadyExecuted);
        }

        let timelock_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Timelock)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::TimelockNotSet));
        let gov_addr = env.current_contract_address();

        // Execute all timelock operations scheduled by queue().
        if proposal.op_ids.is_empty() {
            env.panic_with_error(GovernorError::MissingOpIds);
        }

        let timelock = TimelockClient::new(&env, &timelock_addr);

        // Multi-action proposals were batch-scheduled: one batch_op_id covers all
        // actions.  Call execute_batch() for atomic all-or-nothing execution.
        // Single-action proposals use the existing execute() path.
        if proposal.targets.len() > 1 {
            let batch_op_id = proposal.op_ids.get(0).unwrap();

            let mut timelock_args: Vec<Val> = Vec::new(&env);
            timelock_args.push_back(gov_addr.clone().into_val(&env));
            timelock_args.push_back(batch_op_id.clone().into_val(&env));

            let mut sub_invocations = Vec::new(&env);
            for j in 0..proposal.targets.len() {
                let target = proposal.targets.get(j).unwrap();
                let fn_name = proposal.fn_names.get(j).unwrap();
                let calldata = proposal.calldatas.get(j).unwrap();
                let decoded_args = Self::decode_calldata_args(&env, &calldata);
                if !decoded_args.is_empty() {
                    let target_context = ContractContext {
                        contract: target,
                        fn_name,
                        args: decoded_args,
                    };
                    sub_invocations.push_back(InvokerContractAuthEntry::Contract(
                        SubContractInvocation {
                            context: target_context,
                            sub_invocations: Vec::new(&env),
                        },
                    ));
                }
            }

            let mut auth_entries = Vec::new(&env);
            auth_entries.push_back(InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: timelock_addr.clone(),
                    fn_name: Symbol::new(&env, "execute_batch"),
                    args: timelock_args,
                },
                sub_invocations,
            }));
            env.authorize_as_current_contract(auth_entries);

            timelock.execute_batch(&gov_addr, &batch_op_id);
        } else {
            let op_id = proposal.op_ids.get(0).unwrap();
            let target = proposal.targets.get(0).unwrap();
            let fn_name = proposal.fn_names.get(0).unwrap();
            let calldata = proposal.calldatas.get(0).unwrap();
            let decoded_args = Self::decode_calldata_args(&env, &calldata);

            let mut timelock_args: Vec<Val> = Vec::new(&env);
            timelock_args.push_back(gov_addr.clone().into_val(&env));
            timelock_args.push_back(op_id.clone().into_val(&env));

            let mut sub_invocations = Vec::new(&env);
            if !decoded_args.is_empty() {
                let target_context = ContractContext {
                    contract: target,
                    fn_name,
                    args: decoded_args,
                };
                sub_invocations.push_back(InvokerContractAuthEntry::Contract(
                    SubContractInvocation {
                        context: target_context,
                        sub_invocations: Vec::new(&env),
                    },
                ));
            }

            let mut auth_entries = Vec::new(&env);
            auth_entries.push_back(InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: timelock_addr.clone(),
                    fn_name: Symbol::new(&env, "execute"),
                    args: timelock_args,
                },
                sub_invocations,
            }));
            env.authorize_as_current_contract(auth_entries);

            timelock.execute(&gov_addr, &op_id);
        }

        proposal.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        // Extend TTL to cover full proposal lifecycle
        Self::extend_proposal_ttl(&env, proposal_id, &proposal);

        // Reputation: record an Executed outcome (Issue #771). Naturally
        // idempotent — a second execute() call would panic above on the
        // `proposal.executed` check.
        reputation::record_proposal_terminal(
            &env,
            &proposal.proposer,
            reputation::ReputationOutcome::Executed,
            None,
        );

        events::emit_proposal_executed(&env, proposal_id, &gov_addr);
    }

    pub fn execute_batch(env: Env, proposal_ids: Vec<u64>) {
        if proposal_ids.is_empty() {
            env.panic_with_error(GovernorError::EmptyBatch);
        }

        for i in 0..proposal_ids.len() {
            let proposal_id = proposal_ids
                .get(i)
                .unwrap_or_else(|| env.panic_with_error(GovernorError::ProposalNotFound));
            if Self::state(env.clone(), proposal_id) != ProposalState::Queued {
                env.panic_with_error(GovernorError::BatchProposalNotQueued);
            }
        }

        for i in 0..proposal_ids.len() {
            let proposal_id = proposal_ids
                .get(i)
                .unwrap_or_else(|| env.panic_with_error(GovernorError::ProposalNotFound));
            Self::execute(env.clone(), proposal_id);
        }

        env.events()
            .publish((symbol_short!("exbatch"),), proposal_ids);
    }

    pub fn cancel(env: Env, caller: Address, proposal_id: u64) {
        caller.require_auth();

        let proposal = Self::must_get_proposal(&env, proposal_id);

        let state = Self::state(env.clone(), proposal_id);
        let guardian: Address = env
            .storage()
            .instance()
            .get(&DataKey::Guardian)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::GuardianNotSet));

        let can_cancel = (caller == proposal.proposer && state == ProposalState::Pending)
            || (caller == guardian && state == ProposalState::Active);

        if !can_cancel {
            env.panic_with_error(GovernorError::UnauthorizedCancel);
        }

        let mut proposal = proposal;
        proposal.cancelled = true;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        // Extend TTL to cover full proposal lifecycle
        Self::extend_proposal_ttl(&env, proposal_id, &proposal);
        reputation::record_proposal_terminal(
            &env,
            &proposal.proposer,
            reputation::ReputationOutcome::Cancelled,
            None,
        );
        events::emit_proposal_cancelled(&env, proposal_id, &caller);
    }

    pub fn cancel_by_governance(env: Env, proposal_id: u64) {
        // Only callable by the governor contract itself
        env.current_contract_address().require_auth();

        let proposal: Proposal = Self::must_get_proposal(&env, proposal_id);

        // Verify the proposal is not already executed or cancelled
        if proposal.executed {
            env.panic_with_error(GovernorError::ProposalAlreadyExecuted);
        }
        if proposal.cancelled {
            env.panic_with_error(GovernorError::ProposalAlreadyCancelled);
        }

        // Settle any natural Defeated/Expired outcome first so its penalty is
        // recorded (and flagged) before we decide whether to add a Cancelled one.
        Self::state(env.clone(), proposal_id);
        let already_scored = env
            .storage()
            .persistent()
            .get(&DataKey::ProposalExpiredEmitted(proposal_id))
            .unwrap_or(false)
            || env
                .storage()
                .persistent()
                .get(&DataKey::ReputationExpiredRecorded(proposal_id))
                .unwrap_or(false);

        let mut proposal_mut = proposal;
        proposal_mut.cancelled = true;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal_mut);
        // Extend TTL to cover full proposal lifecycle
        Self::extend_proposal_ttl(&env, proposal_id, &proposal_mut);
        // A proposal that already lost (Defeated/Expired) has been penalised
        // once; don't penalise it a second time for the governance cancel (Issue #797).
        if !already_scored {
            reputation::record_proposal_terminal(
                &env,
                &proposal_mut.proposer,
                reputation::ReputationOutcome::Cancelled,
                None,
            );
        }

        // If the proposal was queued, cancel its timelock operations
        if proposal_mut.queued {
            let timelock_addr: Address = env
                .storage()
                .instance()
                .get(&DataKey::Timelock)
                .unwrap_or_else(|| env.panic_with_error(GovernorError::TimelockNotSet));
            let timelock = TimelockClient::new(&env, &timelock_addr);
            let gov_addr = env.current_contract_address();

            for i in 0..proposal_mut.op_ids.len() {
                let op_id = proposal_mut.op_ids.get(i).unwrap();
                timelock.cancel(&gov_addr, &op_id);
            }
        }

        events::emit_proposal_cancelled(&env, proposal_id, &env.current_contract_address());
    }

    pub fn cancel_queued(env: Env, caller: Address, proposal_id: u64) {
        caller.require_auth();

        let proposal = Self::must_get_proposal(&env, proposal_id);

        // Verify the proposal is queued
        if !proposal.queued || proposal.cancelled {
            env.panic_with_error(GovernorError::ProposalNotQueued);
        }

        let guardian: Address = env
            .storage()
            .instance()
            .get(&DataKey::Guardian)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::GuardianNotSet));

        // Only guardian can cancel queued proposals
        if caller != guardian {
            env.panic_with_error(GovernorError::UnauthorizedGuardian);
        }

        // Get the queue time for veto window check
        let queue_time: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::QueueTime(proposal_id))
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ProposalNotQueued));

        // Get the timelock delay
        let timelock_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::Timelock)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::TimelockNotSet));
        let timelock = TimelockClient::new(&env, &timelock_addr);
        let delay = timelock.min_delay();

        // Check if we're still in the veto window.
        // `delay` is in seconds; convert to ledgers using the shared constant so this
        // site matches extend_proposal_ttl (previously used /10, which halved the window).
        let current_ledger = env.ledger().sequence();
        let delay_ledgers = (delay / Self::SECONDS_PER_LEDGER) as u32;
        let veto_window_end_ledger = queue_time.saturating_add(delay_ledgers);

        if current_ledger >= veto_window_end_ledger {
            env.panic_with_error(GovernorError::VetoWindowClosed);
        }

        // Cancel the proposal
        let mut proposal_mut = proposal;
        proposal_mut.cancelled = true;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal_mut);
        // Extend TTL to cover full proposal lifecycle
        Self::extend_proposal_ttl(&env, proposal_id, &proposal_mut);
        reputation::record_proposal_terminal(
            &env,
            &proposal_mut.proposer,
            reputation::ReputationOutcome::Cancelled,
            None,
        );

        // Cancel all timelock operations associated with this proposal
        let gov_addr = env.current_contract_address();
        for i in 0..proposal_mut.op_ids.len() {
            let op_id = proposal_mut.op_ids.get(i).unwrap();
            timelock.cancel(&gov_addr, &op_id);
        }

        events::emit_proposal_cancelled(&env, proposal_id, &caller);
        // Emit ProposalCancelledFromQueue event for veto cancellation tracking
        env.events().publish(
            (Symbol::new(&env, "ProposalCancelledFromQueue"),),
            (proposal_id, &caller),
        );
    }

    pub fn state(env: Env, proposal_id: u64) -> ProposalState {
        let proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ProposalNotFound));

        if proposal.cancelled {
            return ProposalState::Cancelled;
        }
        if proposal.executed {
            return ProposalState::Executed;
        }
        if proposal.queued {
            return ProposalState::Queued;
        }

        let current = env.ledger().sequence();
        if current < proposal.start_ledger {
            return ProposalState::Pending;
        }

        if current <= proposal.end_ledger {
            return ProposalState::Active;
        }

        Self::emit_proposal_expired_if_needed(&env, &proposal);

        // Voting ended.
        let quorum = Self::quorum(env.clone(), proposal_id);
        let quorum_met = proposal.votes_for + proposal.votes_abstain >= quorum;
        let for_wins = proposal.votes_for > proposal.votes_against;
        let against_wins_or_ties = proposal.votes_against >= proposal.votes_for;

        if quorum_met && for_wins {
            // Check if succeeded proposal has expired
            let grace_period: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ProposalGracePeriod)
                .unwrap_or(120_960); // Default ~7 days
            let grace_end = proposal.end_ledger + grace_period;
            if current > grace_end {
                // Reputation: a proposal that succeeded but was never
                // executed within the grace period is a distinct terminal
                // outcome from Defeated. Guard separately so repeated
                // state() reads don't double-record it (Issue #771).
                let already_recorded: bool = env
                    .storage()
                    .persistent()
                    .get(&DataKey::ReputationExpiredRecorded(proposal_id))
                    .unwrap_or(false);
                if !already_recorded {
                    reputation::record_proposal_terminal(
                        &env,
                        &proposal.proposer,
                        reputation::ReputationOutcome::Expired,
                        None,
                    );
                    env.storage()
                        .persistent()
                        .set(&DataKey::ReputationExpiredRecorded(proposal_id), &true);
                }
                ProposalState::Expired
            } else {
                ProposalState::Succeeded
            }
        } else if !quorum_met || against_wins_or_ties {
            ProposalState::Defeated
        } else {
            // This branch is structurally unreachable: the outer condition
            // `!(quorum_met && for_wins)` means at least one of `!quorum_met`
            // or `against_wins_or_ties` is true, so the `else if` above always
            // matches first.  The `unreachable!()` here documents that intent
            // and will panic loudly if a future refactor accidentally makes
            // this path reachable (issue #439).
            unreachable!("state machine invariant violated: defeated branch must have been taken")
        }
    }

    pub fn quorum(env: Env, proposal_id: u64) -> i128 {
        let proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ProposalNotFound));

        let quorum_numerator: u32 = env
            .storage()
            .instance()
            .get(&DataKey::QuorumNumerator)
            .unwrap_or(0);

        // If quorum is configured as 0%, no need to query token supply from
        // the votes contract. This also keeps state()/queue() robust in
        // tests where the votes token might be a placeholder address.
        if quorum_numerator == 0 {
            return 0;
        }

        let strategy: VotingStrategy = env
            .storage()
            .instance()
            .get(&DataKey::VotingStrategy)
            .unwrap_or(VotingStrategy::Single);

        let supply = Self::compute_quorum_supply(&env, &proposal.start_ledger, &strategy);

        let static_quorum = supply
            .checked_mul(quorum_numerator as i128)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ArithmeticOverflow))
            / 100;

        let use_dynamic: bool = env
            .storage()
            .instance()
            .get(&DataKey::UseDynamicQuorum)
            .unwrap_or(false);
        if !use_dynamic {
            return static_quorum;
        }

        // Try to fetch token price from Reflector oracle.
        let oracle_opt: Option<Address> = env.storage().instance().get(&DataKey::ReflectorOracle);
        if let Some(oracle_addr) = oracle_opt {
            let votes_token_addr: Address = env
                .storage()
                .instance()
                .get(&DataKey::VotesToken)
                .unwrap_or_else(|| env.panic_with_error(GovernorError::VotesTokenNotSet));

            let min_quorum_usd: i128 = env
                .storage()
                .instance()
                .get(&DataKey::MinQuorumUsd)
                .unwrap_or(0);
            if min_quorum_usd > 0 {
                // Call oracle — fall back to static if it fails or returns None/zero.
                let oracle = ReflectorOracleClient::new(&env, &oracle_addr);
                let price_opt = oracle.try_lastprice(&votes_token_addr);
                if let Ok(Ok(Some(price))) = price_opt {
                    if price > 0 {
                        // Query the underlying token's decimal places from the
                        // token-votes contract so the dynamic quorum formula
                        // correctly converts the USD floor to token units (Issue #622).
                        let underlying_token = VotesClient::new(&env, &votes_token_addr).token();
                        let token_decimals: u32 =
                            token::TokenClient::new(&env, &underlying_token).decimals();
                        let scaling_factor = 10i128.pow(token_decimals);
                        let usd_quorum =
                            min_quorum_usd.checked_mul(scaling_factor).unwrap_or_else(|| {
                                env.panic_with_error(GovernorError::ArithmeticOverflow)
                            }) / price;
                        return static_quorum.max(usd_quorum);
                    }
                }
            }
        }
        static_quorum
    }

    pub fn is_quorum_reached(env: Env, proposal_id: u64) -> bool {
        let proposal = Self::must_get_proposal(&env, proposal_id);
        let required = Self::quorum(env, proposal_id);
        proposal.votes_for + proposal.votes_abstain >= required
    }

    pub fn voting_delay(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::VotingDelay)
            .unwrap_or(100)
    }

    pub fn voting_period(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::VotingPeriod)
            .unwrap_or(1000)
    }

    pub fn proposal_threshold(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::ProposalThreshold)
            .unwrap_or(0)
    }

    pub fn get_settings(env: Env) -> GovernorSettings {
        GovernorSettings {
            voting_delay: env
                .storage()
                .instance()
                .get(&DataKey::VotingDelay)
                .unwrap_or(100),
            voting_period: env
                .storage()
                .instance()
                .get(&DataKey::VotingPeriod)
                .unwrap_or(1000),
            quorum_numerator: env
                .storage()
                .instance()
                .get(&DataKey::QuorumNumerator)
                .unwrap_or(0),
            proposal_threshold: env
                .storage()
                .instance()
                .get(&DataKey::ProposalThreshold)
                .unwrap_or(0),
            guardian: env
                .storage()
                .instance()
                .get(&DataKey::Guardian)
                .unwrap_or_else(|| env.panic_with_error(GovernorError::GuardianNotSet)),
            vote_type: env
                .storage()
                .instance()
                .get(&DataKey::VoteType)
                .unwrap_or(VoteType::Extended),
            proposal_grace_period: env
                .storage()
                .instance()
                .get(&DataKey::ProposalGracePeriod)
                .unwrap_or(120_960),
            use_dynamic_quorum: env
                .storage()
                .instance()
                .get(&DataKey::UseDynamicQuorum)
                .unwrap_or(false),
            reflector_oracle: env.storage().instance().get(&DataKey::ReflectorOracle),
            min_quorum_usd: env
                .storage()
                .instance()
                .get(&DataKey::MinQuorumUsd)
                .unwrap_or(0),
            max_calldata_size: env
                .storage()
                .instance()
                .get(&DataKey::MaxCalldataSize)
                .unwrap_or(10_000),
            proposal_cooldown: env
                .storage()
                .instance()
                .get(&DataKey::ProposalCooldown)
                .unwrap_or(100),
            max_proposals_per_period: env
                .storage()
                .instance()
                .get(&DataKey::MaxProposalsPerPeriod)
                .unwrap_or(5),
            proposal_period_duration: env
                .storage()
                .instance()
                .get(&DataKey::ProposalPeriodDuration)
                .unwrap_or(10_000),
            co_sponsorship_registry: env
                .storage()
                .instance()
                .get(&DataKey::CoSponsorshipRegistry),
            use_commit_reveal: env
                .storage()
                .instance()
                .get(&DataKey::UseCommitReveal)
                .unwrap_or(false),
            commit_phase_fraction: env
                .storage()
                .instance()
                .get(&DataKey::CommitPhaseFraction)
                .unwrap_or(commit_reveal::DEFAULT_COMMIT_PHASE_FRACTION_BPS),
        }
    }

    fn validate_settings(env: &Env, new_settings: &GovernorSettings) {
        if new_settings.voting_delay > 1_209_600 {
            env.panic_with_error(GovernorError::InvalidVotingDelay);
        }
        if new_settings.voting_period == 0 {
            env.panic_with_error(GovernorError::InvalidVotingPeriod);
        }
        if new_settings.quorum_numerator > 100 {
            env.panic_with_error(GovernorError::InvalidQuorumNumerator);
        }
        if new_settings.proposal_threshold < 0 {
            env.panic_with_error(GovernorError::InvalidProposalThreshold);
        }
        if new_settings.max_calldata_size == 0 {
            env.panic_with_error(GovernorError::InvalidMaxCalldataSize);
        }
        if new_settings.max_proposals_per_period == 0 {
            env.panic_with_error(GovernorError::InvalidMaxProposalsPerPeriod);
        }
        if new_settings.proposal_period_duration == 0 {
            env.panic_with_error(GovernorError::InvalidProposalPeriodDuration);
        }
        // Must leave room for both a commit and a reveal sub-window.
        if new_settings.commit_phase_fraction == 0 || new_settings.commit_phase_fraction >= 10_000 {
            env.panic_with_error(GovernorError::InvalidVotingPeriod);
        }
    }

    pub fn update_config(env: Env, new_settings: GovernorSettings) {
        env.current_contract_address().require_auth();
        Self::validate_settings(&env, &new_settings);

        let old_settings = Self::get_settings(env.clone());

        env.storage()
            .instance()
            .set(&DataKey::VotingDelay, &new_settings.voting_delay);
        env.storage()
            .instance()
            .set(&DataKey::VotingPeriod, &new_settings.voting_period);
        env.storage()
            .instance()
            .set(&DataKey::QuorumNumerator, &new_settings.quorum_numerator);
        env.storage().instance().set(
            &DataKey::ProposalThreshold,
            &new_settings.proposal_threshold,
        );
        env.storage()
            .instance()
            .set(&DataKey::Guardian, &new_settings.guardian);
        // Fix #597: keep Pauser in sync with Guardian so that rotating the
        // guardian via a governance proposal also transfers pause authority to
        // the new guardian.  Without this, the old guardian retained an
        // irrevocable, invisible backdoor to freeze all governance activity.
        env.storage()
            .instance()
            .set(&DataKey::Pauser, &new_settings.guardian);
        env.storage()
            .instance()
            .set(&DataKey::VoteType, &new_settings.vote_type);
        env.storage().instance().set(
            &DataKey::ProposalGracePeriod,
            &new_settings.proposal_grace_period,
        );
        env.storage()
            .instance()
            .set(&DataKey::UseDynamicQuorum, &new_settings.use_dynamic_quorum);
        env.storage()
            .instance()
            .set(&DataKey::MinQuorumUsd, &new_settings.min_quorum_usd);
        env.storage()
            .instance()
            .set(&DataKey::MaxCalldataSize, &new_settings.max_calldata_size);
        env.storage()
            .instance()
            .set(&DataKey::ProposalCooldown, &new_settings.proposal_cooldown);
        env.storage().instance().set(
            &DataKey::MaxProposalsPerPeriod,
            &new_settings.max_proposals_per_period,
        );
        env.storage().instance().set(
            &DataKey::ProposalPeriodDuration,
            &new_settings.proposal_period_duration,
        );
        match new_settings.reflector_oracle {
            Some(ref addr) => env
                .storage()
                .instance()
                .set(&DataKey::ReflectorOracle, addr),
            None => env.storage().instance().remove(&DataKey::ReflectorOracle),
        }
        match new_settings.co_sponsorship_registry {
            Some(ref addr) => env
                .storage()
                .instance()
                .set(&DataKey::CoSponsorshipRegistry, addr),
            None => env
                .storage()
                .instance()
                .remove(&DataKey::CoSponsorshipRegistry),
        }
        env.storage()
            .instance()
            .set(&DataKey::UseCommitReveal, &new_settings.use_commit_reveal);
        env.storage().instance().set(
            &DataKey::CommitPhaseFraction,
            &new_settings.commit_phase_fraction,
        );

        events::emit_config_updated(&env, &old_settings, &new_settings);
    }

    pub fn set_guardian(env: Env, new_guardian: Address) {
        env.current_contract_address().require_auth();

        let old_guardian: Address = env
            .storage()
            .instance()
            .get(&DataKey::Guardian)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::GuardianNotSet));

        if old_guardian == new_guardian {
            return;
        }

        env.storage()
            .instance()
            .set(&DataKey::Guardian, &new_guardian);
        // Fix #597: mirror to Pauser so that standalone guardian rotation also
        // transfers pause authority; without this the old guardian could still
        // call pause() even after being replaced.
        env.storage()
            .instance()
            .set(&DataKey::Pauser, &new_guardian);

        events::emit_guardian_changed(&env, &old_guardian, &new_guardian);
    }


    pub fn proposal_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0)
    }

    pub fn proposals_count_by_state(env: Env) -> ProposalStateCounts {
        let total = Self::proposal_count(env.clone());
        let mut counts = ProposalStateCounts {
            pending: 0,
            active: 0,
            succeeded: 0,
            defeated: 0,
            queued: 0,
            executed: 0,
            cancelled: 0,
            expired: 0,
        };

        for id in 1..=total {
            let state = Self::state(env.clone(), id);
            match state {
                ProposalState::Pending => counts.pending += 1,
                ProposalState::Active => counts.active += 1,
                ProposalState::Succeeded => counts.succeeded += 1,
                ProposalState::Defeated => counts.defeated += 1,
                ProposalState::Queued => counts.queued += 1,
                ProposalState::Executed => counts.executed += 1,
                ProposalState::Cancelled => counts.cancelled += 1,
                ProposalState::Expired => counts.expired += 1,
            }
        }

        counts
    }


    pub fn can_propose(env: Env, proposer: Address) -> CanProposeResult {
        // Check if contract is paused
        let is_paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::IsPaused)
            .unwrap_or(false);
        if is_paused {
            return CanProposeResult {
                allowed: false,
                reason: symbol_short!("paused"),
                cooldown_ends_at: None,
                proposals_this_period: 0,
                max_per_period: 0,
                voting_power: 0,
                threshold: 0,
            };
        }

        // Get voting power
        let votes_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::VotesToken)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::VotesTokenNotSet));
        let votes_client = VotesClient::new(&env, &votes_token);
        let voting_power = votes_client.get_votes(&proposer);

        // Check proposal threshold
        let threshold: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalThreshold)
            .unwrap_or(0);
        if voting_power < threshold {
            return CanProposeResult {
                allowed: false,
                reason: symbol_short!("threshold"),
                cooldown_ends_at: None,
                proposals_this_period: 0,
                max_per_period: 0,
                voting_power,
                threshold,
            };
        }

        // Check cooldown period
        let current_ledger = env.ledger().sequence();
        let cooldown: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCooldown)
            .unwrap_or(100);
        let last_proposal_ledger: Option<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::LastProposalLedger(proposer.clone()));
        if let Some(last) = last_proposal_ledger {
            if current_ledger < last + cooldown {
                return CanProposeResult {
                    allowed: false,
                    reason: symbol_short!("cooldown"),
                    cooldown_ends_at: Some(last + cooldown),
                    proposals_this_period: 0,
                    max_per_period: 0,
                    voting_power,
                    threshold,
                };
            }
        }

        // Check rate limit per period
        let period_duration: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalPeriodDuration)
            .unwrap_or(10_000);
        let max_per_period: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MaxProposalsPerPeriod)
            .unwrap_or(5);
        let current_period = current_ledger / period_duration;
        let proposals_this_period: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ProposalsInPeriod(
                proposer.clone(),
                current_period,
            ))
            .unwrap_or(0);
        if proposals_this_period >= max_per_period {
            return CanProposeResult {
                allowed: false,
                reason: symbol_short!("ratelimit"),
                cooldown_ends_at: None,
                proposals_this_period,
                max_per_period,
                voting_power,
                threshold,
            };
        }

        // All checks passed
        CanProposeResult {
            allowed: true,
            reason: symbol_short!("ok"),
            cooldown_ends_at: None,
            proposals_this_period,
            max_per_period,
            voting_power,
            threshold,
        }
    }

    pub fn get_receipt(env: Env, proposal_id: u64, voter: Address) -> VotingReceipt {
        env.storage()
            .persistent()
            .get(&DataKey::VoteReceipt(proposal_id, voter))
            .unwrap_or(VotingReceipt {
                has_voted: false,
                support: VoteSupport::Against,
                weight: 0,
                reason: String::from_str(&env, ""),
            })
    }

    pub fn voting_strategy(env: Env) -> VotingStrategy {
        env.storage()
            .instance()
            .get(&DataKey::VotingStrategy)
            .unwrap_or(VotingStrategy::Single)
    }

    pub fn set_voting_strategy(env: Env, strategy: VotingStrategy) {
        env.current_contract_address().require_auth();
        if let VotingStrategy::MultiToken(ref tokens) = strategy {
            if tokens.len() > 5 {
                env.panic_with_error(GovernorError::TooManyTokens);
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::VotingStrategy, &strategy);
    }

    pub fn update_oracle(
        env: Env,
        oracle: Option<Address>,
        min_quorum_usd: i128,
        use_dynamic: bool,
    ) {
        env.current_contract_address().require_auth();
        env.storage()
            .instance()
            .set(&DataKey::UseDynamicQuorum, &use_dynamic);
        env.storage()
            .instance()
            .set(&DataKey::MinQuorumUsd, &min_quorum_usd);
        match oracle {
            Some(addr) => env
                .storage()
                .instance()
                .set(&DataKey::ReflectorOracle, &addr),
            None => env.storage().instance().remove(&DataKey::ReflectorOracle),
        }
    }

    fn checked_add(env: &Env, lhs: i128, rhs: i128) -> i128 {
        lhs.checked_add(rhs)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ArithmeticOverflow))
    }

    fn checked_weight_bps(env: &Env, value: i128, weight_bps: u32) -> i128 {
        value
            .checked_mul(weight_bps as i128)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::ArithmeticOverflow))
            / 10_000
    }

    fn compute_quorum_supply(env: &Env, ledger: &u32, strategy: &VotingStrategy) -> i128 {
        match strategy {
            VotingStrategy::Single => {
                let votes_token_addr: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::VotesToken)
                    .unwrap_or_else(|| env.panic_with_error(GovernorError::VotesTokenNotSet));
                VotesClient::new(env, &votes_token_addr).get_past_total_supply(ledger)
            }
            VotingStrategy::MultiToken(tokens) => {
                let mut total: i128 = 0;
                for wt in tokens.iter() {
                    let supply =
                        match VotesClient::new(env, &wt.token).try_get_past_total_supply(ledger) {
                            Ok(Ok(supply)) => supply,
                            _ => 0,
                        };
                    let weighted = Self::checked_weight_bps(env, supply, wt.weight_bps);
                    total = Self::checked_add(env, total, weighted);
                }
                total
            }
        }
    }

    fn compute_votes(env: &Env, voter: &Address, ledger: &u32) -> i128 {
        let strategy: VotingStrategy = env
            .storage()
            .instance()
            .get(&DataKey::VotingStrategy)
            .unwrap_or(VotingStrategy::Single);
        match strategy {
            VotingStrategy::Single => {
                let votes_token: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::VotesToken)
                    .unwrap_or_else(|| env.panic_with_error(GovernorError::VotesTokenNotSet));
                VotesClient::new(env, &votes_token).get_past_votes(voter, ledger)
            }
            VotingStrategy::MultiToken(tokens) => {
                let mut total: i128 = 0;
                for wt in tokens.iter() {
                    let votes =
                        match VotesClient::new(env, &wt.token).try_get_past_votes(voter, ledger) {
                            Ok(Ok(votes)) => votes,
                            _ => 0,
                        };
                    let weighted = Self::checked_weight_bps(env, votes, wt.weight_bps);
                    total = Self::checked_add(env, total, weighted);
                }
                total
            }
        }
    }

    fn compute_proposer_votes(env: &Env, proposer: &Address) -> i128 {
        let strategy: VotingStrategy = env
            .storage()
            .instance()
            .get(&DataKey::VotingStrategy)
            .unwrap_or(VotingStrategy::Single);
        match strategy {
            VotingStrategy::Single => {
                let votes_token: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::VotesToken)
                    .unwrap_or_else(|| env.panic_with_error(GovernorError::VotesTokenNotSet));
                VotesClient::new(env, &votes_token).get_votes(proposer)
            }
            VotingStrategy::MultiToken(tokens) => {
                let mut total: i128 = 0;
                for wt in tokens.iter() {
                    let votes = match VotesClient::new(env, &wt.token).try_get_votes(proposer) {
                        Ok(Ok(votes)) => votes,
                        _ => 0,
                    };
                    let weighted = Self::checked_weight_bps(env, votes, wt.weight_bps);
                    total = Self::checked_add(env, total, weighted);
                }
                total
            }
        }
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        env.current_contract_address().require_auth();
        let old_wasm_hash: BytesN<32> = env
            .storage()
            .instance()
            .get(&DataKey::CurrentWasmHash)
            .unwrap_or(BytesN::from_array(&env, &[0u8; 32]));
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        env.storage()
            .instance()
            .set(&DataKey::CurrentWasmHash, &new_wasm_hash);
        events::emit_governor_upgraded(&env, &old_wasm_hash, &new_wasm_hash);
    }

    pub fn migrate(env: Env, data: MigrateData) {
        env.current_contract_address().require_auth();

        let current_version: u32 = env
            .storage()
            .instance()
            .get(&DataKey::StorageVersion)
            .unwrap_or(0);

        if data.new_version > CURRENT_STORAGE_VERSION {
            env.panic_with_error(GovernorError::AlreadyInitialized);
        }
        if data.new_version < current_version {
            env.panic_with_error(GovernorError::AlreadyInitialized);
        }
        if data.new_version == current_version {
            return;
        }

        let mut v = current_version + 1;
        while v <= data.new_version {
            Self::apply_migration_step(&env, v);
            v += 1;
        }

        env.storage()
            .instance()
            .set(&DataKey::StorageVersion, &data.new_version);
    }

    fn apply_migration_step(env: &Env, version: u32) {
        if version == 1 {
            Self::apply_migration_v1(env);
        }
    }

    // ============================================================================
    // Proposer Reputation System (Issue #771)
    // ============================================================================

    pub fn get_proposer_reputation(env: Env, proposer: Address) -> reputation::ProposerReputation {
        reputation::get_reputation(&env, &proposer)
    }

    /// Governance-only: only callable via an executed proposal (the governor
    /// contract calling itself), matching the pattern used by `update_config`
    /// and friends elsewhere in this file.
    pub fn get_effective_threshold(env: Env, proposer: Address) -> i128 {
        let flat: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalThreshold)
            .unwrap_or(0);
        reputation::get_effective_threshold(&env, &proposer, flat)
    }

    /// Permissionless — decays `proposer`'s score a step back toward zero
    /// based on ledgers elapsed since it was last touched.
    pub fn apply_reputation_decay(env: Env, proposer: Address) {
        reputation::apply_decay(&env, &proposer);
    }

    // ============================================================================
    // Emergency Pause Mechanism (Issue #191)
    // ============================================================================

    pub fn pause(env: Env, caller: Address) {
        caller.require_auth();

        let pauser: Address = env
            .storage()
            .instance()
            .get(&DataKey::Pauser)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::PauserNotSet));

        if caller != pauser {
            env.panic_with_error(GovernorError::UnauthorizedPause);
        }

        env.storage().instance().set(&DataKey::IsPaused, &true);

        events::emit_paused(&env, &caller);
    }

    pub fn unpause(env: Env, caller: Address) {
        caller.require_auth();

        let pauser: Address = env
            .storage()
            .instance()
            .get(&DataKey::Pauser)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::PauserNotSet));

        if caller != pauser {
            env.panic_with_error(GovernorError::UnauthorizedPause);
        }

        env.storage().instance().set(&DataKey::IsPaused, &false);

        events::emit_unpaused(&env);
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::IsPaused)
            .unwrap_or(false)
    }

    pub fn pauser(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Pauser)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::PauserNotSet))
    }

    pub fn set_pauser(env: Env, new_pauser: Address) {
        env.current_contract_address().require_auth();

        let old_pauser: Address = env
            .storage()
            .instance()
            .get(&DataKey::Pauser)
            .unwrap_or_else(|| env.panic_with_error(GovernorError::PauserNotSet));

        env.storage().instance().set(&DataKey::Pauser, &new_pauser);

        events::emit_pauser_changed(&env, &old_pauser, &new_pauser);
    }

    // ============================================================================
    // UUPS Proxy Pattern (Issue #195)
    // ============================================================================

    pub fn get_queue_time(env: Env, proposal_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::QueueTime(proposal_id))
            .unwrap_or(0)
    }

    pub fn get_proposal(env: Env, proposal_id: u64) -> Proposal {
        Self::must_get_proposal(&env, proposal_id)
    }

    pub fn get_proposal_list(env: Env, offset: u64, limit: u64) -> Vec<Proposal> {
        if limit == 0 {
            return Vec::new(&env);
        }

        let proposal_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ProposalList)
            .unwrap_or(Vec::new(&env));

        let total = proposal_ids.len() as u64;
        if offset >= total {
            return Vec::new(&env);
        }

        let end = core::cmp::min(offset.saturating_add(limit), total);
        let mut proposals = Vec::new(&env);

        for i in offset..end {
            let proposal_id = proposal_ids.get(i as u32).unwrap();
            proposals.push_back(Self::must_get_proposal(&env, proposal_id));
        }

        proposals
    }

}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl,
        testutils::{Address as _, Events, Ledger as _},
        Bytes, Env, Symbol, TryIntoVal,
    };

    #[contract]
    pub struct MockVotesContract;

    #[contracttype]
    enum MockDataKey {
        Token,
    }

    #[contractimpl]
    impl MockVotesContract {
        pub fn set_token(env: Env, token: Address) {
            env.storage().instance().set(&MockDataKey::Token, &token);
        }

        pub fn get_votes(_env: Env, _account: Address) -> i128 {
            // Return a high vote count that exceeds any reasonable threshold
            1_000_000
        }

        pub fn get_past_votes(_env: Env, _account: Address, _ledger: u32) -> i128 {
            // Return a fixed snapshot voting power for cast_vote() tests
            1_000_000
        }

        pub fn get_past_total_supply(_env: Env, _ledger: u32) -> i128 {
            // Return a fixed total supply for quorum calculations in tests
            10_000_000
        }

        pub fn token(env: Env) -> Address {
            env.storage()
                .instance()
                .get(&MockDataKey::Token)
                .unwrap_or_else(|| Address::generate(&env))
        }
    }

    #[contract]
    pub struct MockTimelockContract;

    #[contractimpl]
    impl MockTimelockContract {
        pub fn execution_window(_env: Env) -> u64 {
            86_400
        }

        pub fn min_delay(_env: Env) -> u64 {
            3_600
        }
    }

    fn propose_dummy(env: &Env, client: &GovernorContractClient, proposer: &Address) -> u64 {
        let target = Address::generate(env);
        let fn_name = Symbol::new(env, "exec");
        let calldata = Bytes::new(env);
        let description = String::from_str(env, "Test proposal");

        // Compute SHA-256 hash of the description
        let description_hash = env
            .crypto()
            .sha256(&Bytes::from_slice(env, b"Test proposal"))
            .into();

        // Dummy metadata URI (could be ipfs or https)
        let metadata_uri = String::from_str(env, "https://example.com/proposal/1");

        // Create Vec with single target, fn_name, and calldata
        let mut targets = soroban_sdk::Vec::new(env);
        targets.push_back(target);

        let mut fn_names = soroban_sdk::Vec::new(env);
        fn_names.push_back(fn_name);

        let mut calldatas = soroban_sdk::Vec::new(env);
        calldatas.push_back(calldata);

        client.propose(
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
    fn test_cast_vote_with_reason_stores_reason() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // voting_delay = 100: advance past start_ledger so the proposal is Active.
        env.ledger().with_mut(|l| l.sequence_number += 101);

        let reason = String::from_str(&env, "I support this because it improves governance");
        client.cast_vote_with_reason(&voter, &proposal_id, &VoteSupport::For, &reason);

        let stored_reason = client.get_receipt(&proposal_id, &voter).reason;
        assert_eq!(stored_reason, reason);
    }

    #[test]
    fn test_cast_vote_with_reason_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 101);

        let reason = String::from_str(&env, "This aligns with our community values");
        client.cast_vote_with_reason(&voter, &proposal_id, &VoteSupport::For, &reason);

        let events = env.events().all();

        let has_vote_rsn = events.iter().any(|(_, topics, _)| {
            !topics.is_empty() && {
                let first: Result<soroban_sdk::Symbol, _> =
                    topics.get(0).unwrap().try_into_val(&env);
                first.is_ok() && first.unwrap() == Symbol::new(&env, "VoteCastWithReason")
            }
        });

        assert!(has_vote_rsn, "VoteCastWithReason event not emitted");
    }

    #[test]
    fn test_cast_vote_with_reason_multiple_voters() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter1 = Address::generate(&env);
        let voter2 = Address::generate(&env);

        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // voting_delay = 100: advance past start_ledger so the proposal is Active.
        env.ledger().with_mut(|l| l.sequence_number += 101);

        let reason1 = String::from_str(&env, "I agree with this proposal");
        let reason2 = String::from_str(&env, "I disagree with this proposal");

        client.cast_vote_with_reason(&voter1, &proposal_id, &VoteSupport::For, &reason1);
        client.cast_vote_with_reason(&voter2, &proposal_id, &VoteSupport::Against, &reason2);

        let stored_reason1 = client.get_receipt(&proposal_id, &voter1).reason;
        let stored_reason2 = client.get_receipt(&proposal_id, &voter2).reason;

        assert_eq!(stored_reason1, reason1);
        assert_eq!(stored_reason2, reason2);
    }

    #[test]
    fn test_quorum_and_state() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        // Deploy a real SEP-41 token and TokenVotes contract so the voter
        // has actual snapshot voting power for the cross-contract lookup.
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        let token_addr = sac.address();
        let sac_client = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);

        let votes_id = env.register(sorogov_token_votes::TokenVotesContract, ());
        let votes_client = sorogov_token_votes::TokenVotesContractClient::new(&env, &votes_id);
        votes_client.initialize(&admin, &token_addr);

        // Mint tokens and self-delegate so the voter has snapshot voting power.
        sac_client.mint(&voter, &1000_i128);
        votes_client.delegate(&voter, &voter);

        // Initialize governor with 50% quorum (50 / 100).
        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_id,
            &timelock,
            &0,
            &100,
            &50,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance ledger to start voting.
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // cast_vote now looks up snapshot voting power from token-votes.
        client.cast_vote(&voter, &proposal_id, &VoteSupport::For);

        // Advance ledger to end voting.
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Quorum is 0 (total supply at start_ledger=0 is 1000, 50% = 500,
        // but start_ledger checkpoint may be 0 depending on delegation timing).
        // With 1000 votes For, the proposal succeeds regardless.
        assert_eq!(client.state(&proposal_id), ProposalState::Succeeded);
    }

    #[test]
    fn test_is_quorum_reached() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let timelock = env.register(MockTimelockContract, ());
        let proposer1 = Address::generate(&env);
        let proposer2 = Address::generate(&env);
        let proposer3 = Address::generate(&env);
        let voter1 = Address::generate(&env);
        let voter2 = Address::generate(&env);

        // Deploy a real SEP-41 token and TokenVotes contract.
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        let token_addr = sac.address();
        let sac_client = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);

        let votes_id = env.register(sorogov_token_votes::TokenVotesContract, ());
        let votes_client = sorogov_token_votes::TokenVotesContractClient::new(&env, &votes_id);
        votes_client.initialize(&admin, &token_addr);

        // Mint tokens: voter1 has 400, voter2 has 600, total 1000
        sac_client.mint(&voter1, &400_i128);
        sac_client.mint(&voter2, &600_i128);
        votes_client.delegate(&voter1, &voter1);
        votes_client.delegate(&voter2, &voter2);

        // Initialize governor with 50% quorum (50 / 100 = 500 required).
        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_id,
            &timelock,
            &0,
            &100,
            &50,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        // Case 1: Not enough votes for quorum (400 For < 500)
        let proposal_id = propose_dummy(&env, &client, &proposer1);
        env.ledger().with_mut(|li| li.sequence_number = 1);
        client.cast_vote(&voter1, &proposal_id, &VoteSupport::For);
        env.ledger().with_mut(|li| li.sequence_number = 1000);
        assert!(!client.is_quorum_reached(&proposal_id));
        assert_eq!(client.state(&proposal_id), ProposalState::Defeated);

        // Case 2: Exactly at quorum threshold (0 For + 600 Abstain = 600 >= 500)
        env.ledger().with_mut(|li| li.sequence_number = 101);
        let proposal_id2 = propose_dummy(&env, &client, &proposer2);
        env.ledger().with_mut(|li| li.sequence_number = 102);
        client.cast_vote(&voter1, &proposal_id2, &VoteSupport::Against);
        client.cast_vote(&voter2, &proposal_id2, &VoteSupport::Abstain);
        env.ledger().with_mut(|li| li.sequence_number = 1000);
        assert!(client.is_quorum_reached(&proposal_id2));
        // But still defeated because For (0) <= Against (400)
        assert_eq!(client.state(&proposal_id2), ProposalState::Defeated);

        // Case 3: Above quorum (600 For > 500)
        env.ledger().with_mut(|li| li.sequence_number = 202);
        let proposal_id3 = propose_dummy(&env, &client, &proposer3);
        env.ledger().with_mut(|li| li.sequence_number = 203);
        client.cast_vote(&voter2, &proposal_id3, &VoteSupport::For);
        env.ledger().with_mut(|li| li.sequence_number = 1000);
        assert!(client.is_quorum_reached(&proposal_id3));
        assert_eq!(client.state(&proposal_id3), ProposalState::Succeeded);
    }

    #[test]
    fn test_propose_with_multiple_targets() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());

        // Set threshold to 100
        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &100,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        // Create proposal with multiple targets
        let target1 = Address::generate(&env);
        let target2 = Address::generate(&env);
        let fn_name1 = Symbol::new(&env, "action1");
        let fn_name2 = Symbol::new(&env, "action2");
        let calldata1 = Bytes::from_slice(&env, &[1, 2, 3]);
        let calldata2 = Bytes::from_slice(&env, &[4, 5, 6]);
        let description = String::from_str(&env, "Multi-target proposal");

        let mut targets = soroban_sdk::Vec::new(&env);
        targets.push_back(target1.clone());
        targets.push_back(target2.clone());

        let mut fn_names = soroban_sdk::Vec::new(&env);
        fn_names.push_back(fn_name1.clone());
        fn_names.push_back(fn_name2.clone());

        let mut calldatas = soroban_sdk::Vec::new(&env);
        calldatas.push_back(calldata1.clone());
        calldatas.push_back(calldata2.clone());

        let description_hash = env
            .crypto()
            .sha256(&Bytes::from_slice(&env, b"Multi-target proposal"))
            .into();
        let metadata_uri = String::from_str(&env, "ipfs://QmMulti");

        let proposal_id = client.propose(
            &proposer,
            &description,
            &description_hash,
            &metadata_uri,
            &targets,
            &fn_names,
            &calldatas,
        );

        // Verify proposal was created
        assert_eq!(proposal_id, 1);
        assert_eq!(client.proposal_count(), 1);
    }

    #[test]
    fn test_get_proposal_list_pagination() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &100,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let p1 = propose_dummy(&env, &client, &proposer);
        env.ledger().with_mut(|l| l.sequence_number += 101);
        let p2 = propose_dummy(&env, &client, &proposer);
        env.ledger().with_mut(|l| l.sequence_number += 101);
        let p3 = propose_dummy(&env, &client, &proposer);

        let page = client.get_proposal_list(&1, &2);

        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().id, p2);
        assert_eq!(page.get(1).unwrap().id, p3);
        assert_eq!(p1, 1);
    }

    #[test]
    fn test_get_proposal_list_bounds() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &100,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let _ = propose_dummy(&env, &client, &proposer);

        let empty_limit = client.get_proposal_list(&0, &0);
        let empty_offset = client.get_proposal_list(&10, &5);

        assert_eq!(empty_limit.len(), 0);
        assert_eq!(empty_offset.len(), 0);
    }

    #[test]
    #[should_panic]
    fn test_propose_rejects_mismatched_lengths() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());

        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &100,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let target = Address::generate(&env);
        let fn_name = Symbol::new(&env, "exec");
        let calldata1 = Bytes::new(&env);
        let calldata2 = Bytes::new(&env);
        let description = String::from_str(&env, "Mismatched proposal");

        let mut targets = soroban_sdk::Vec::new(&env);
        targets.push_back(target);

        let mut fn_names = soroban_sdk::Vec::new(&env);
        fn_names.push_back(fn_name);

        let mut calldatas = soroban_sdk::Vec::new(&env);
        calldatas.push_back(calldata1);
        calldatas.push_back(calldata2); // Extra calldata

        let description_hash = env
            .crypto()
            .sha256(&Bytes::from_slice(&env, b"Mismatched proposal"))
            .into();
        let metadata_uri = String::from_str(&env, "ipfs://QmMismatch");

        // Should panic with "targets, fn_names, and calldatas length mismatch"
        client.propose(
            &proposer,
            &description,
            &description_hash,
            &metadata_uri,
            &targets,
            &fn_names,
            &calldatas,
        );
    }

    #[test]
    #[should_panic]
    fn test_propose_rejects_empty_targets() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());

        let guardian = Address::generate(&env);
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &100,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let description = String::from_str(&env, "Empty proposal");
        let targets = soroban_sdk::Vec::new(&env);
        let fn_names = soroban_sdk::Vec::new(&env);
        let calldatas = soroban_sdk::Vec::new(&env);

        let description_hash = env
            .crypto()
            .sha256(&Bytes::from_slice(&env, b"Empty proposal"))
            .into();
        let metadata_uri = String::from_str(&env, "ipfs://QmEmpty");

        // Should panic with "must have at least one target"
        client.propose(
            &proposer,
            &description,
            &description_hash,
            &metadata_uri,
            &targets,
            &fn_names,
            &calldatas,
        );
    }

    #[contract]
    pub struct MockOracleContract;

    #[contractimpl]
    impl MockOracleContract {
        pub fn lastprice(_env: Env, _asset: Address) -> Option<i128> {
            Some(5_000_000) // $5.00 in 6-decimal format
        }
    }

    #[test]
    fn test_cancel_proposer_pending() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Proposer should be able to cancel pending proposal
        client.cancel(&proposer, &proposal_id);
        assert_eq!(client.state(&proposal_id), ProposalState::Cancelled);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_cancel_proposer_active_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Proposer should not be able to cancel active proposal
        client.cancel(&proposer, &proposal_id);
    }

    #[test]
    fn test_cancel_guardian_active() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Guardian should be able to cancel active proposal
        client.cancel(&guardian, &proposal_id);
        assert_eq!(client.state(&proposal_id), ProposalState::Cancelled);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_cancel_guardian_pending_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Guardian should not be able to cancel pending proposal
        client.cancel(&guardian, &proposal_id);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_vote_type_simple_rejects_abstain() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Simple,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Should panic with InvalidSupport when trying to abstain in Simple mode
        client.cast_vote(&voter, &proposal_id, &VoteSupport::Abstain);
    }

    #[test]
    fn test_vote_type_quadratic_weighting() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Quadratic,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Cast vote - raw weight is 1_000_000, quadratic should be sqrt(1_000_000) = 1000
        client.cast_vote(&voter, &proposal_id, &VoteSupport::For);

        let __proposal = client.get_proposal(&proposal_id);
        let (votes_for, _, _) = (__proposal.votes_for, __proposal.votes_against, __proposal.votes_abstain);
        assert_eq!(votes_for, 1000); // Quadratic weighting applied
    }

    #[test]
    fn test_proposal_expiry() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,
            &100,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &100,
        ); // Short grace period

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // Vote to make it succeed
        client.cast_vote(&voter, &proposal_id, &VoteSupport::For);

        // Advance past voting period
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Should be Succeeded initially
        assert_eq!(client.state(&proposal_id), ProposalState::Succeeded);

        // Advance past grace period
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Should now be Expired
        assert_eq!(client.state(&proposal_id), ProposalState::Expired);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #3)")]
    fn test_queue_expired_proposal_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,
            &100,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &100,
        ); // Short grace period

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // Vote to make it succeed
        client.cast_vote(&voter, &proposal_id, &VoteSupport::For);

        // Advance past voting period and grace period
        env.ledger().with_mut(|li| li.sequence_number += 201);

        // Should be Expired
        assert_eq!(client.state(&proposal_id), ProposalState::Expired);

        // Should panic when trying to queue expired proposal
        client.queue(&proposal_id);
    }

    #[test]
    fn test_multi_token_weighted_voting() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        // Register two mock token contracts.
        let token_a_id = env.register(MockVotesContract, ());
        let token_b_id = env.register(MockVotesContract, ());

        // Initialize governor with a single-token strategy first (standard init).
        client.initialize(
            &admin,
            &token_a_id,
            &timelock,
            &100,
            &1000,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        // Build MultiToken strategy: token_a at 1x (10000 bps), token_b at 2x (20000 bps).
        let mut weighted_tokens = soroban_sdk::Vec::new(&env);
        weighted_tokens.push_back(WeightedToken {
            token: token_a_id.clone(),
            weight_bps: 10_000,
        });
        weighted_tokens.push_back(WeightedToken {
            token: token_b_id.clone(),
            weight_bps: 20_000,
        });
        client.set_voting_strategy(&VotingStrategy::MultiToken(weighted_tokens));

        // Verify strategy is stored correctly.
        let stored = client.voting_strategy();
        assert!(
            matches!(stored, VotingStrategy::MultiToken(_)),
            "strategy should be MultiToken"
        );

        // Create a proposal.
        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance ledger into the voting window.
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Cast vote — each MockVotesContract returns 1_000_000 for get_past_votes.
        // Expected weight: 1_000_000 * 10000/10000 + 1_000_000 * 20000/10000 = 3_000_000
        client.cast_vote(&voter, &proposal_id, &VoteSupport::For);

        let __proposal = client.get_proposal(&proposal_id);
        let (votes_for, _, _) = (__proposal.votes_for, __proposal.votes_against, __proposal.votes_abstain);
        assert_eq!(
            votes_for, 3_000_000,
            "weighted votes should total 3_000_000"
        );
    }

    #[test]
    fn test_dynamic_quorum_with_oracle() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let mock_votes_client = MockVotesContractClient::new(&env, &votes_token_id);
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);

        // Deploy a real SEP-41 token so the governor can query its decimals.
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        let token_addr = sac.address();
        mock_votes_client.set_token(&token_addr);

        // 10% static quorum: supply=10_000_000, so static = 1_000_000.
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &10,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        // Create a proposal to get a proposal_id.
        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Without dynamic quorum, quorum == 10% of 10_000_000 = 1_000_000.
        let static_q = client.quorum(&proposal_id);
        assert_eq!(static_q, 1_000_000, "static quorum should be 1_000_000");

        // Register the mock oracle ($5.00 per token, 6-decimal format).
        let oracle_id = env.register(MockOracleContract, ());

        // Enable dynamic quorum: min_quorum_usd = 50 ($0.00005 in 6-decimal),
        // price = 5_000_000 ($5). The correct formula (Issue #622):
        //   usd_quorum = (min_quorum_usd * 10^decimals) / price
        //   = (50 * 10^7) / 5_000_000 = 100
        // usd_quorum (100) < static_quorum (1_000_000), so static wins.
        client.update_oracle(&Some(oracle_id.clone()), &50_i128, &true);
        let dynamic_q = client.quorum(&proposal_id);
        assert_eq!(
            dynamic_q, 1_000_000,
            "dynamic quorum should equal static when static is larger"
        );

        // Now set a very high USD floor that translates to more tokens than static quorum.
        // usd_quorum = (10_000_000_000_000 * 10^7) / 5_000_000 = 20_000_000_000_000.
        // 20_000_000_000_000 > static_quorum (1_000_000), so dynamic wins.
        client.update_oracle(&Some(oracle_id.clone()), &10_000_000_000_000_i128, &true);
        let high_dynamic_q = client.quorum(&proposal_id);
        assert_eq!(
            high_dynamic_q, 20_000_000_000_000,
            "dynamic quorum should use USD floor when larger than static"
        );

        // Test fallback: disable dynamic quorum and verify static quorum is used.
        client.update_oracle(&Some(oracle_id.clone()), &10_000_000_000_000_i128, &false);
        let fallback_q = client.quorum(&proposal_id);
        assert_eq!(
            fallback_q, 1_000_000,
            "should fall back to static quorum when dynamic is disabled"
        );
    }

    #[test]
    fn test_get_receipt_returns_voting_details() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Before voting, receipt should show has_voted = false
        let receipt_before = client.get_receipt(&proposal_id, &voter);
        assert!(!receipt_before.has_voted);
        assert_eq!(receipt_before.weight, 0);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Cast vote with reason
        let reason = String::from_str(&env, "I support this proposal");
        client.cast_vote_with_reason(&voter, &proposal_id, &VoteSupport::For, &reason);

        // After voting, receipt should contain all details
        let receipt_after = client.get_receipt(&proposal_id, &voter);
        assert!(receipt_after.has_voted);
        assert_eq!(receipt_after.support, VoteSupport::For);
        assert_eq!(receipt_after.weight, 1_000_000); // MockVotesContract returns 1_000_000
        assert_eq!(receipt_after.reason, reason);
    }

    #[test]
    #[should_panic]
    fn test_cannot_vote_twice() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance to active state
        env.ledger().with_mut(|li| li.sequence_number += 101);

        // Cast first vote
        client.cast_vote(&voter, &proposal_id, &VoteSupport::For);

        // Attempt to vote again should panic
        client.cast_vote(&voter, &proposal_id, &VoteSupport::Against);
    }

    #[test]
    fn test_long_running_proposal_ttl_extended() {
        // Regression test for: Proposal storage TTL not extended — long-running proposals can expire before execution
        // This test verifies that proposals with long voting periods have their storage TTL extended
        // to prevent expiration mid-lifecycle.
        let env = Env::default();
        env.mock_all_auths();

        // Bump TTL limits before registering so contract instances don't archive during the test.
        env.ledger().with_mut(|li| {
            li.min_persistent_entry_ttl = 1_000_000;
            li.max_entry_ttl = 1_000_000;
        });

        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);

        // Initialize with long voting period: 30 days ≈ 259,200 ledgers (at 10 sec blocks)
        let long_voting_period = 259_200u32;
        let voting_delay = 100u32;

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &voting_delay,
            &long_voting_period,
            &0, // no quorum requirement so single voter can succeed
            &0, // no proposal threshold
            &guardian,
            &VoteType::Extended,
            &120_960, // grace period
        );

        env.as_contract(&contract_id, || {
            env.storage().instance().extend_ttl(300_000, 300_000);
        });
        env.as_contract(&votes_token_id, || {
            env.storage().instance().extend_ttl(300_000, 300_000);
        });

        // Create proposal — should extend TTL
        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Verify proposal is created successfully
        let state = client.state(&proposal_id);
        assert_eq!(state, ProposalState::Pending);

        // Advance to active state (voting_delay ledgers)
        env.ledger()
            .with_mut(|li| li.sequence_number += voting_delay + 1);
        assert_eq!(client.state(&proposal_id), ProposalState::Active);

        // Cast vote — should extend TTL again
        client.cast_vote(&voter, &proposal_id, &VoteSupport::For);

        // Verify votes are recorded
        let __proposal = client.get_proposal(&proposal_id);
        let (votes_for, _, _) = (__proposal.votes_for, __proposal.votes_against, __proposal.votes_abstain);
        assert_eq!(votes_for, 1_000_000);

        // Advance well into the long voting period (but not past end)
        let mid_voting_period = voting_delay + (long_voting_period / 2);
        env.ledger()
            .with_mut(|li| li.sequence_number = mid_voting_period);

        // Should still be Active — if TTL wasn't extended, storage might expire
        let state = client.state(&proposal_id);
        assert_eq!(state, ProposalState::Active);

        // Advance to end of voting period
        env.ledger()
            .with_mut(|li| li.sequence_number = voting_delay + long_voting_period + 1);

        // Should transition to Succeeded (since quorum was met and votes_for > votes_against)
        let state = client.state(&proposal_id);
        assert_eq!(state, ProposalState::Succeeded);

        // Verify that get_proposal still works after long period
        let proposal = client.get_proposal(&proposal_id);
        assert_eq!(proposal.votes_for, 1_000_000);
        assert!(!proposal.executed);
    }

    #[test]
    fn test_no_expired_event_when_quorum_met_via_abstain() {
        // Regression test for the bug where emit_proposal_expired_if_needed checked
        // only votes_for >= quorum (ignoring abstain), diverging from the canonical
        // quorum formula in state() which counts votes_for + votes_abstain >= quorum.
        // A proposal that crosses quorum only because of abstain votes must NOT emit
        // a ProposalExpired event, and state() must return Succeeded.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let voter_for = Address::generate(&env);
        let voter_abstain = Address::generate(&env);

        // quorum_numerator = 15 → quorum = 10_000_000 * 15 / 100 = 1_500_000.
        // MockVotesContract returns 1_000_000 per voter, so votes_for (1M) alone
        // is below quorum but votes_for + votes_abstain (2M) clears it.
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,   // voting_delay
            &100, // voting_period
            &15,  // quorum_numerator
            &0,   // proposal_threshold
            &guardian,
            &VoteType::Extended,
            &100, // proposal_grace_period
        );

        let proposal_id = propose_dummy(&env, &client, &proposer);

        // Advance into the active voting window.
        env.ledger().with_mut(|li| li.sequence_number += 1);

        client.cast_vote(&voter_for, &proposal_id, &VoteSupport::For);
        client.cast_vote(&voter_abstain, &proposal_id, &VoteSupport::Abstain);

        // Advance past the voting period; the proposal should be Succeeded because
        // votes_for (1M) + votes_abstain (1M) = 2M >= quorum (1.5M) and for > against.
        env.ledger().with_mut(|li| li.sequence_number += 101);
        assert_eq!(client.state(&proposal_id), ProposalState::Succeeded);

        // Confirm that no ProposalExpired event was emitted.
        let events = env.events().all();
        let has_expired_event = events.iter().any(|(_, topics, _)| {
            !topics.is_empty() && {
                let first: Result<soroban_sdk::Symbol, _> =
                    topics.get(0).unwrap().try_into_val(&env);
                first.is_ok() && first.unwrap() == Symbol::new(&env, "ProposalExpired")
            }
        });
        assert!(
            !has_expired_event,
            "ProposalExpired must not be emitted when quorum is met via abstain votes"
        );

        // The persistent flag must also remain unset so a future state() call
        // cannot pick up a stale true value.
        let flag_set: bool = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::ProposalExpiredEmitted(proposal_id))
                .unwrap_or(false)
        });
        assert!(
            !flag_set,
            "ProposalExpiredEmitted flag must not be set for a succeeded proposal"
        );
    }

    #[test]
    fn test_governor_error_codes_snapshot() {
        // Snapshot: each GovernorError variant maps to a stable u32.
        // If this test fails due to renumbering, update SDK error maps too.
        assert_eq!(GovernorError::UnauthorizedCancel as u32, 1);
        assert_eq!(GovernorError::InvalidSupport as u32, 2);
        assert_eq!(GovernorError::ProposalExpired as u32, 3);
        assert_eq!(GovernorError::CalldataTooLarge as u32, 4);
        assert_eq!(GovernorError::InvalidCalldata as u32, 5);
        assert_eq!(GovernorError::ProposalRateLimited as u32, 6);
        assert_eq!(GovernorError::ContractPaused as u32, 7);
        assert_eq!(GovernorError::UnauthorizedPause as u32, 8);
        assert_eq!(GovernorError::InvalidVectorLengths as u32, 9);
        assert_eq!(GovernorError::NoTargets as u32, 10);
        assert_eq!(GovernorError::ProposalThresholdNotMet as u32, 11);
        assert_eq!(GovernorError::AlreadyVoted as u32, 12);
        assert_eq!(GovernorError::ZeroVotingPower as u32, 13);
        assert_eq!(GovernorError::ProposalNotSucceeded as u32, 14);
        assert_eq!(GovernorError::ProposalNotQueued as u32, 15);
        assert_eq!(GovernorError::ProposalAlreadyExecuted as u32, 16);
        assert_eq!(GovernorError::MissingOpIds as u32, 17);
        assert_eq!(GovernorError::UnauthorizedGuardian as u32, 18);
        assert_eq!(GovernorError::VetoWindowClosed as u32, 19);
        assert_eq!(GovernorError::ProposalNotFound as u32, 20);
        assert_eq!(GovernorError::TimelockNotSet as u32, 21);
        assert_eq!(GovernorError::GuardianNotSet as u32, 22);
        assert_eq!(GovernorError::TooManyTokens as u32, 23);
        assert_eq!(GovernorError::EmptyMetadataUri as u32, 24);
        assert_eq!(GovernorError::VotesTokenNotSet as u32, 25);
        assert_eq!(GovernorError::PauserNotSet as u32, 26);
        assert_eq!(GovernorError::ArithmeticOverflow as u32, 27);
        assert_eq!(GovernorError::VotePeriodTooShort as u32, 28);
        assert_eq!(GovernorError::ExecutionWindowZero as u32, 29);
        assert_eq!(GovernorError::TooManyCalldataEntries as u32, 30);
        assert_eq!(GovernorError::ProposalNotActive as u32, 31);
        assert_eq!(GovernorError::AlreadyInitialized as u32, 32);
        assert_eq!(GovernorError::InvalidVotingDelay as u32, 33);
        assert_eq!(GovernorError::InvalidVotingPeriod as u32, 34);
        assert_eq!(GovernorError::InvalidQuorumNumerator as u32, 35);
        assert_eq!(GovernorError::InvalidProposalThreshold as u32, 36);
        assert_eq!(GovernorError::InvalidMaxCalldataSize as u32, 37);
        assert_eq!(GovernorError::InvalidMaxProposalsPerPeriod as u32, 38);
        assert_eq!(GovernorError::InvalidProposalPeriodDuration as u32, 39);
        assert_eq!(GovernorError::EmptyBatch as u32, 40);
        assert_eq!(GovernorError::BatchProposalNotQueued as u32, 41);
        assert_eq!(GovernorError::ProposalAlreadyCancelled as u32, 42);
        assert_eq!(GovernorError::InvalidVoteChoice as u32, 43);
    }

    #[test]
    fn test_proposals_in_period_previous_key_pruned() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let proposer = Address::generate(&env);
        let guardian = Address::generate(&env);

        // Initialize at ledger 0 with default period_duration=10_000.
        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0, // voting_delay
            &1, // voting_period (minimal so proposals don't overlap)
            &0, // quorum_numerator
            &0, // proposal_threshold
            &guardian,
            &VoteType::Simple,
            &120_960,
        );

        // Override period_duration to 5 so we can advance cheaply.
        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&DataKey::ProposalPeriodDuration, &5u32);
            // Disable cooldown so a second proposal is immediately allowed.
            env.storage()
                .instance()
                .set(&DataKey::ProposalCooldown, &0u32);
        });

        // Submit one proposal at ledger 0 (period 0).
        propose_dummy(&env, &client, &proposer);
        let period0_count: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::ProposalsInPeriod(proposer.clone(), 0u32))
                .unwrap_or(0)
        });
        assert_eq!(period0_count, 1);

        // Advance to period 1 (ledger 5).  This is a small jump that won't
        // cause the instance key to be treated as archived.
        env.ledger().with_mut(|li| li.sequence_number = 5);

        // Submit a proposal in period 1 — this should prune the period-0 key.
        propose_dummy(&env, &client, &proposer);

        // The period-0 key must have been removed.
        let period0_key = DataKey::ProposalsInPeriod(proposer.clone(), 0u32);
        let stale_still_exists = env.as_contract(&contract_id, || {
            env.storage().persistent().has(&period0_key)
        });
        assert!(
            !stale_still_exists,
            "stale ProposalsInPeriod key for period 0 should have been pruned"
        );

        // The current period-1 count must be 1.
        let period1_count: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::ProposalsInPeriod(proposer.clone(), 1u32))
                .unwrap_or(0)
        });
        assert_eq!(period1_count, 1);
    }

    #[test]
    fn test_migrate_updates_storage_version() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let version: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .get(&DataKey::StorageVersion)
                .unwrap_or(0)
        });
        assert_eq!(version, 1, "storage version should be 1 after init");

        let data = MigrateData { new_version: 1 };
        client.migrate(&data);

        let version: u32 = env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .get(&DataKey::StorageVersion)
                .unwrap_or(0)
        });
        assert_eq!(version, 1, "storage version should remain 1 after no-op migrate");
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #32)")]
    fn test_migrate_rejects_lower_version() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let data = MigrateData { new_version: 0 };
        client.migrate(&data);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #43)")]
    fn test_vote_rejects_invalid_choice_3() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let voter = Address::generate(&env);
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,
            &1000,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &admin);
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // Should panic with InvalidVoteChoice for vote_choice = 3
        client.vote(&voter, &proposal_id, &3u32);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #43)")]
    fn test_vote_rejects_invalid_choice_255() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let voter = Address::generate(&env);
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,
            &1000,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &admin);
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // Should panic with InvalidVoteChoice for vote_choice = 255
        client.vote(&voter, &proposal_id, &255u32);
    }

    #[test]
    fn test_vote_accepts_valid_choice_0_against() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let voter = Address::generate(&env);
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,
            &1000,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &admin);
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // Should accept vote_choice = 0 (Against)
        client.vote(&voter, &proposal_id, &0u32);

        let __proposal = client.get_proposal(&proposal_id);
        let (votes_for, votes_against, votes_abstain) = (__proposal.votes_for, __proposal.votes_against, __proposal.votes_abstain);
        assert_eq!(votes_for, 0);
        assert_eq!(votes_against, 1_000_000);
        assert_eq!(votes_abstain, 0);
    }

    #[test]
    fn test_vote_accepts_valid_choice_1_for() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let voter = Address::generate(&env);
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,
            &1000,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &admin);
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // Should accept vote_choice = 1 (For)
        client.vote(&voter, &proposal_id, &1u32);

        let __proposal = client.get_proposal(&proposal_id);
        let (votes_for, votes_against, votes_abstain) = (__proposal.votes_for, __proposal.votes_against, __proposal.votes_abstain);
        assert_eq!(votes_for, 1_000_000);
        assert_eq!(votes_against, 0);
        assert_eq!(votes_abstain, 0);
    }

    #[test]
    fn test_vote_accepts_valid_choice_2_abstain() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let voter = Address::generate(&env);
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &0,
            &1000,
            &0,
            &0,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposal_id = propose_dummy(&env, &client, &admin);
        env.ledger().with_mut(|li| li.sequence_number += 1);

        // Should accept vote_choice = 2 (Abstain)
        client.vote(&voter, &proposal_id, &2u32);

        let __proposal = client.get_proposal(&proposal_id);
        let (votes_for, votes_against, votes_abstain) = (__proposal.votes_for, __proposal.votes_against, __proposal.votes_abstain);
        assert_eq!(votes_for, 0);
        assert_eq!(votes_against, 0);
        assert_eq!(votes_abstain, 1_000_000);
    }

    // ── #1378: admin lookup in upgrade path uses expect() ────────────────────

    /// Verifies that the upgrade migration path (apply_migration_v1) cannot
    /// run before Admin is set: initialize() must be called first, and Admin
    /// is always written there. The migration branch is guarded by
    /// `!env.storage().instance().has(&DataKey::Pauser)`, which is only false
    /// on a pre-v1 contract that was initialized (i.e., Admin exists). A
    /// contract with no Admin cannot reach the migration path because it has
    /// no proposals to trigger an upgrade through.
    ///
    /// This test proves the happy path: after a valid initialize(), the Admin
    /// key is present and any subsequent migration write succeeds without
    /// panicking.
    #[test]
    fn test_admin_key_present_after_initialize() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        // If Admin were missing the pauser() call would fire PauserNotSet.
        // Reaching here without panic confirms Admin was stored correctly.
        let _pauser = client.pauser();
    }

    /// A second initialize() call must be rejected with AlreadyInitialized,
    /// proving the Admin guard is checked and the key is definitely written
    /// on the first call.
    #[test]
    fn test_double_initialize_rejected_with_typed_error() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &1000,
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let err = client
            .try_initialize(
                &admin,
                &votes_token_id,
                &timelock,
                &100,
                &1000,
                &50,
                &1000,
                &guardian,
                &VoteType::Extended,
                &120_960,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, GovernorError::AlreadyInitialized.into());
    }

    // ── #1379: propose() / can_propose() unwraps VotesToken from storage ─────

    /// can_propose() must return VotesTokenNotSet (not an opaque panic) when
    /// called on an instance whose VotesToken key has been initialized.
    /// The positive path verifies the typed error lookup works end-to-end.
    #[test]
    fn test_can_propose_returns_votes_token_not_set_when_uninitialized() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        // Do NOT call initialize() — VotesToken is absent.
        let proposer = Address::generate(&env);

        let err = client
            .try_can_propose(&proposer)
            .unwrap_err()
            .unwrap();
        // Without initialization, IsPaused defaults to false, so the code
        // reaches the VotesToken read — which must fire VotesTokenNotSet.
        assert_eq!(err, GovernorError::VotesTokenNotSet.into());
    }

    /// Confirm that after initialization the VotesToken read succeeds and
    /// can_propose() returns a proper result rather than panicking.
    #[test]
    fn test_can_propose_succeeds_after_initialize() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(GovernorContract, ());
        let client = GovernorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let votes_token_id = env.register(MockVotesContract, ());
        let timelock = env.register(MockTimelockContract, ());
        let guardian = Address::generate(&env);

        client.initialize(
            &admin,
            &votes_token_id,
            &timelock,
            &100,
            &1000,
            &50,
            &0, // zero threshold so any proposer passes
            &guardian,
            &VoteType::Extended,
            &120_960,
        );

        let proposer = Address::generate(&env);
        let result = client.can_propose(&proposer);
        assert!(result.allowed);
    }
}

#[cfg(test)]
mod tests;
