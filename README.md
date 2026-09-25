# NebGov

**Permissionless on-chain governance for every Soroban protocol.**

NebGov is the canonical governance framework for the Stellar ecosystem — a modular, auditable, and composable set of smart contracts that any Soroban protocol can plug into to add on-chain governance.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![CI](https://github.com/nebgov/nebgov/actions/workflows/rust.yml/badge.svg)](https://github.com/nebgov/nebgov/actions)
[![codecov](https://codecov.io/gh/nebgov/nebgov/graph/badge.svg)](https://codecov.io/gh/nebgov/nebgov)
[![Security Policy](https://img.shields.io/badge/Security-Policy-2f855a)](./SECURITY.md)

---

## What It Does

| Feature                | Description                                         |
| ---------------------- | --------------------------------------------------- |
| Proposal lifecycle     | Create, vote, queue, and execute on-chain proposals |
| Timelock execution     | Mandatory delay between passing and execution       |
| Token-weighted voting  | Snapshot voting power from any SEP-41 token         |
| Delegation             | Delegate voting power to any address, in full or split by percentage ([docs](./docs/split-delegation.md)) |
| Time-locked voting boost | Lock tokens to gain amplified, linearly-decaying voting power ([docs](./docs/vote-escrow.md)) |
| Multi-sig treasury     | DAO-controlled treasury with configurable threshold |
| Treasury yield strategies | Governance-controlled allocation of idle treasury funds to whitelisted yield adapters ([docs](./docs/treasury-strategies.md)) |
| Permissionless factory | Deploy your own governance instance in one call     |
| Conviction voting      | Self-executing voting based on staking duration and amount ([docs](./docs/conviction-voting.md)) |
| Optimistic governance  | Fast-track proposals that execute unless challenged ([docs](./docs/optimistic-governance.md)) |
| Proposal bonds         | Refundable proposer bonds with governance-triggered slashing ([docs](./docs/proposal-bonds.md)) |
| Gasless signaling      | Off-chain polling with on-chain result anchoring ([docs](./docs/signal-anchor.md)) |

---

## Packages

### Smart contracts (Rust/Soroban)

| Package                         | Description                                                      |
| ------------------------------- | ---------------------------------------------------------------- |
| `contracts/governor`            | Core governance contract — proposal lifecycle, voting, execution |
| `contracts/timelock`            | Delayed execution controller                                     |
| `contracts/token-votes`         | Voting power with checkpointing and delegation                   |
| `contracts/governor-factory`    | Permissionless governor deployer                                 |
| `contracts/treasury`            | Multi-sig treasury with budget streams                           |
| `contracts/token-votes-wrapper` | SEP-41 token wrapper that adds governance voting                 |
| `contracts/co-sponsorship`      | Pool voting power to meet the proposal threshold                 |
| `contracts/liquidity`           | Protocol-owned liquidity pools (constant-product AMM)            |
| `contracts/proposal-bonds`      | Refundable proposer bonds with governance-triggered slashing     |
| `contracts/optimistic-governor` | Object-to-block optimistic proposal track                        |
| `contracts/conviction-voting`   | Self-executing conviction voting                                 |
| `contracts/treasury-strategies` | Governance-controlled yield allocation for idle treasury funds   |
| `contracts/signal-anchor`       | On-chain anchoring of off-chain signaling poll results           |
| `contracts/vote-escrow`         | Time-locked voting power boost (vote-escrow model)               |
| `contracts/voting-rewards`      | Epoch-based, Merkle-claimed voting participation rewards         |

### TypeScript packages

| Package             | Description                                                |
| ------------------- | ---------------------------------------------------------- |
| `sdk/`              | TypeScript SDK (`@nebgov/sdk`)                             |
| `app/`              | Next.js governance dashboard                               |
| `backend/`          | Off-chain REST API (notifications, relayer, signaling)     |
| `packages/indexer/` | Off-chain governance event indexer (`@nebgov/indexer`)     |
| `packages/cli/`     | Terminal-based governance workflows (`@nebgov/cli`)        |

### Tooling

| Package             | Description                                                            |
| ------------------- | ---------------------------------------------------------------------- |
| `tools/sim/`        | Rust governance simulation harness (`nebgov-sim`)                      |
| `tools/simulation/` | Lifecycle simulation against a mock Soroban RPC (`@nebgov/simulation`) |

---

## Quick Start

Get started by deploying your first NebGov DAO to the Stellar testnet in under 10 minutes:

👉 **[Deploy your first DAO on Stellar with NebGov](./docs/tutorial.md)**

### Local development stack (Docker)

Bring up Postgres + indexer + backend + app with one command:

```bash
cp .env.example .env
# Set GOVENOR_ADDRESS in .env (required)
docker compose up --build
```

Then open:

- App: `http://localhost:3000`
- Backend health: `http://localhost:3001/health`
- Indexer health: `http://localhost:3002/health`

For full setup instructions and contribution guidelines, see [CONTRIBUTING.md](./CONTRIBUTING.md).

For a step-by-step local development guide, see [docs/local-development.md](./docs/local-development.md).

---

## Architecture

See [docs/architecture.md](./docs/architecture.md) for the full design overview.

## Production Deployment Guides

- [docs/deployment.md](./docs/deployment.md) — full production deployment walkthrough for contracts, backend, indexer, and frontend
- [docs/parameter-guide.md](./docs/parameter-guide.md) — safe governance parameter ranges and preset configurations
- [docs/security.md](./docs/security.md) — treasury reentrancy analysis and contract security notes
- [docs/deployments.md](./docs/deployments.md) — official contract addresses for all networks
- [docs/troubleshooting.md](./docs/troubleshooting.md) — RPC, wallet, transaction, and contract error reference

```
propose() → Governor → [voting period] → queue() → Timelock → execute()
                ↓
          Token Votes (snapshot voting power)
```

---

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) to get started.

Issues are labeled by complexity:

- `complexity: trivial`
- `complexity: medium`
- `complexity: high`

---

## Ecosystem Integrations

- **[Reflector Oracle](https://reflector.network)** — dynamic quorum based on token price
- **[Stellar Wallets Kit](https://github.com/Creit-Tech/Stellar-Wallets-Kit)** — multi-wallet support in the frontend
- **[OpenZeppelin Contracts for Stellar](https://github.com/OpenZeppelin/openzeppelin-contracts-stellar)** — composable token standards

---

## License

MIT

## Security

If you discover a vulnerability, please review [SECURITY.md](./SECURITY.md) and report it privately instead of opening a public issue.
