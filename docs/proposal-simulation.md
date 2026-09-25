# Proposal Simulation

Proposal simulation enables you to dry-run governance actions against the current blockchain state before submitting them as proposals. This allows discovery of bugs, permission issues, and state conflicts before spending governance capital.

---

## Overview

The Proposal Simulation system lets you:

- **Simulate draft proposals** before submission to identify reverts and errors.
- **Re-simulate queued proposals** to check if state has changed since voting opened.
- **Inspect treasury impacts** to understand how treasury-modifying actions will affect available balances.
- **View human-readable summaries** of complex contract calls (XDR decoding).
- **Track simulation history** to compare simulations over time as network state evolves.

Simulation results are computed by the backend using the Stellar RPC's simulation endpoint, then stored for auditing and historical comparison.

---

## How Simulation Works

### Request Flow

1. **User prepares draft proposal** — specifies `targets` (contract addresses), `fnNames` (function names), and `calldatas` (XDR-encoded arguments).
2. **Frontend sends preview request** to `POST /proposal-simulation/preview` with the draft actions.
3. **Backend invokes each action** against current ledger state via Stellar RPC `prepareTransaction` + `simulateTransaction`.
4. **Backend decodes results** using known contract ABIs and returns human-readable summaries.
5. **Frontend displays results** — whether each action would succeed and any revert reasons.

### Simulation Lifecycle

```
[Draft]
  ↓
[User clicks "Preview"]
  ↓
[POST /proposal-simulation/preview]
  ↓
[Backend simulates each action]
  ↓
[Results returned: success/failure, decoded output, revert reason]
  ↓
[User submits or revises draft]
  ↓
[Proposal created on-chain]
  ↓
[Later: User/Admin clicks "Simulate" on queued proposal]
  ↓
[GET /proposal-simulation/:proposalId]
  ↓
[Backend re-simulates with current state]
  ↓
[Results may differ from vote-open time]
```

---

## Using Proposal Simulation

### SDK: Preview a Draft Proposal

```typescript
import { ProposalSimulationClient } from "@nebgov/sdk";

const client = new ProposalSimulationClient({
  governorAddress: "GOVERNOR_ADDR",
  timelockAddress: "TIMELOCK_ADDR",
  votesAddress: "VOTES_ADDR",
  network: "testnet",
  backendUrl: "http://localhost:3001", // Required: backend must provide /proposal-simulation endpoint
});

// Prepare action: call treasury.spend(token, recipient, amount)
const targets = ["TREASURY_ADDR"];
const fnNames = ["spend"];
const calldatas = [encodedXdrBytes]; // See @stellar/stellar-sdk nativeToScVal()

// Preview before submission
const results = await client.previewDraft(targets, fnNames, calldatas);

results.forEach((result) => {
  console.log(`${result.fnName}: ${result.success ? "✓" : "✗"}`);
  if (!result.success) {
    console.log(`  Reason: ${result.revertReason}`);
  }
  if (result.decodedSummary) {
    console.log(`  Summary: ${result.decodedSummary}`);
  }
});
```

### SDK: Simulate an Existing Proposal

```typescript
// After proposal is created on-chain (state may have changed)
const proposalId = 42;
const results = await client.simulateProposal(proposalId);

// Same result structure as previewDraft()
```

### Frontend: Create Proposal with Simulation

The NebGov web app integrates simulation into the proposal creation flow:

1. Go to **Create Proposal** page.
2. Add actions (targets, functions, calldata).
3. Click **"Preview Simulation"** — the UI calls `previewDraft()`.
4. Review results:
   - ✓ Green checkmark = action succeeds, no revert expected.
   - ✗ Red X = action would revert with reason shown.
   - 📊 Treasury impact (if applicable) = token balance changes.
5. If all pass, click **"Submit Proposal"** to create on-chain.
6. Before voting ends or to check for state changes, click **"Re-simulate"** on the queued proposal — the UI calls `simulateProposal()`.

---

## Interpreting Results

### SimulationResult Structure

```typescript
interface SimulationResult {
  target: string;            // Contract address called
  fnName: string;            // Function name
  success: boolean;          // Did the invocation succeed?
  decodedSummary?: string;   // Human-readable action description
  returnValue?: any;         // Decoded return value (if success)
  revertReason?: string;     // Error message (if !success)
  treasuryImpact?: {
    token: string;           // Token address affected
    capRemainingBefore: bigint;  // Treasury balance before
    capRemainingAfter: bigint;   // Treasury balance after
  };
}
```

### Success (✓)

All listed actions would execute without reverting. You can safely submit the proposal.

**Example:**
```
✓ treasury.spend
  Summary: Transfer 1000 USDC from treasury to alice
  Treasury impact: USDC balance 10000 → 9000
```

### Revert (✗)

An action would fail during execution. Do not submit the proposal without fixing the issue.

**Example:**
```
✗ treasury.spend
  Revert reason: Insufficient treasury balance
  Treasury impact: USDC balance 5000 (not enough for 10000 transfer)
```

### Partial Failure

Some actions succeed while others revert. Execution halts at the first revert, so downstream actions are not simulated.

**Example:**
```
✓ governor.queue_proposal
✗ treasury.spend
  Revert reason: Treasury paused
[No further actions simulated]
```

