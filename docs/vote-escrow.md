# Vote Escrow (ve model)

## Overview

The **Vote Escrow** feature (`contracts/vote-escrow`) implements a **time-locked voting power boost** where token holders lock their tokens for a specified duration to gain amplified voting power. The longer a token is locked, the higher the multiplier applied to its voting power. This mechanism:

- **Incentivizes long-term commitment** by rewarding participants who lock tokens for extended periods
- **Aligns incentives** between governance participants and the protocol's long-term success
- **Prevents vote concentration** by making large short-term voting attacks more expensive
- **Implements linear decay** so voting power naturally decreases as the lock expiration approaches

## How It Works

### Time-Lock Model

```
Holder locks 1000 tokens for 1 year
    ↓
Gains initial voting power = 1000 × (1 + boost multiplier)
    ↓
As time passes, voting power decays linearly
    ↓
At expiration, voting power returns to base amount
    ↓
Holder can withdraw locked tokens
```

### Key Concepts

**Lock**: A record binding a token holder's address to:
- `amount`: The number of tokens locked
- `start_ledger`: When the lock was created
- `end_ledger`: When the lock expires and can be withdrawn
- `initial_voting_power`: The voting power granted at lock time

**Boost Multiplier**: A basis-point multiplier applied based on lock duration:
- Minimum lock duration: Configurable (e.g., 7 days)
- Maximum lock duration: Configurable (e.g., 1 year)
- Max multiplier: Governance-settable (e.g., 4x at max duration)

The boost is calculated as:

```
boost_factor = (lock_duration / max_lock_duration) × max_multiplier
initial_voting_power = amount × (1 + boost_factor)
```

**Linear Decay**: Voting power linearly decays from `initial_voting_power` to `amount` as the lock approaches expiration:

```
decayed_power = amount + (boost_amount × remaining_ledgers / total_duration)
```

At lock expiration, the holder regains exactly `amount` voting power (the original deposit with no boost).

**Lock History**: Every modification to a lock (increase amount, extend duration) creates a historical record. This allows `get_past_votes()` to compute voting power at any past ledger, essential for snapshot voting.

## Contract Configuration

The vote-escrow contract requires initialization with:

| Parameter               | Description                                           | Example     |
| ----------------------- | ----------------------------------------------------- | ----------- |
| `min_lock_duration`     | Minimum lock duration in ledgers                      | 604,800 (~7 days at 5s/block) |
| `max_lock_duration`     | Maximum lock duration in ledgers                      | 31,536,000 (~1 year) |
| `max_multiplier_bps`    | Maximum boost as basis points (out of 10,000)        | 40,000 (4x)  |

## Lifecycle

A token holder's interaction with vote-escrow follows these phases:

### 1. Lock Creation

Create a new time lock:

```rust
create_lock(owner: Address, amount: i128, duration_ledgers: u32) -> Lock
```

- Owner must have sufficient token balance
- Duration must be between `min_lock_duration` and `max_lock_duration`
- Cannot create a second lock while an existing lock is active (non-withdrawn)
- Emits `LockCreated` event with voting power assigned

### 2. Increase Lock Amount

Add more tokens to an existing lock (preserves expiration):

```rust
increase_lock_amount(owner: Address, additional_amount: i128) -> Lock
```

- Extends lock history and recalculates voting power
- Useful when a holder wants more voting power without changing the duration

### 3. Extend Lock Duration

Delay the expiration to increase the boost:

```rust
extend_lock(owner: Address, new_end_ledger: u32) -> Lock
```

- `new_end_ledger` must be later than the current expiration
- Recalculates voting power based on the new duration
- Fails if called after lock expiration

### 4. Withdraw

Unlock tokens and recover them:

```rust
withdraw(owner: Address) -> i128
```

- Only callable after the lock's `end_ledger` has passed
- Returns the locked amount and emits `Withdrawn` event
- After withdrawal, voting power becomes zero

## Voting Power Queries

### Current Voting Power

Get the current decayed voting power of an account:

```rust
get_votes(account: Address) -> i128
```

Returns zero for accounts with no active lock or a withdrawn lock.

### Historical Voting Power (for Snapshot Voting)

Query voting power at a specific past ledger (used for proposal voting):

```rust
get_past_votes(account: Address, ledger: u32) -> i128
```

- Consults lock history to find which lock was active at that ledger
- Computes voting power at that historical moment
- Returns zero if account had no active lock at that time
- Essential for snapshot voting where votes must be checked at proposal creation time, not execution time

### Total Supply (Global)

Get the summed voting power across all accounts at a historical ledger:

