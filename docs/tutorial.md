# Deploy Your First DAO on Stellar with NebGov

In this tutorial, you'll deploy your first NebGov governance instance to the Stellar testnet in under 10 minutes. You'll learn how to set up contracts, initialize a DAO, and execute your first governance proposal.

## Prerequisites

- **Stellar CLI**: Install from https://developers.stellar.org/docs/cli
- **Futurenet account** with testnet credentials (get free XLM at https://friendbot.stellar.org)
- **Node.js 20+** and **pnpm 9+** (for SDK interaction)

## Step 1: Build Contracts (5 minutes)

Clone and build the NebGov contracts:

```bash
git clone https://github.com/nebgov/nebgov.git
cd nebgov

# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown

# Install Stellar CLI
cargo install --locked stellar-cli

# Build all contracts to WASM
stellar contract build
```

## Step 2: Deploy Governor Contract (2 minutes)

Set your testnet credentials and deploy:

```bash
export STELLAR_NETWORK=futurenet
export SECRET_KEY="your-secret-key-here"

# Deploy governor contract
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/sorogov_governor.wasm \
  --network futurenet \
  --source $SECRET_KEY

# Save the returned contract address
export GOVERNOR_ADDRESS="CAD..." # Copy from output above
```

## Step 3: Initialize Governor

Create `init_governor.json`:

```json
{
  "proposal_duration": 604800,
  "timelock_delay": 86400,
  "quorum_numerator": 40,
  "quorum_denominator": 100,
  "voting_token": "native",
  "treasury": "CAD..."
}
```

Initialize via Stellar CLI:

```bash
stellar contract invoke \
  --contract-id $GOVERNOR_ADDRESS \
  --network futurenet \
  --source $SECRET_KEY \
  -- initialize \
  --config init_governor.json
```

## Step 4: Deploy Token Votes (1 minute)

Deploy voting power contract:

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/sorogov_token_votes.wasm \
  --network futurenet \
  --source $SECRET_KEY

export VOTES_ADDRESS="CAD..." # Copy from output
```

Initialize with your token:

```bash
stellar contract invoke \
  --contract-id $VOTES_ADDRESS \
  --network futurenet \
  --source $SECRET_KEY \
  -- initialize \
  --token $TOKEN_ADDRESS \
  --governor $GOVERNOR_ADDRESS
```

## Step 5: Create Your First Proposal (1 minute)

Use the TypeScript SDK to create a proposal. First, set up the SDK:

```bash
cd sdk
pnpm install
pnpm build
```

Create `deploy.ts`:

```typescript
import { Governor } from "@nebgov/sdk";

const governor = new Governor({
  rpc: "https://soroban-futurenet.stellar.org",
  network: "futurenet",
  contractId: process.env.GOVERNOR_ADDRESS!,
});

const proposal = await governor.propose({
  title: "Mint 1M tokens to treasury",
  description: "Initial funding for governance operations",
  actions: [
    {
      target: process.env.TOKEN_ADDRESS!,
      action: "mint",
      params: { amount: "1000000000000" },
    },
  ],
});

console.log("Proposal created:", proposal.id);
```

Run it:

```bash
export GOVERNOR_ADDRESS="CAD..."
export TOKEN_ADDRESS="CAD..."
npx ts-node deploy.ts
```

## Next Steps

- **Vote on proposals**: Learn more in [Governance Workflow](./architecture.md)
- **Set up local development**: See [Local Development Guide](./local-development.md)
- **Integrate with your protocol**: Check out the [SDK documentation](../sdk/README.md)
- **Review contract architecture**: Read [Architecture Guide](./architecture.md)

## Troubleshooting

**"Contract not found" error**: Verify the contract address is correct and deployed to the right network.

**"Insufficient balance" error**: Fund your account at https://friendbot.stellar.org

**"Invalid parameter" error**: Check the contract initialization JSON against the contract's expected types in [contracts/governor/](../contracts/governor/)

For more help, see [Troubleshooting Guide](./troubleshooting.md).

## Congratulations! 🎉

You've deployed your first DAO on Stellar. You can now:
- Create and vote on proposals
- Delegate voting power
- Execute proposals through timelock
- Extend with optional modules (conviction voting, optimistic governance, etc.)

For questions or issues, open a discussion on [GitHub](https://github.com/nebgov/nebgov/discussions).
