//! Deterministic description of a governance simulation scenario.
//!
//! A [`Scenario`] is pure data (serializable to/from JSON) describing a cast
//! of [`SimActor`]s and an ordered list of [`SimStep`]s to execute against a
//! freshly-deployed governor/timelock/token-votes/treasury stack. See
//! `tools/sim/src/scenarios/*.json` for examples.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub seed: u64,
    pub governor_settings: SimGovernorSettings,
    pub actors: Vec<SimActor>,
    pub steps: Vec<SimStep>,
}

/// Governor configuration values a scenario can set at genesis or via
/// `SimStep::UpdateConfig`. Distinct from the on-chain `GovernorSettings`
/// struct so scenario JSON stays stable even if the contract's settings
/// shape changes; the runner maps this onto the real struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimGovernorSettings {
    pub voting_delay: u32,
    pub voting_period: u32,
    pub quorum_numerator: u32,
    pub proposal_threshold: i128,
    pub vote_type: SimVoteType,
    pub proposal_grace_period: u32,
    #[serde(default = "default_max_calldata_size")]
    pub max_calldata_size: u32,
    #[serde(default)]
    pub proposal_cooldown: u32,
    #[serde(default = "default_max_proposals_per_period")]
    pub max_proposals_per_period: u32,
    #[serde(default = "default_proposal_period_duration")]
    pub proposal_period_duration: u32,
}