```rust
get_past_total_supply(ledger: u32) -> i128
```

- Used to compute quorum and pass-rate thresholds
- Tracks global checkpoints to optimize lookups

## Integration with Governor

Vote-escrow integrates with the main governor contract as an alternative voting-weight source. The governor can delegate vote-counting to vote-escrow instead of (or alongside) token-votes:

```typescript
// Query voting power from vote-escrow for a proposal snapshot
const votingPower = await voteEscrowClient.getPastVotes(voterAddress, proposalCreationLedger);

// Use this voting power when vote-counting instead of token balance
```

### Typical Integration Pattern

1. When a proposal is created, snapshot the current ledger number
2. When vote-counting (at proposal close or execution), use `get_past_votes(voter, snapshot_ledger)` instead of token balance
3. Aggregate `get_past_total_supply(snapshot_ledger)` for quorum thresholds

This ensures voting power is deterministic and cannot change after a vote is cast.

## Configuration Updates

The admin can update escrow parameters after initialization:

```rust
update_escrow_config(
    admin: Address,
    min_lock_duration: Option<u32>,
    max_lock_duration: Option<u32>,
    max_multiplier_bps: Option<u32>,
)
```

- Each parameter is optional; omit to keep current value
- Changes apply only to new locks, not existing ones
- Existing locks continue to decay with their original parameters

## Error Handling

The contract emits typed errors for common failure cases:

| Error                    | Cause                                              |
| ----------------------- | ------------------------------------------------- |
| `InvalidDuration`       | Lock duration outside [min, max] bounds           |
| `InvalidAmount`         | Lock amount is zero or negative                   |
| `LockAlreadyActive`     | Holder already has a non-withdrawn lock           |
| `LockNotFound`          | Holder has no lock (or it was withdrawn)          |
| `LockAlreadyWithdrawn`  | Attempting operation on a withdrawn lock          |
| `ArithmeticOverflow`    | Integer overflow in lock calculations             |

## Events

All state changes emit structured events for off-chain indexing:

- `LockCreated(owner, amount, end_ledger, initial_voting_power)`
- `LockIncreased(owner, new_amount, new_voting_power)`
- `LockExtended(owner, new_end_ledger, new_voting_power)`
- `Withdrawn(owner, amount)`

## Usage Example (SDK)

### Creating a Lock

```typescript
import { VoteEscrowClient } from '@nebgov/sdk';

const client = new VoteEscrowClient({
  rpc: stellarRpc,
  contractId: voteEscrowAddress,
});

// Lock 1000 tokens for 365 days
const oneYear = Math.floor((365 * 24 * 60 * 60) / 5); // Convert to ledgers (~5s each)

const lock = await client.createLock({
  owner: userAddress,
  amount: '1000000000', // 1000 tokens in stroops
  durationLedgers: oneYear,
});

console.log(`Initial voting power: ${lock.initial_voting_power}`);
```

### Querying Voting Power

```typescript
// Get current voting power (decayed)
const currentVotes = await client.getVotes(userAddress);

// Get historical voting power at proposal snapshot
const snapshotVotes = await client.getPastVotes(userAddress, proposalLedger);

// Get total voting power across all holders at a snapshot
const totalSupply = await client.getPastTotalSupply(proposalLedger);
```

### Increasing Lock or Extending Duration

```typescript
// Add more tokens to the lock
const updated = await client.increaseLockAmount({
  owner: userAddress,
  additionalAmount: '500000000', // 500 more tokens
});

// Extend the lock by another year
const extended = await client.extendLock({
  owner: userAddress,
  newEndLedger: currentLock.endLedger + oneYear,
});
```

### Withdrawing After Lock Expires

```typescript
// Wait for lock.endLedger to pass, then withdraw
const withdrawn = await client.withdraw(userAddress);
console.log(`Recovered ${withdrawn} tokens`);
```

## Design Rationale

**Why Linear Decay?** Linear decay is simpler to audit than polynomial models and aligns voting power directly with time remaining. A voter's conviction weakens proportionally as their lock expiration approaches—fitting for governance.

**Why Lock History?** For snapshot voting, the governor must know voting power at the *proposal creation* time, not at vote-counting time. Lock history enables retrospective queries without re-running all contract state.

**Why Configurable Duration Bounds?** Different governance systems have different security needs. Short-lived (7-day) minimum locks suit high-frequency decisions; longer (1-year) locks suit protocol-wide decisions that need deep commitment signals.

**Why Time-Lock Rather Than Direct Multiplier?** Time-locking prevents whale attacks where a large holder briefly locks tokens just before a vote, then unstakes. It aligns voting power with long-term skin in the game.
