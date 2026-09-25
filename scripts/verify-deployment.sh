#!/usr/bin/env bash
# ============================================================
# scripts/verify-deployment.sh
#
# Post-deploy validation: queries each contract's on-chain
# settings and compares them against expected values from
# the env file. Exits non-zero if any check fails (CI-safe).
#
# Usage:
#   ./scripts/verify-deployment.sh              # uses .env.testnet
#   ENV_FILE=.env.custom ./scripts/verify-deployment.sh
# ============================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="${ENV_FILE:-$ROOT_DIR/.env.testnet}"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

FAILURES=0

pass()  { printf "${GREEN}[✓]${NC} %s\n" "$*"; }
fail()  { printf "${RED}[✗]${NC} %s\n" "$*" >&2; FAILURES=$((FAILURES + 1)); }
info()  { printf "${CYAN}[info]${NC}  %s\n" "$*"; }

[[ -f "$ENV_FILE" ]] || { printf "${RED}[error]${NC} Env file not found: %s\n" "$ENV_FILE" >&2; exit 1; }

set -a
# shellcheck source=/dev/null
source "$ENV_FILE"
set +a

command -v stellar >/dev/null 2>&1 || { printf "${RED}[error]${NC} stellar-cli not found\n" >&2; exit 1; }

NETWORK="${STELLAR_NETWORK:-testnet}"
IDENTITY="${STELLAR_IDENTITY:-deployer}"

# Call a contract getter, optionally with extra `-- <fn>` args after the
# function name (e.g. `query "$ID" get_required_threshold --requested_amount 0`).
# Returns "ERROR" if the invocation fails.
query() {
  local id="$1" fn="$2"
  shift 2
  stellar contract invoke \
    --id "$id" \
    --source "$IDENTITY" \
    --network "$NETWORK" \
    -- "$fn" "$@" 2>/dev/null || echo "ERROR"
}

# Assert got == expected and print a labelled result.
check() {
  local label="$1" got="$2" expected="$3"
  if [[ "$got" == "$expected" ]]; then
    pass "$label: $got"
  else
    fail "$label: got '$got', expected '$expected'"
  fi
}

# Assert that a getter response is neither "ERROR" (deploy/invoke failure)
# nor empty, proving the contract is both deployed and initialized (the
# getters used here panic with NotInitialized until initialize() succeeds).
check_initialized() {
  local label="$1" got="$2"
  if [[ -n "$got" && "$got" != "ERROR" ]]; then
    pass "$label: deployed and initialized"
  else
    fail "$label: contract not deployed or not initialized"
  fi
}

# Assert that `haystack` contains `needle` and print a labelled result.
check_contains() {
  local label="$1" haystack="$2" needle="$3"
  if [[ -n "$needle" && "$haystack" == *"$needle"* ]]; then
    pass "$label: contains $needle"
  else
    fail "$label: expected '$haystack' to contain '$needle'"
  fi
}

DEPLOYER_ADDR="${DEPLOYER_ADDR:-$(stellar keys address "${IDENTITY}" 2>/dev/null || echo '')}"

info "Verifying NebGov deployment against $ENV_FILE"
printf '\n'

# ---- TokenVotes --------------------------------------------------------
info "TokenVotes (${TOKEN_VOTES_ADDRESS:-<not set>})"
check "  token_votes.admin" \
  "$(query "${TOKEN_VOTES_ADDRESS:-}" admin)" \
  "\"${DEPLOYER_ADDR}\""

# ---- Timelock ----------------------------------------------------------
info "Timelock (${TIMELOCK_ADDRESS:-<not set>})"
check "  timelock.min_delay" \
  "$(query "${TIMELOCK_ADDRESS:-}" min_delay)" \
  "${TIMELOCK_MIN_DELAY:-3600}"
check "  timelock.execution_window" \
  "$(query "${TIMELOCK_ADDRESS:-}" execution_window)" \
  "${TIMELOCK_EXECUTION_WINDOW:-86400}"

# ---- Governor ----------------------------------------------------------
info "Governor (${GOVERNOR_ADDRESS:-<not set>})"
check "  governor.voting_delay" \
  "$(query "${GOVERNOR_ADDRESS:-}" voting_delay)" \
  "${VOTING_DELAY:-60}"
check "  governor.voting_period" \
  "$(query "${GOVERNOR_ADDRESS:-}" voting_period)" \
  "${VOTING_PERIOD:-17280}"
check "  governor.quorum_numerator" \
  "$(query "${GOVERNOR_ADDRESS:-}" quorum_numerator)" \
  "${QUORUM_NUMERATOR:-4}"
check "  governor.proposal_threshold" \
  "$(query "${GOVERNOR_ADDRESS:-}" proposal_threshold)" \
  "${PROPOSAL_THRESHOLD:-100000000}"

# ---- Treasury ----------------------------------------------------------
info "Treasury (${TREASURY_ADDRESS:-<not set>})"
check "  treasury.threshold" \
  "$(query "${TREASURY_ADDRESS:-}" threshold)" \
  "${TREASURY_THRESHOLD:-1}"