fn default_max_calldata_size() -> u32 {
    10_000
}
fn default_max_proposals_per_period() -> u32 {
    5
}
fn default_proposal_period_duration() -> u32 {
    10_000
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SimVoteType {
    Simple,
    Extended,
    Quadratic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimActor {
    pub name: String,
    pub initial_balance: i128,
    pub delegate_to: Option<String>,
    pub role: ActorRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorRole {
    TokenHolder,
    Proposer,
    Guardian,
    Pauser,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SimStep {
    AdvanceLedger {
        ledgers: u32,
    },
    Propose {
        actor: String,
        targets: Vec<String>,
        fn_names: Vec<String>,
        description: String,
    },
    Vote {
        actor: String,
        proposal_id: u64,
        support: SimVoteSupport,
    },
    Queue {
        actor: String,
        proposal_id: u64,
    },
    Execute {
        actor: String,
        proposal_id: u64,
    },
    Cancel {
        actor: String,
        proposal_id: u64,
    },
    Delegate {
        actor: String,
        delegatee: String,
    },
    DelegateSplit {
        actor: String,
        splits: Vec<(String, u32)>,
    },
    MintTokens {
        actor: String,
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    BurnTokens {
        actor: String,
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    UpdateConfig {
        actor: String,
        settings: SimGovernorSettings,
    },
    PauseContract {
        actor: String,
    },
    UnpauseContract {
        actor: String,
    },
    ExpectState {
        proposal_id: u64,
        expected_state: SimProposalState,
    },
    ExpectError {
        step_index: usize,
        expected_error: String,
    },
    TakeAnalyticsSnapshot,
    AssertParticipation {
        proposal_id: u64,
        min_bps: u32,
    },
    AssertQuorumReached {
        proposal_id: u64,
    },
    AssertReputationScore {
        actor: String,
        min_score: Option<i32>,
        max_score: Option<i32>,
    },
    CreateDraft {
        actor: String,
        targets: Vec<String>,
        fn_names: Vec<String>,
        description: String,
    },
    CoSponsorDraft {
        actor: String,
        draft_id: u64,
    },
    FinalizeDraft {
        actor: String,
        draft_id: u64,
    },
    CancelDraft {
        actor: String,
        draft_id: u64,
    },
    ScheduleBatch {
        actor: String,
        targets: Vec<String>,
        fn_names: Vec<String>,
        delay: u64,
    },
    ExecuteBatch {
        actor: String,
        schedule_step_index: usize,
    },
    ValidateDag {
        schedule_step_indices: Vec<usize>,
    },
    ConvictionCreateProposal {
        actor: String,
        target: String,
        fn_name: String,
        calldata: Option<String>,
        #[serde(with = "i128_compat")]
        requested_amount: i128,
    },
    ConvictionStake {
        actor: String,
        proposal_id: u64,
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    ConvictionWithdrawStake {
        actor: String,
    },
    ConvictionCheckpoint {
        proposal_id: u64,
    },
    LockProposalBond {
        actor: String,
        description: String,
    },
    RefundProposalBond {
        actor: String,
        description: String,
        proposal_id: u64,
    },
    ProposeBondSlash {
        actor: String,
        bonded_description: String,
        recipient: String,
        description: String,
    },
    ExpectBondState {
        description: String,
        expected_state: SimBondState,
    },
    CreateVoteEscrowLock {
        actor: String,
        #[serde(with = "i128_compat")]
        amount: i128,
        duration_ledgers: u32,
    },
    IncreaseVoteEscrowLock {
        actor: String,
        #[serde(with = "i128_compat")]
        additional_amount: i128,
    },
    ExtendVoteEscrowLock {
        actor: String,
        new_end_ledger: u32,
    },
    WithdrawVoteEscrowLock {
        actor: String,
    },
    ExpectVotingPower {
        actor: String,
        #[serde(with = "i128_compat")]
        expected_power: i128,
    },
    ExpectPastVotingPower {
        actor: String,
        ledger: u32,
        #[serde(with = "i128_compat")]
        expected_power: i128,
    },
    ExpectPastTotalSupply {
        ledger: u32,
        #[serde(with = "i128_compat")]
        expected_total: i128,
    },
    AnchorResult {
        actor: String,
        poll_id: u64,
        /// Hashed with sha256 (same treatment as `LockProposalBond`'s
        /// `description`) to produce the on-chain `result_hash` — the sim
        /// harness has no off-chain tallying to hash a real result from.
        result_seed: String,
    },
    ExpectAnchor {
        poll_id: u64,
        result_seed: String,
    },
    /// Register and initialize the harness's single liquidity pool (asset A /
    /// asset B), called as the governor like `UpdateConfig`.
    CreateLiquidityPool {
        fee_bps: u32,
    },
    MintPoolTokens {
        actor: String,
        #[serde(with = "i128_compat")]
        amount_a: i128,
        #[serde(with = "i128_compat")]
        amount_b: i128,
    },
    AddLiquidity {
        actor: String,
        #[serde(with = "i128_compat")]
        amount_a: i128,
        #[serde(with = "i128_compat")]
        amount_b: i128,
        #[serde(default, with = "i128_compat")]
        min_lp_tokens_out: i128,
    },
    RemoveLiquidity {
        actor: String,
        #[serde(with = "i128_compat")]
        lp_tokens: i128,
    },
    Swap {
        actor: String,
        asset_in: SimPoolAsset,
        #[serde(with = "i128_compat")]
        amount_in: i128,
        #[serde(default, with = "i128_compat")]
        min_amount_out: i128,
    },
    /// Change the pool fee, called as the governor like `UpdateConfig`.
    UpdatePoolFee {
        fee_bps: u32,
    },
    ExpectPool {
        #[serde(with = "i128_compat")]
        reserve_a: i128,
        #[serde(with = "i128_compat")]
        reserve_b: i128,
        #[serde(with = "i128_compat")]
        total_lp_supply: i128,
        fee_bps: u32,
    },
    ExpectLpShares {
        actor: String,
        #[serde(with = "i128_compat")]
        lp_tokens: i128,
    },
    ExpectPoolTokenBalance {
        actor: String,
        #[serde(with = "i128_compat")]
        balance_a: i128,
        #[serde(with = "i128_compat")]
        balance_b: i128,
    },
    /// Assert the pool's product per LP share (`reserve_a * reserve_b /
    /// total_lp_supply^2`) did not decrease across the most recent
    /// `AddLiquidity` / `RemoveLiquidity` / `Swap`. For a swap (LP supply
    /// unchanged) this is the constant-product invariant `k' >= k`; for an
    /// add or remove it means existing LPs' claim on the pool was not
    /// diluted. `expect_growth` requires a strict increase (fee accrual).
    ExpectPoolInvariant {
        #[serde(default)]
        expect_growth: bool,
    },
    /// Mint `amount` of the harness's reward asset to `actor`, then have
    /// `actor` pay it into the voting-rewards pool via `fund_pool`.
    FundRewardsPool {
        actor: String,
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    StartNextRewardsEpoch,
    /// Build a Merkle tree over `allocations` (leaves as the contract's
    /// `merkle::compute_leaf`) and publish its root for `epoch_id`, called as
    /// the governor (the contract's admin) like `UpdateConfig`.
    PublishRewardsRoot {
        epoch_id: u64,
        #[serde(with = "i128_compat")]
        total_reward_amount: i128,
        allocations: Vec<SimRewardAllocation>,
    },
    /// Claim `amount` from `epoch_id` with the proof of `actor`'s own leaf in
    /// that epoch's published allocation (empty if it has none), so claiming
    /// any amount other than the allocated one fails proof verification.
    ClaimReward {
        actor: String,
        epoch_id: u64,
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    ExpectRewardsEpoch {
        epoch_id: u64,
        start_ledger: u32,
        end_ledger: u32,
        #[serde(with = "i128_compat")]
        total_reward_amount: i128,
        #[serde(with = "i128_compat")]
        claimed_amount: i128,
        finalized: bool,
    },
    ExpectCurrentRewardsEpoch {
        epoch_id: u64,
    },
    ExpectAvailableRewardsPool {
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    ExpectRewardClaimed {
        actor: String,
        epoch_id: u64,
        claimed: bool,
    },
    ExpectRewardBalance {
        actor: String,
        #[serde(with = "i128_compat")]
        balance: i128,
    },
    /// Register a strategy for the harness token on the harness's mock
    /// adapter, called as the treasury-strategies admin. Strategy ids are
    /// assigned sequentially from 1.
    RegisterTreasuryStrategy {
        max_allocation_bps: u32,
        withdrawal_cooldown_ledgers: u32,
    },
    /// Mint `amount` of the harness token to the treasury contract.
    FundTreasury {
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    /// Move `amount` out of the treasury into the least-allocated active
    /// strategy, called as the treasury like an outbound multisig transfer.
    DepositToTreasuryStrategy {
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    /// Request a withdrawal of `amount` from `strategy_id`, called as the
    /// treasury. Withdrawal ids are assigned sequentially from 1.
    RequestTreasuryWithdrawal {
        strategy_id: u64,
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    /// Claim `withdrawal_id` once its cooldown has elapsed. Permissionless.
    ClaimTreasuryWithdrawal {
        withdrawal_id: u64,
    },
    ExpectTreasuryAllocation {
        strategy_id: u64,
        #[serde(with = "i128_compat")]
        amount: i128,
    },
    ExpectTreasuryBalance {
        #[serde(with = "i128_compat")]
        balance: i128,
    },
    ExpectTreasuryTotalValue {
        #[serde(with = "i128_compat")]
        total: i128,
    },
}

impl SimStep {
    /// Short tag used in reports (`StepResult::step_type`) and CLI output.
    pub fn kind(&self) -> &'static str {
        match self {
            SimStep::AdvanceLedger { .. } => "AdvanceLedger",
            SimStep::Propose { .. } => "Propose",
            SimStep::Vote { .. } => "Vote",
            SimStep::Queue { .. } => "Queue",
            SimStep::Execute { .. } => "Execute",
            SimStep::Cancel { .. } => "Cancel",
            SimStep::Delegate { .. } => "Delegate",
            SimStep::DelegateSplit { .. } => "DelegateSplit",
            SimStep::MintTokens { .. } => "MintTokens",
            SimStep::BurnTokens { .. } => "BurnTokens",
            SimStep::UpdateConfig { .. } => "UpdateConfig",
            SimStep::PauseContract { .. } => "PauseContract",
            SimStep::UnpauseContract { .. } => "UnpauseContract",
            SimStep::ExpectState { .. } => "ExpectState",
            SimStep::ExpectError { .. } => "ExpectError",
            SimStep::TakeAnalyticsSnapshot => "TakeAnalyticsSnapshot",
            SimStep::AssertParticipation { .. } => "AssertParticipation",
            SimStep::AssertQuorumReached { .. } => "AssertQuorumReached",
            SimStep::AssertReputationScore { .. } => "AssertReputationScore",
            SimStep::CreateDraft { .. } => "CreateDraft",
            SimStep::CoSponsorDraft { .. } => "CoSponsorDraft",
            SimStep::FinalizeDraft { .. } => "FinalizeDraft",
            SimStep::CancelDraft { .. } => "CancelDraft",
            SimStep::ScheduleBatch { .. } => "ScheduleBatch",
            SimStep::ExecuteBatch { .. } => "ExecuteBatch",
            SimStep::ValidateDag { .. } => "ValidateDag",
            SimStep::ConvictionCreateProposal { .. } => "ConvictionCreateProposal",
            SimStep::ConvictionStake { .. } => "ConvictionStake",
            SimStep::ConvictionWithdrawStake { .. } => "ConvictionWithdrawStake",
            SimStep::ConvictionCheckpoint { .. } => "ConvictionCheckpoint",
            SimStep::LockProposalBond { .. } => "LockProposalBond",
            SimStep::RefundProposalBond { .. } => "RefundProposalBond",
            SimStep::ProposeBondSlash { .. } => "ProposeBondSlash",
            SimStep::ExpectBondState { .. } => "ExpectBondState",
            SimStep::CreateVoteEscrowLock { .. } => "CreateVoteEscrowLock",
            SimStep::IncreaseVoteEscrowLock { .. } => "IncreaseVoteEscrowLock",
            SimStep::ExtendVoteEscrowLock { .. } => "ExtendVoteEscrowLock",
            SimStep::WithdrawVoteEscrowLock { .. } => "WithdrawVoteEscrowLock",
            SimStep::ExpectVotingPower { .. } => "ExpectVotingPower",
            SimStep::ExpectPastVotingPower { .. } => "ExpectPastVotingPower",
            SimStep::ExpectPastTotalSupply { .. } => "ExpectPastTotalSupply",
            SimStep::AnchorResult { .. } => "AnchorResult",
            SimStep::ExpectAnchor { .. } => "ExpectAnchor",
            SimStep::CreateLiquidityPool { .. } => "CreateLiquidityPool",
            SimStep::MintPoolTokens { .. } => "MintPoolTokens",
            SimStep::AddLiquidity { .. } => "AddLiquidity",
            SimStep::RemoveLiquidity { .. } => "RemoveLiquidity",
            SimStep::Swap { .. } => "Swap",
            SimStep::UpdatePoolFee { .. } => "UpdatePoolFee",
            SimStep::ExpectPool { .. } => "ExpectPool",
            SimStep::ExpectLpShares { .. } => "ExpectLpShares",
            SimStep::ExpectPoolTokenBalance { .. } => "ExpectPoolTokenBalance",
            SimStep::ExpectPoolInvariant { .. } => "ExpectPoolInvariant",
            SimStep::FundRewardsPool { .. } => "FundRewardsPool",
            SimStep::StartNextRewardsEpoch => "StartNextRewardsEpoch",
            SimStep::PublishRewardsRoot { .. } => "PublishRewardsRoot",
            SimStep::ClaimReward { .. } => "ClaimReward",
            SimStep::ExpectRewardsEpoch { .. } => "ExpectRewardsEpoch",
            SimStep::ExpectCurrentRewardsEpoch { .. } => "ExpectCurrentRewardsEpoch",
            SimStep::ExpectAvailableRewardsPool { .. } => "ExpectAvailableRewardsPool",
            SimStep::ExpectRewardClaimed { .. } => "ExpectRewardClaimed",
            SimStep::ExpectRewardBalance { .. } => "ExpectRewardBalance",
            SimStep::RegisterTreasuryStrategy { .. } => "RegisterTreasuryStrategy",
            SimStep::FundTreasury { .. } => "FundTreasury",
            SimStep::DepositToTreasuryStrategy { .. } => "DepositToTreasuryStrategy",
            SimStep::RequestTreasuryWithdrawal { .. } => "RequestTreasuryWithdrawal",
            SimStep::ClaimTreasuryWithdrawal { .. } => "ClaimTreasuryWithdrawal",
            SimStep::ExpectTreasuryAllocation { .. } => "ExpectTreasuryAllocation",
            SimStep::ExpectTreasuryBalance { .. } => "ExpectTreasuryBalance",
            SimStep::ExpectTreasuryTotalValue { .. } => "ExpectTreasuryTotalValue",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimVoteSupport {
    Against,
    For,
    Abstain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimProposalState {
    Pending,
    Active,
    Defeated,
    Succeeded,
    Queued,
    Executed,
    Cancelled,
    Expired,
}

/// One `(actor, amount)` leaf of a voting-rewards epoch's Merkle tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimRewardAllocation {
    pub actor: String,
    #[serde(with = "i128_compat")]
    pub amount: i128,
}

/// Which side of the harness's liquidity pool a `Swap` sells into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimPoolAsset {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimBondState {
    Locked,
    Refunded,
    Slashed,
}

/// `serde`'s content deserializer for internally-tagged enums does not
/// forward `deserialize_i128`. Accept the JSON integer widths it does
/// support (and decimal strings for the full i128 range), then widen here.
mod i128_compat {
    use super::*;
    use serde::de::{Error, Visitor};
    use std::fmt;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<i128, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct I128Visitor;

        impl<'de> Visitor<'de> for I128Visitor {
            type Value = i128;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an integer or a base-10 i128 string")
            }

            fn visit_i64<E: Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(i128::from(value))
            }

            fn visit_u64<E: Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(i128::from(value))
            }

            fn visit_str<E: Error>(self, value: &str) -> Result<Self::Value, E> {
                value.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_any(I128Visitor)
    }

    pub fn serialize<S>(value: &i128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Ok(value) = i64::try_from(*value) {
            serializer.serialize_i64(value)
        } else {
            serializer.serialize_str(&value.to_string())
        }
    }
}

impl Scenario {
    pub fn from_json_str(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_json_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Structural validation independent of running the scenario: every
    /// actor/proposal reference in `steps` resolves to something declared
    /// earlier, and roles are internally consistent.
    pub fn validate(&self) -> Result<(), String> {
        use std::collections::HashSet;

        if self.actors.is_empty() {
            return Err("scenario has no actors".to_string());
        }

        let actor_names: HashSet<&str> = self.actors.iter().map(|a| a.name.as_str()).collect();
        if actor_names.len() != self.actors.len() {
            return Err("duplicate actor names".to_string());
        }

        for actor in &self.actors {
            if let Some(ref d) = actor.delegate_to {
                if !actor_names.contains(d.as_str()) {
                    return Err(format!(
                        "actor '{}' delegates to unknown actor '{}'",
                        actor.name, d
                    ));
                }
            }
        }

        let mut known_proposals: HashSet<u64> = HashSet::new();
        for (i, step) in self.steps.iter().enumerate() {
            let actor_ref = match step {
                SimStep::Propose { actor, .. }
                | SimStep::Vote { actor, .. }
                | SimStep::Queue { actor, .. }
                | SimStep::Execute { actor, .. }
                | SimStep::Cancel { actor, .. }
                | SimStep::Delegate { actor, .. }
                | SimStep::MintTokens { actor, .. }
                | SimStep::BurnTokens { actor, .. }
                | SimStep::UpdateConfig { actor, .. }
                | SimStep::PauseContract { actor }
                | SimStep::UnpauseContract { actor }
                | SimStep::ScheduleBatch { actor, .. }
                | SimStep::ExecuteBatch { actor, .. }
                | SimStep::LockProposalBond { actor, .. }
                | SimStep::RefundProposalBond { actor, .. }
                | SimStep::ProposeBondSlash { actor, .. }
                | SimStep::MintPoolTokens { actor, .. }
                | SimStep::AddLiquidity { actor, .. }
                | SimStep::RemoveLiquidity { actor, .. }
                | SimStep::Swap { actor, .. }
                | SimStep::ExpectLpShares { actor, .. }
                | SimStep::ExpectPoolTokenBalance { actor, .. }
                | SimStep::FundRewardsPool { actor, .. }
                | SimStep::ClaimReward { actor, .. }
                | SimStep::ExpectRewardClaimed { actor, .. }
                | SimStep::ExpectRewardBalance { actor, .. } => Some(actor.as_str()),
                _ => None,
            };
            if let Some(name) = actor_ref {
                if !actor_names.contains(name) {
                    return Err(format!("step {} references unknown actor '{}'", i, name));
                }
            }
            if let SimStep::Delegate { delegatee, .. } = step {
                if !actor_names.contains(delegatee.as_str()) {
                    return Err(format!(
                        "step {} delegates to unknown actor '{}'",
                        i, delegatee
                    ));
                }
            }
            if let SimStep::PublishRewardsRoot { allocations, .. } = step {
                for allocation in allocations {
                    if !actor_names.contains(allocation.actor.as_str()) {
                        return Err(format!(
                            "step {} allocates rewards to unknown actor '{}'",
                            i, allocation.actor
                        ));
                    }
                }
            }
            if let SimStep::ProposeBondSlash { recipient, .. } = step {
                if !actor_names.contains(recipient.as_str()) {
                    return Err(format!(
                        "step {} references unknown bond slash recipient '{}'",
                        i, recipient
                    ));
                }
            }
            match step {
                SimStep::Propose { .. }
                | SimStep::FinalizeDraft { .. }
                | SimStep::ProposeBondSlash { .. } => {
                    // Proposal ids are assigned sequentially starting at 1 by
                    // the contract; the scenario doesn't declare them, so we
                    // just track how many have been created so far.
                    known_proposals.insert(known_proposals.len() as u64 + 1);
                }
                SimStep::Vote { proposal_id, .. }
                | SimStep::Queue { proposal_id, .. }
                | SimStep::Execute { proposal_id, .. }
                | SimStep::Cancel { proposal_id, .. }
                | SimStep::ExpectState { proposal_id, .. }
                | SimStep::AssertParticipation { proposal_id, .. }
                | SimStep::AssertQuorumReached { proposal_id }
                | SimStep::RefundProposalBond { proposal_id, .. } => {
                    if !known_proposals.contains(proposal_id) {
                        return Err(format!(
                            "step {} references proposal #{} before it is created",
                            i, proposal_id
                        ));
                    }
                }
                SimStep::ExpectError { step_index, .. } => {
                    if *step_index >= self.steps.len() {
                        return Err(format!(
                            "step {} references out-of-range step_index {}",
                            i, step_index
                        ));
                    }
                }
                SimStep::ExecuteBatch {
                    schedule_step_index,
                    ..
                } => {
                    if *schedule_step_index >= self.steps.len() {
                        return Err(format!(
                            "step {} references out-of-range schedule_step_index {}",
                            i, schedule_step_index
                        ));
                    }
                }
                SimStep::ValidateDag {
                    schedule_step_indices,
                } => {
                    for idx in schedule_step_indices {
                        if *idx >= self.steps.len() {
                            return Err(format!(
                                "step {} references out-of-range schedule_step_index {}",
                                i, idx
                            ));
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }
}