---

## Common Issues

### "Simulation Failed" Error

**Cause:** Backend could not connect to Stellar RPC, or simulation endpoint is unavailable.

**Fix:**
1. Verify `STELLAR_RPC_URL` in the backend `.env` is correct.
2. Check that the RPC endpoint is reachable: `curl https://soroban-testnet.stellar.org`.
3. Ensure backend is running: `npm run dev` in the `backend/` directory.

---

### "No simulation results returned"

**Cause:** Backend simulated the actions but the transaction builder did not produce a valid result.

**Fix:**
1. Verify `targets`, `fnNames`, and `calldatas` are correct.
2. Ensure all target contract addresses are deployed on the network.
3. Check that function names match the contract ABI exactly (case-sensitive).

---

### "Simulation succeeds but on-chain execution fails"

**Cause:** State changed between simulation time and execution time. Another proposal or transaction modified the contracts.

**Fix:**
1. Re-simulate the proposal before execution. Click **"Re-simulate"** to get current results.
2. If results are still positive, the issue may be a race condition — retry execution.
3. If the re-simulation now shows a revert, address the state change and resubmit.

---

### Decoded Summary Shows "Unknown Action"

**Cause:** The backend does not recognize the target contract or function.

**Fix:**
1. Verify the target contract address is registered in the backend's ABI registry.
2. Ensure the function name matches the contract's public API.
3. The simulation still runs (backend makes a best effort), but a human-readable summary is not available.

---

## Treasury Impact Analysis

Simulation results include treasury impact data to help forecast treasury balance changes:

```typescript
if (result.treasuryImpact) {
  const { token, capRemainingBefore, capRemainingAfter } = result.treasuryImpact;
  const amountSpent = capRemainingBefore - capRemainingAfter;
  console.log(`Proposal would spend ${amountSpent} ${token}`);
}
```

Use this to validate that:
- Treasury has sufficient balance for the action.
- Multiple proposals do not over-allocate treasury funds.
- Spending aligns with governance policy.

---

## Simulation History

The backend stores a history of simulations for each proposal, allowing you to compare results across time:

```typescript
const history = await client.getSimulationHistory(proposalId);

history.forEach((entry) => {
  console.log(`Simulated at ledger ${entry.simulatedAtLedger}:`);
  entry.results.forEach((r) => {
    console.log(`  ${r.fnName}: ${r.success ? "✓" : "✗"}`);
  });
});
```

Use simulation history to:
- Identify when a proposal's state crossed from "would succeed" to "would fail".
- Understand the impact of other proposals queued or executed.
- Demonstrate due diligence to stakeholders ("we simulated at 3 different points; results changed at ledger X").

---

## Limitations

### No Cross-Transaction State

Simulations assume each action runs independently. Soroban does not support ordered multi-action simulation within a single transaction as of this document's writing.

**Workaround:** If your proposal has dependent actions, test them separately and reason through the sequence manually.

---

### Limited ABI Coverage

The backend can decode results only for contracts in its ABI registry (governor, treasury, liquidity, etc.). Custom contracts will show "Unknown Action".

**Workaround:** The simulation still runs; you just lack human-readable decoding. Inspect the raw XDR or add the contract's ABI to the backend registry.

---

### Simulation vs. Execution

Simulation reflects the current ledger state. If the proposal is queued and many other proposals are executed before yours, the state may diverge.

**Workaround:** Re-simulate before execution to catch divergence. This is why the UI provides a **"Re-simulate"** button on queued proposals.

---

## API Reference

### POST /proposal-simulation/preview

Preview a draft proposal's actions.

**Request:**
```json
{
  "targets": ["GBBBB...", "GCCCC..."],
  "fnNames": ["spend", "set_strategy"],
  "calldatas": ["base64-encoded XDR", "base64-encoded XDR"],
  "descriptionHash": "0x..." // Optional: for deduplication in simulation history
}
```

**Response:**
```json
{
  "results": [
    {
      "target": "GBBBB...",
      "fn_name": "spend",
      "success": true,
      "decoded_summary": "Transfer 1000 USDC to alice",
      "return_value": null,
      "treasury_impact": {
        "token": "GUSDC...",
        "cap_remaining_before": "10000000",
        "cap_remaining_after": "9000000"
      }
    }
  ],
  "any_action_would_revert": false
}
```

---

### GET /proposal-simulation/:proposalId

Simulate an existing proposal against current state.

**Response:** Same structure as `/preview`.

---

### GET /proposal-simulation/:proposalId/history

Fetch prior simulations for a proposal.

**Response:**
```json
[
  {
    "simulated_at": "2026-09-25T10:30:00Z",
    "simulated_at_ledger": 1000000,
    "any_action_would_revert": false,
    "results": [ ... ]
  },
  {
    "simulated_at": "2026-09-25T10:35:00Z",
    "simulated_at_ledger": 1001000,
    "any_action_would_revert": true,
    "results": [ ... ]
  }
]
```

---

## See Also

- [Governor Contract](./architecture.md#governor) — Understanding proposal lifecycle and execution.
- [Treasury Contract](./architecture.md#treasury) — Treasury operations and constraints.
- [Deployment Guide](./deployment.md) — Setting up simulation service on your network.
