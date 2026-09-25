# Governance Health Score

The Governance Health Score provides a real-time snapshot of governance participation and activity, helping stakeholders understand the health and engagement level of the NebGov system.

---

## Overview

The Governance Health Score aggregates on-chain data to present key metrics about governance participation. Rather than a single composite score, it surfaces multiple dimensions of governance health, allowing informed assessment of network engagement.

### Key Metrics

#### Total Proposals

The cumulative count of all proposals ever created on the network, regardless of their final state (Pending, Active, Succeeded, Defeated, Queued, Executed, or Expired).

**Interpretation:** Indicates the overall activity level and frequency of governance participation.

---

#### Total Votes Cast

The total number of individual vote transactions submitted on the network, across all proposals and voters.

**Interpretation:** Reflects engagement intensity and the depth of participation during voting periods.

---

#### Unique Voters

The count of distinct wallet addresses that have cast at least one vote.

**Interpretation:** Measures voter diversity and decentralization. Higher numbers suggest broader governance participation across the community.

---

#### Quorum Performance

- **Quorum Hit:** The number of proposals that achieved quorum (met the minimum voting participation threshold).
- **Quorum Miss:** The number of proposals that failed to reach quorum.

**Interpretation:** Consecutive quorum misses may indicate inadequate governance participation or suboptimal quorum thresholds. See [Governor Configuration](./architecture.md#governor) for tuning guidance.

---

#### Pass Rate

The percentage of proposals (in basis points, 0–10,000 where 10,000 = 100%) that succeeded after reaching quorum.

**Interpretation:** A low pass rate may indicate polarized or contested proposals; a high pass rate suggests consensus-driven governance.

---

## Accessing Governance Health Data

### Frontend Component

The Governance Health Score is displayed via the `GovernanceHealthScore` React component in the NebGov web application:

```tsx
import { GovernanceHealthScore } from "@/components/GovernanceHealthScore";
import { AllTimeStats } from "@nebgov/sdk";

function Dashboard({ stats }: { stats: AllTimeStats | null }) {
  return <GovernanceHealthScore stats={stats} loading={false} />;
}
```

### SDK Integration

Retrieve governance health statistics using the `VotesClient` from the NebGov SDK:

```typescript
import { VotesClient } from "@nebgov/sdk";

const votesClient = new VotesClient({
  governorAddress: "...",
  network: "testnet",
});

const distribution = await votesClient.getVotingPowerDistribution();
```

The SDK also provides indexer-driven queries through the backend API, which aggregates historical voting and proposal data.

---

## Data Sources

All governance health metrics originate from:

1. **On-Chain Governor Events:** Proposal creation, vote casting, and state changes logged by the Governor contract.
2. **Indexer:** The NebGov indexer processes on-chain events and stores aggregated statistics in a PostgreSQL database.
3. **Backend API:** The backend exposes query endpoints for the frontend to fetch pre-computed health statistics.

---

## Understanding Governance Health

### Healthy Indicators

- ✅ Gradual growth in unique voters over time
- ✅ Consistent quorum achievement
- ✅ Regular proposal creation cycles
- ✅ Diverse voting patterns (not dominated by single addresses)

### Warning Signs

- ⚠️ Declining or stagnant proposal creation
- ⚠️ Repeated quorum misses
- ⚠️ High concentration of voting power (low Gini coefficient)
- ⚠️ Very low participation rates
- ⚠️ Long gaps between voting events

---

## Governance Tuning

If governance health metrics indicate problems, consider:

### Quorum Too High

**Symptom:** Frequent quorum misses despite active voting.

**Remedy:** Lower the `QUORUM_NUMERATOR` in the Governor configuration. See [architecture.md](./architecture.md#governor) for parameters.

### Voter Participation Low

**Symptom:** Few unique voters despite numerous proposals.

**Remedy:**
1. Ensure voting power delegation is enabled and clear to users.
2. Confirm token holders are aware of voting periods and proposals.
3. Monitor wallet integration for UX friction in the voting flow.

### Voting Power Concentration

**Symptom:** A small number of addresses hold most voting power.

**Remedy:**
1. Consider vote-splitting mechanisms (see [split-delegation.md](./split-delegation.md)).
2. Evaluate if voting power distribution reflects the governance model's intent.
3. Use conviction voting to give smaller token holders increased influence (see [conviction-voting.md](./conviction-voting.md)).

---

## FAQ

**Q: Why is the Governance Health Score neutral in color (no green/yellow/red)?**

A: The health of a governance system depends on the organization's policies and goals. A "low" pass rate is not inherently bad (it reflects contested proposals); a "high" quorum hit rate is not automatically good (it may reflect a low quorum threshold). Use the individual metrics to assess health relative to your governance model.

**Q: How often is the health score updated?**

A: The indexer polls the network every `POLL_INTERVAL_MS` (default: 5 seconds). Statistics are updated whenever new voting or proposal events are detected.

**Q: Can I query historical health metrics?**

A: The indexer stores full voting and proposal history. Use the backend API or query the indexer database directly to retrieve snapshots at specific ledger ranges. The `VotesClient` supports filtering by `fromLedger` for historical queries.

**Q: What is the Gini coefficient?**

A: A statistical measure of inequality in voting power distribution. 0 = perfectly equal, 1 = maximally concentrated. Used to identify governance health risks from power centralization.
