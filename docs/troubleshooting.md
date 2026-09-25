# Troubleshooting Guide

Common errors when interacting with NebGov on Stellar, along with their causes and fixes.

---

## Table of Contents

- [RPC Errors](#rpc-errors)
- [Wallet Errors](#wallet-errors)
- [Transaction Errors](#transaction-errors)
- [Contract Errors](#contract-errors)
- [Voting and Proposals](#voting-and-proposals)
- [Liquidity Contract Errors](#liquidity-contract-errors)
- [FAQ](#faq)

---

## RPC Errors

### `Error: Simulation failed: HostError`

**Cause:** The contract invocation failed during simulation. Common reasons: wrong function name, incorrect argument types, or the contract is not deployed at the configured address.

**Fix:**
1. Verify the function name matches the contract ABI exactly (case-sensitive).
2. Check that all argument types are correct (e.g., `u32` vs `i128`, `Address` vs `String`).
3. Confirm `NEXT_PUBLIC_GOVERNOR_ADDRESS` in your `.env` points to a deployed contract on the correct network.
4. Run the simulation manually with `stellar contract invoke --simulate-only` to inspect the full error.

---

### `RPC connection refused` / `Failed to fetch`

**Cause:** The app cannot reach the Stellar RPC endpoint. The `NEXT_PUBLIC_RPC_URL` environment variable is missing or points to the wrong host.

**Fix:**
1. Open your `.env` (or `.env.local`) and verify `NEXT_PUBLIC_RPC_URL`.
   - Testnet: `https://soroban-testnet.stellar.org`
   - Futurenet: `https://rpc-futurenet.stellar.org`
   - Mainnet: `https://soroban-mainnet.stellar.org`
2. Restart the dev server after changing `.env`.
3. Check that your firewall or VPN is not blocking outbound HTTPS to port 443.

---

### `Error: Network mismatch`

**Cause:** The wallet is connected to a different Stellar network than the one the app targets.

**Fix:**
1. In your wallet (Freighter, Lobstr, etc.), switch to the network specified in `NEXT_PUBLIC_NETWORK_PASSPHRASE`.
2. The passphrase values are:
   - Testnet: `Test SDF Network ; September 2015`
   - Mainnet: `Public Global Stellar Network ; September 2015`

---

## Wallet Errors

### `Wallet not connected` / `No wallet found`

**Cause:** `WalletKitProvider` is not wrapping the component tree, or the provider was initialized after the component mounted.

**Fix:**
1. Ensure `<WalletKitProvider>` wraps your root layout in `app/layout.tsx` (or equivalent).
2. Do not conditionally render the provider — it must wrap the tree unconditionally.
3. If using `useWalletKit()`, confirm you are inside a component that is a descendant of the provider.

---

### `User declined the transaction`

**Cause:** The user cancelled the signing prompt in their wallet extension.

**Fix:** This is expected behavior. Show a non-blocking notification and allow the user to retry. Do not treat this as an application error.

---

### `PublicKey is not valid`

**Cause:** The wallet returned an empty or malformed public key. This can happen when the wallet is locked or has not yet granted permission to the dApp.

**Fix:**
1. Unlock your wallet and approve the site connection request.
2. Disconnect and reconnect via the Connect Wallet button.
3. If the issue persists, clear wallet permissions for the site and reconnect.

---

## Transaction Errors

### `Transaction rejected: insufficient fee`

**Cause:** The submitted fee is too low for current network conditions. The Stellar network uses a fee market during congestion.

**Fix:**
1. Use fee bumping: submit the transaction with a higher `base_fee` (e.g., 1 000 000 stroops during high congestion).
2. The SDK's `submitTransaction` helper retries with fee bumping automatically. Ensure you are on SDK v0.3.0+.
3. Monitor current base fees with `stellar ledger fetch --network testnet | jq .base_fee`.

---

### `Transaction expired`

**Cause:** The transaction's `timeBounds` window elapsed before it was included in a ledger. This often happens when the user delays signing.

**Fix:**
1. Increase `timeboundsSeconds` in the transaction builder (default is 30 s; try 120 s for complex multi-step flows).
2. Prompt the user to sign promptly after building the transaction.

---

### `Error: Ledger entry not found`

**Cause:** A persistent storage entry (e.g., a proposal or LP position) does not exist at the queried key. The contract may be on a different network than expected.

**Fix:**
1. Confirm `NEXT_PUBLIC_GOVERNOR_ADDRESS` and `NEXT_PUBLIC_TIMELOCK_ADDRESS` are set for the correct network.
2. Verify the data was written in a transaction that was successfully included in the ledger (not just simulated).

---

## Contract Errors

### VoteEscrow Errors

#### `Not initialized` (`VoteEscrowError::NotInitialized`)

**Cause:** The VoteEscrow contract has not been initialized. This occurs when calling contract methods before `initialize()` is invoked.

**Fix:**
1. Ensure the deployment script has completed and called `initialize()` on the VoteEscrow contract.
2. Verify `VOTE_ESCROW_ADDRESS` in your `.env` points to a deployed and initialized contract.
3. Run `scripts/verify-deployment.sh` to check VoteEscrow initialization status.

---

#### `Invalid duration` (`VoteEscrowError::InvalidDuration`)

**Cause:** The lock duration is outside the allowed range. Lock duration must be between `min_lock_duration` and `max_lock_duration`.

**Fix:**
1. Query `min_lock_duration` and `max_lock_duration` from the contract.
2. Ensure `duration_ledgers` for `create_lock()` is within this range.
3. Typical ranges: minimum 1 ledger, maximum 52 weeks in ledgers (~2,700,000 on Stellar).

---

#### `Invalid amount` (`VoteEscrowError::InvalidAmount`)

**Cause:** The lock amount is zero or negative. VoteEscrow requires a positive token amount to lock.

**Fix:**
1. Ensure the amount being locked is greater than zero.
2. Account for token decimals when calculating amounts (e.g., 7 decimals = multiply by 10^7).

---

#### `Lock not found` (`VoteEscrowError::LockNotFound`)

**Cause:** No lock exists for the given address. Either no lock was created, or the address is incorrect.

**Fix:**
1. Call `create_lock()` to create a new lock before attempting to query or modify it.
2. Verify the wallet address matches the owner of the lock.
3. Use the same account that created the lock when querying or modifying it.

---

#### `Lock not matured` (`VoteEscrowError::LockNotMatured`)

**Cause:** The lock's `end_ledger` has not yet passed. Locks must mature before withdrawal.

**Fix:**
1. Query `get_lock()` to see the lock's `end_ledger`.
2. Wait until the current ledger sequence exceeds the `end_ledger` to withdraw.
3. Monitor the network: `stellar ledger fetch --network testnet | jq .sequence`.

---

#### `Lock already withdrawn` (`VoteEscrowError::LockAlreadyWithdrawn`)

**Cause:** The lock has already been withdrawn. Each lock can only be withdrawn once.

**Fix:**
1. Create a new lock if you wish to lock tokens again.
2. Check the lock's `withdrawn` field before attempting to withdraw.

---

#### `Invalid end ledger` (`VoteEscrowError::InvalidEndLedger`)

**Cause:** The computed `end_ledger` (start + duration) is invalid or has already passed.

**Fix:**
1. Ensure the lock duration is valid and not zero.
2. Check that the current ledger is not already past the intended `end_ledger`.

---

#### `Unauthorized` (`VoteEscrowError::Unauthorized`)

**Cause:** The caller is not authorized to perform this action. Some operations require admin privileges.

**Fix:**
1. Ensure the wallet address is authorized (admin or lock owner).
2. For admin-only operations, use the admin account configured during deployment.

---

#### `Arithmetic overflow` (`VoteEscrowError::ArithmeticOverflow`)

**Cause:** An internal calculation overflowed. This typically occurs with very large amounts or edge cases in voting power calculations.

**Fix:**
1. Reduce the lock amount or duration if possible.
2. Report this as a bug if using normal amounts; it indicates a contract issue.

---

### VotingRewards Errors

#### `Already initialized` (`VotingRewardsError::AlreadyInitialized`)

**Cause:** The VotingRewards contract has already been initialized. `initialize()` can only be called once.

**Fix:**
1. Verify the contract is already initialized by checking `total_pool()`.
2. If deploying fresh, use a new contract address.

---

#### `Not initialized` (`VotingRewardsError::NotInitialized`)

**Cause:** The VotingRewards contract has not been initialized yet.

**Fix:**
1. Ensure `initialize()` has been called after contract deployment.
2. Run `scripts/verify-deployment.sh` to confirm VotingRewards initialization.

---

#### `Not authorized` (`VotingRewardsError::NotAuthorized`)

**Cause:** The caller lacks the required permissions. This typically applies to admin-only operations.

**Fix:**
1. Use the admin account configured during deployment.
2. For claim operations, ensure the caller's address matches the one in the Merkle proof.

---

#### `Invalid epoch duration` (`VotingRewardsError::InvalidEpochDuration`)

**Cause:** The epoch duration is zero or invalid. Epochs must have positive duration.

**Fix:**
1. Pass a positive epoch duration (in ledgers) to `initialize()`.
2. Typical epoch durations: 1 week to 1 month of ledgers (~252,000 to 2,628,000 ledgers on Stellar).

---

#### `Epoch not found` (`VotingRewardsError::EpochNotFound`)

**Cause:** The specified epoch does not exist. The epoch number or state is incorrect.

**Fix:**
1. Query recent epochs with `get_epochs()`.
2. Verify the epoch number is within the range of finalized or active epochs.

---

#### `Epoch not ended` (`VotingRewardsError::EpochNotEnded`)

**Cause:** The epoch is still active. Operations requiring a closed epoch cannot proceed.

**Fix:**
1. Wait until the current ledger surpasses the epoch's end ledger.
2. Call `finalize_epoch()` only after the epoch has ended.

---

#### `Epoch already finalized` (`VotingRewardsError::EpochAlreadyFinalized`)

**Cause:** The epoch has already been finalized. Finalization is idempotent but unnecessary to repeat.

**Fix:**
1. Verify the epoch state before attempting to finalize again.
2. Proceed to the next operation (e.g., computing or submitting Merkle proofs).

---

#### `Epoch not finalized` (`VotingRewardsError::EpochNotFinalized`)

**Cause:** The epoch must be finalized before claims can be submitted. This ensures reward calculations are complete.

**Fix:**
1. Call `finalize_epoch()` first to compute the final reward distribution.
2. Wait for finalization to complete before submitting claims.

---

#### `Insufficient pool` (`VotingRewardsError::InsufficientPool`)

**Cause:** The reward pool has insufficient balance to pay the claim. Rewards may have already been partially claimed.

**Fix:**
1. Verify the total reward pool was funded correctly during epoch setup.
2. Check how much has already been claimed for this epoch.
3. If pool is exhausted, it indicates an issue with reward calculations or multiple claims.

---

#### `Invalid amount` (`VotingRewardsError::InvalidAmount`)

**Cause:** The claim amount is zero, negative, or exceeds the claimable amount for this address.

**Fix:**
1. Query the claimable amount for your address using `get_claimable()`.
2. Ensure the proof corresponds to a valid, non-zero reward for this epoch.

---

#### `Already claimed` (`VotingRewardsError::AlreadyClaimed`)

**Cause:** The address has already claimed rewards for this epoch. Each address can claim only once per epoch.

**Fix:**
1. Use a different address if you wish to claim again (e.g., for a different vote weight).
2. Check the claim status with `get_claim()` before submitting a claim.

---

#### `Invalid proof` (`VotingRewardsError::InvalidProof`)

**Cause:** The Merkle proof is invalid or does not match the epoch's Merkle root. This occurs when the proof was corrupted or computed incorrectly.

**Fix:**
1. Re-fetch the Merkle proof from the backend API (`GET /voting-rewards/proof?epoch=<EPOCH>&address=<ADDRESS>`).
2. Verify the proof is for the correct epoch and address.
3. Ensure the backend's Merkle root matches the contract's stored root for this epoch.

---

#### `Epoch overclaimed` (`VotingRewardsError::EpochOverclaimed`)

**Cause:** The total claimed amount exceeds the finalized rewards for this epoch. This indicates a calculation error or corrupted Merkle root.

**Fix:**
1. Report this as a critical bug; it suggests either backend miscalculation or a malicious proof.
2. Stop processing claims and investigate the Merkle tree computation.

---

### `Proposal not found` (`GovernorError::ProposalNotFound`)

**Cause:** The proposal ID does not exist in the governor contract. Either the wrong contract address is configured, or the proposal was never created on this network.

**Fix:**
1. Verify `NEXT_PUBLIC_GOVERNOR_ADDRESS` matches the deployed governor on the active network.
2. Confirm the proposal was created by checking the transaction that called `propose()`.

---

### `Not authorized` / `require_auth` failed

**Cause:** The caller's signature was not present in the transaction authorization envelope, or the wrong address was passed as the `caller` argument.

**Fix:**
1. Ensure the wallet's public key matches the address passed to privileged functions (e.g., `governor`, `admin`, `provider`).
2. When constructing the transaction, call `addSignatureBase64` or equivalent for each required signer before submitting.

---

### `Pool not found` (`LiquidityError::PoolNotFound`)

**Cause:** No liquidity pool exists for the given outcome pair, or the pair was registered under a different token ordering.

**Fix:**
1. Confirm `create_pool` and `initialize_pool` were called by the governor for this outcome pair.
2. Outcome pair ordering is canonical (smaller id first) — querying `(1, 2)` and `(2, 1)` resolve to the same pool.
3. Check that the `NEXT_PUBLIC_LIQUIDITY_ADDRESS` env var points to the correct contract.

---

## Voting and Proposals

### `Vote period not started`

**Cause:** `cast_vote` was called before the proposal's `start_ledger`. The proposal is in `Pending` state.

**Fix:**
1. Call `governor.state(proposal_id)` to get the current state.
2. Read `proposal.start_ledger` and wait until the current ledger sequence surpasses it.
3. The UI should disable the Vote button until the proposal is `Active`.

---

### `Already voted` (`GovernorError::AlreadyVoted`)

**Cause:** The wallet address has already cast a vote for this proposal. The contract rejects duplicate votes.

**Fix:** Each address may vote only once per proposal. Show the user their existing vote choice instead of the voting form.

---

### `Insufficient voting power`

**Cause:** The user has no delegated voting power at the proposal's snapshot ledger. Tokens must be delegated (self-delegated or to another account) before the proposal was created.

**Fix:**
1. Call `votes.delegate(account, account)` to self-delegate before the next proposal is created.
2. Wrapping tokens via `token_votes.wrap(amount)` and then delegating grants voting power.
3. Voting power is snapshotted at `proposal.start_ledger - 1`; delegation after that point does not count for this proposal.

---

### `Proposal not in voting period` (`GovernorError::ProposalNotActive`)

**Cause:** `cast_vote` was called when the proposal is in `Succeeded`, `Defeated`, `Queued`, `Executed`, or `Expired` state.

**Fix:** Check `governor.state(proposal_id)` before casting a vote. Only `Active` proposals accept new votes.

---

## Liquidity Contract Errors

### `Imbalanced deposit: amount_b below required ratio`

**Cause:** A subsequent deposit to a pool provided a `amount_b` that is less than the proportional amount required by the current reserve ratio. This prevents price manipulation.

**Fix:**
1. Fetch the current pool reserves with `liquidity.get_pool(outcome_a, outcome_b)`.
2. Compute `required_b = amount_a * reserve_b / reserve_a` before calling `add_liquidity`.
3. Pass `required_b` (or a value greater than it) as `amount_b`.

---

### `Below minimum liquidity`

**Cause:** The deposit amount is below `MIN_LIQUIDITY` (1 000 units). Tiny deposits are rejected to prevent dust griefing.

**Fix:** Ensure both `amount_a` and the computed `required_b` are at least 1 000 units.

---

### `Slippage exceeded`

**Cause:** The swap output `amount_out` after fees is less than the caller's `min_amount_out`. The pool moved between simulation and execution.

**Fix:**
1. Re-simulate the swap immediately before submitting to get the latest output.
2. Add a slippage tolerance (e.g., 0.5%) to `min_amount_out`: `min_amount_out = simulated_out * 995 / 1000`.

---

## FAQ

**Q: How do I find the correct contract addresses for my network?**

A: Contract addresses are published in [scripts/](../scripts/) after each deployment. You can also query the factory: `liquidity.governor()` returns the governor address linked to the liquidity contract.

**Q: My transaction simulates successfully but fails on-chain. Why?**

A: State can change between simulation and submission (another transaction was included first). Retry the simulation immediately before resubmitting, or add a short polling loop.

**Q: How do I run contract calls without the frontend?**

A: Use the Stellar CLI:
```bash
stellar contract invoke \
  --network testnet \
  --id <CONTRACT_ADDRESS> \
  --source <SECRET_KEY> \
  -- <function_name> --arg1 value1
```

**Q: Can I test on Futurenet before Testnet?**

A: Yes. Set `NEXT_PUBLIC_RPC_URL` to `https://rpc-futurenet.stellar.org` and `NEXT_PUBLIC_NETWORK_PASSPHRASE` to the Futurenet passphrase. Deploy contracts to Futurenet first using `stellar contract deploy --network futurenet`.

**Q: How do I reset local state during development?**

A: Re-initialize the contracts with fresh deployments:
```bash
stellar contract deploy --wasm target/wasm32v1-none/release/sorogov_governor.wasm --network testnet --source <KEY>
```
Update the address in `.env.local` and restart the dev server.

**Q: Why does `get_lp_position` return 0 even after adding liquidity?**

A: The provider address queried must exactly match the one used in `add_liquidity`. Also verify that `outcome_a` and `outcome_b` are in the correct order (use canonical order: smaller id first, e.g., `(0, 1)` not `(1, 0)`).