# ---- ConvictionVoting ----------------------------------------------------
# No config/settings getter exists on this contract; get_required_threshold
# is the only read-only entrypoint that panics with NotInitialized until
# initialize() succeeds, so it doubles as the deployed+initialized check.
info "ConvictionVoting (${CONVICTION_VOTING_ADDRESS:-<not set>})"
check_initialized "  conviction_voting.get_required_threshold" \
  "$(query "${CONVICTION_VOTING_ADDRESS:-}" get_required_threshold --requested_amount 0)"

# ---- TreasuryStrategies ---------------------------------------------------
info "TreasuryStrategies (${TREASURY_STRATEGIES_ADDRESS:-<not set>})"
TREASURY_STRATEGIES_TREASURY="$(query "${TREASURY_STRATEGIES_ADDRESS:-}" get_treasury)"
check_initialized "  treasury_strategies.get_treasury" "$TREASURY_STRATEGIES_TREASURY"
if [[ "$TREASURY_STRATEGIES_TREASURY" != "ERROR" ]]; then
  check_contains "  treasury_strategies.treasury" \
    "$TREASURY_STRATEGIES_TREASURY" \
    "${TREASURY_ADDRESS:-}"
fi

# ---- Liquidity ---------------------------------------------------------
info "Liquidity (${LIQUIDITY_ADDRESS:-<not set>})"
check "  liquidity.governor" \
  "$(query "${LIQUIDITY_ADDRESS:-}" governor)" \
  "\"${DEPLOYER_ADDR}\""

# ---- OptimisticGovernor -------------------------------------------------
info "OptimisticGovernor (${OPTIMISTIC_GOVERNOR_ADDRESS:-<not set>})"
OPTIMISTIC_GOVERNOR_CONFIG="$(query "${OPTIMISTIC_GOVERNOR_ADDRESS:-}" get_config)"
check_initialized "  optimistic_governor.get_config" "$OPTIMISTIC_GOVERNOR_CONFIG"
if [[ "$OPTIMISTIC_GOVERNOR_CONFIG" != "ERROR" ]]; then
  check_contains "  optimistic_governor.votes_token" \
    "$OPTIMISTIC_GOVERNOR_CONFIG" \
    "${TOKEN_VOTES_ADDRESS:-}"
fi

# ---- SignalAnchor -------------------------------------------------------
# No config/settings getter exists on this contract; admin() is the only
# read-only entrypoint that panics with NotInitialized until initialize()
# succeeds, so it doubles as the deployed+initialized check.
info "SignalAnchor (${SIGNAL_ANCHOR_ADDRESS:-<not set>})"
check_initialized "  signal_anchor.admin" \
  "$(query "${SIGNAL_ANCHOR_ADDRESS:-}" admin)"

# ---- VoteEscrow ---------------------------------------------------------
info "VoteEscrow (${VOTE_ESCROW_ADDRESS:-<not set>})"
check_initialized "  vote_escrow.admin" \
  "$(query "${VOTE_ESCROW_ADDRESS:-}" admin)"

# ---- VotingRewards -------------------------------------------------------
info "VotingRewards (${VOTING_REWARDS_ADDRESS:-<not set>})"
check_initialized "  voting_rewards.total_pool" \
  "$(query "${VOTING_REWARDS_ADDRESS:-}" total_pool)"

# ---- ProposalBonds -------------------------------------------------------
info "ProposalBonds (${PROPOSAL_BONDS_ADDRESS:-<not set>})"
PROPOSAL_BONDS_SETTINGS="$(query "${PROPOSAL_BONDS_ADDRESS:-}" get_settings)"
check_initialized "  proposal_bonds.get_settings" "$PROPOSAL_BONDS_SETTINGS"
if [[ "$PROPOSAL_BONDS_SETTINGS" != "ERROR" ]]; then
  check_contains "  proposal_bonds.governor" \
    "$PROPOSAL_BONDS_SETTINGS" \
    "${GOVERNOR_ADDRESS:-}"
fi

# ---- CoSponsorship -------------------------------------------------------
info "CoSponsorship (${CO_SPONSORSHIP_ADDRESS:-<not set>})"
check_initialized "  co_sponsorship.admin" \
  "$(query "${CO_SPONSORSHIP_ADDRESS:-}" admin)"

# ---- TokenVotesWrapper ---------------------------------------------------
info "TokenVotesWrapper (${WRAPPER_ADDRESS:-<not set>})"
check_initialized "  token_votes_wrapper.admin" \
  "$(query "${WRAPPER_ADDRESS:-}" admin)"

# ---- GovernorFactory -----------------------------------------------------
info "GovernorFactory (${FACTORY_ADDRESS:-<not set>})"
check_initialized "  governor_factory.admin" \
  "$(query "${FACTORY_ADDRESS:-}" admin)"

printf '\n'
if [[ "$FAILURES" -gt 0 ]]; then
  printf "${RED}%d check(s) failed.${NC}\n" "$FAILURES" >&2
  exit 1
else
  printf "${GREEN}All checks passed.${NC}\n"
fi
