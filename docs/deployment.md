# Production deployment guide

This guide covers a secure, production-grade deployment of the full NebGov stack:

- Smart contracts
- Backend API
- Indexer
- Frontend application

The repository already includes deployment helpers and environment templates in [../scripts/deploy-testnet.sh](../scripts/deploy-testnet.sh), [../.env.example](../.env.example), and [../docker-compose.yml](../docker-compose.yml). Use them as a baseline, but treat production as a separate environment with stricter controls.

## 1. Pre-deployment checklist

Complete the following before you cut over to production:

- [ ] Contract audit completed and remediation issues closed.
- [ ] Environment variables reviewed; no development defaults remain in production.
- [ ] Database backup completed and restore procedure tested.
- [ ] Access keys and deployment credentials rotated.
- [ ] Production RPC endpoint chosen and documented.
- [ ] TLS certificates and reverse proxy configured.
- [ ] Monitoring, alerting, and log retention enabled.
- [ ] A rollback plan exists for contracts, backend, and frontend.

## 2. Prepare the deployment environment

### Required tooling

Install the following on the deployment runner:

- Rust and Cargo
- Stellar CLI (`stellar`)
- Node.js 20+ and pnpm
- Docker and Docker Compose (for indexer/container deployments)
- PostgreSQL client tools (for manual database setup)

### Production secrets

Store secrets in a secret manager or protected CI/CD variable store. Never hard-code them in the repository or in frontend builds.

Recommended secrets:

- `JWT_SECRET`
- `POSTGRES_PASSWORD`
- `STELLAR_PRIVATE_KEY` or deployment identity secret
- Any admin or guardian keys used by the governance instance

## 3. Deploy the contracts

### 3.1 Build the optimized WASM artifacts

Build the contracts using a release profile:

```bash
cargo build --release --target wasm32-unknown-unknown --manifest-path Cargo.toml --workspace
```

> The repository deployment helper in [../scripts/deploy-testnet.sh](../scripts/deploy-testnet.sh) uses the current Stellar toolchain target and is a good reference for local automation.

### 3.2 Create or select a deployment identity

Use a dedicated deployment identity that is not shared with day-to-day operations:

```bash
stellar keys generate --global nebgov-prod
stellar keys address nebgov-prod
```

Fund the account on the target network before deployment. For testnet, use friendbot; for mainnet, fund it from a secure source account.

### 3.3 Deploy the contracts

Example deployment flow:

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/sorogov_token_votes.wasm \
  --source nebgov-prod \
  --network public
```

Repeat the same pattern for each contract artifact:

- `sorogov_token_votes.wasm`
- `sorogov_timelock.wasm`
- `sorogov_governor.wasm`
- `sorogov_treasury.wasm`
- `sorogov_governor_factory.wasm`

### 3.4 Verify the deployment

After each deployment, verify that the returned contract ID is present and usable:

```bash
stellar contract inspect --id <CONTRACT_ID> --network public
```

Record the contract IDs in a secure deployment record and rotate any temporary placeholder values that were used during initialization.

### 3.5 Initialize governance settings

Initialize the governor, timelock, treasury, and token-votes contracts with production-safe values. Example:

```bash
stellar contract invoke \
  --id <TOKEN_VOTES_ID> \
  --source nebgov-prod \
  --network public \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --token <SEP41_TOKEN_ADDRESS>
```

Use governance parameters that match your intended risk profile and review them carefully before activation.

## 3.6 Upgrade a deployed contract

Use `scripts/upgrade-contract.sh` to submit a governance-driven WASM upgrade proposal for any of the following contracts:

- `sorogov_governor`
- `sorogov_timelock`
- `sorogov_token_votes`
- `sorogov_treasury`
- `sorogov_governor_factory`
- `sorogov_proposal_bonds`
- `sorogov_optimistic_governor`
- `sorogov_conviction_voting`
- `sorogov_treasury_strategies`
- `sorogov_signal_anchor`
- `sorogov_vote_escrow`
- `sorogov_voting_rewards`

`sorogov_vote_escrow` and `sorogov_voting_rewards` hold user funds (locked tokens and an unclaimed reward pool respectively), so rehearsing the upgrade path against testnet before a mainnet upgrade is strongly recommended.

```bash
GOVERNOR_ADDRESS=<GOVERNOR_CONTRACT_ID> \
DEPLOYER_ADDR=<YOUR_ADDRESS> \
  ./scripts/upgrade-contract.sh sorogov_vote_escrow
```

The script builds the WASM, installs it on-chain, and submits an `upgrade(wasm_hash)` governance proposal. Follow the printed next-steps to vote, queue, and execute the proposal.

## 3.7 Deploy and initialize the optional modules

The five newest contracts are optional and are not deployed by `scripts/deploy-testnet.sh`. Deploy only the ones your instance needs, using the same `stellar contract deploy` pattern as section 3.3 with these artifacts:

- `sorogov_proposal_bonds.wasm`
- `sorogov_optimistic_governor.wasm`
- `sorogov_conviction_voting.wasm`
- `sorogov_treasury_strategies.wasm`
- `sorogov_signal_anchor.wasm`

Each contract must be initialized once, right after deployment. `initialize` requires the admin's authorization and fails with `AlreadyInitialized` if it is called again. Deploy and initialize the core contracts first, because several of these modules take the governor, votes token, or treasury address as an argument.

| Contract | Depends on | `initialize` arguments |
| --- | --- | --- |
| Proposal bonds | Governor, bond token (SEP-41) | `admin`, `bond_token`, `bond_amount` (must be > 0), `governor`, `refund_grace_ledgers`, `max_lock_ledgers` |
| Optimistic governor | Votes token (`sorogov_token_votes`) | `admin`, `votes_token`, `challenge_window_ledgers` (must be > 0), `objection_threshold_bps` (1–10000), `proposer_bond_amount` (≥ 0, `0` disables the bond), `proposer_bond_token` |
| Conviction voting | Votes token (`sorogov_token_votes`) | `admin`, `votes_token`, `decay_bps` (1–9999), `max_ratio_bps` (1–10000), `min_threshold_conviction` (must be > 0), `weight_bps` (1–10000) |
| Treasury strategies | Treasury (`sorogov_treasury`) | `admin`, `treasury` |
| Signal anchor | None | `admin` |

Initialize each deployed contract with the `stellar contract invoke` pattern from section 3.5. For example, for the signal anchor:

```bash
stellar contract invoke \
  --id <SIGNAL_ANCHOR_ID> \
  --source nebgov-prod \
  --network public \
  -- initialize \
  --admin <ADMIN_ADDRESS>
```

Follow the same pattern for the other contracts, passing the arguments from the table above. Use the contract IDs you recorded in section 3.4 for any address argument.

After initializing the treasury strategies contract, register at least one yield adapter with `register_strategy`. Its arguments are `admin`, `adapter`, `token`, `max_allocation_bps` (≤ 10000), and `withdrawal_cooldown_ledgers`. Only the treasury can call `deposit` and `request_withdrawal`, so keep the `treasury` argument set to the treasury contract ID. See [treasury-strategies.md](./treasury-strategies.md) for the adapter model and governance controls.

Verify each deployment with `stellar contract inspect` (section 3.4) and record its contract ID with the others.

## 4. Deploy the backend

### 4.1 Prepare the database

Create a production database and run the backend migrations:

```bash
createdb nebgov_prod
cd backend
pnpm install
pnpm run build
pnpm run migrate
```

If you use a managed PostgreSQL instance, confirm that the database user has the correct privileges and that the connection pool size matches your expected traffic.

### 4.2 Configure backend environment variables

Set the following environment variables in the deployment environment:

```bash
export PORT=3001
export DATABASE_URL="postgresql://USER:PASSWORD@HOST:5432/DB"
export JWT_SECRET="<long-random-secret>"
export FRONTEND_URL="https://governance.example.com"
```

### 4.3 Run the backend service

Use a process manager such as PM2 or systemd. Example PM2 configuration:

```bash
pm2 start "cd backend && pnpm run start" --name nebgov-backend
pm2 save
```

### 4.4 Reverse proxy and TLS

Place the backend behind Nginx or another reverse proxy with TLS enabled. Example Nginx location block:

```nginx
location / {
  proxy_pass http://127.0.0.1:3001;
  proxy_http_version 1.1;
  proxy_set_header Host $host;
  proxy_set_header X-Forwarded-Proto $scheme;
  proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```

Ensure the proxy enforces HTTPS and does not expose the backend directly on the public internet unless that is a deliberate design choice.

## 5. Deploy the frontend

### 5.1 Configure public environment variables

The frontend uses `NEXT_PUBLIC_*` variables for values that must be available to the browser. Keep secrets out of this namespace.

Set the following values:

```bash
export NEXT_PUBLIC_NETWORK="public"
export NEXT_PUBLIC_BACKEND_URL="https://api.governance.example.com"
export NEXT_PUBLIC_GOVERNOR_ADDRESS="<GOVERNOR_CONTRACT_ID>"
export NEXT_PUBLIC_TIMELOCK_ADDRESS="<TIMELOCK_CONTRACT_ID>"
export NEXT_PUBLIC_VOTES_ADDRESS="<TOKEN_VOTES_CONTRACT_ID>"
export NEXT_PUBLIC_RPC_URL="https://soroban-rpc.mainnet.stellar.gateway.fm"
export NEXT_PUBLIC_INDEXER_URL="https://indexer.governance.example.com"
```

### 5.2 Build the frontend

```bash
cd app
pnpm install
pnpm build
```

### 5.3 Publish the frontend

Deploy the built output to one of the following:

- Vercel or Netlify for managed hosting
- Self-hosted Node.js with a reverse proxy
- A container platform such as Docker or Kubernetes

When you deploy behind a proxy, verify that the production response includes a strict Content Security Policy (CSP) and that the app only connects to approved backend and indexer origins.

## 6. Deploy the indexer

### 6.1 Prepare the indexer database and runtime

The indexer uses PostgreSQL and a dedicated Stellar RPC endpoint. It should not rely on a public endpoint for production traffic when a private or dedicated RPC node is available.

### 6.2 Example production Docker Compose snippet

```yaml
version: "3.9"
services:
  db:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: nebgov_prod
      POSTGRES_USER: nebgov_prod
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - pgdata:/var/lib/postgresql/data
    restart: unless-stopped

  indexer:
    image: ghcr.io/nebgov/indexer:latest
    environment:
      DATABASE_URL: postgresql://nebgov_prod:${POSTGRES_PASSWORD}@db:5432/nebgov_prod
      STELLAR_RPC_URL: https://soroban-rpc.mainnet.stellar.gateway.fm
      STELLAR_NETWORK_PASSPHRASE: "Public Global Stellar Network ; September 2015"
      GOVERNOR_ADDRESS: ${GOVERNOR_ADDRESS}
      TREASURY_ADDRESS: ${TREASURY_ADDRESS}
      TREASURY_SIMULATION_ACCOUNT: ${TREASURY_SIMULATION_ACCOUNT}
      TIMELOCK_ADDRESS: ${TIMELOCK_ADDRESS}
      POLL_INTERVAL_MS: 5000
      PORT: 3002
    depends_on:
      - db
    restart: unless-stopped

  backend:
    image: ghcr.io/nebgov/backend:latest
    environment:
      DATABASE_URL: postgresql://nebgov_prod:${POSTGRES_PASSWORD}@db:5432/nebgov_prod
      JWT_SECRET: ${JWT_SECRET}
      FRONTEND_URL: https://governance.example.com
      PORT: 3001
    depends_on:
      - db
    restart: unless-stopped

volumes:
  pgdata:
```

### 6.3 Verify indexer health and lag

Check the indexer health endpoint and confirm that the latest indexed ledger is close to the current network ledger before going live.

## 7. Post-deployment verification

Run a smoke-test checklist before announcing the deployment complete:

- [ ] Frontend loads without runtime errors.
- [ ] Backend health endpoint returns `200`.
- [ ] Indexer health endpoint returns `200` and lag is acceptable.
- [ ] A new proposal can be created.
- [ ] A vote can be cast successfully.
- [ ] The proposal can be queued and executed according to governance rules.
- [ ] Notifications and webhooks are delivered.
- [ ] Monitoring dashboards and alerts are active.
- [ ] Logs are flowing to the central log system.

## 8. Environment variable reference

| Service | Variable | Required | Purpose |
| --- | --- | --- | --- |
| Contracts | `STELLAR_IDENTITY` | Yes | Stellar CLI identity used for deployment |
| Contracts | `STELLAR_NETWORK` | Yes | Network name such as `public` or `testnet` |
| Contracts | `STELLAR_RPC_URL` | Recommended | Dedicated RPC endpoint |
| Contracts | `SEP41_TOKEN_ADDRESS` | Conditional | Token address for votes initialization |
| Contracts | `NATIVE_TOKEN_ADDRESS` | Conditional | Fallback native token address |
| Backend | `DATABASE_URL` | Yes | PostgreSQL connection string |
| Backend | `JWT_SECRET` | Yes | Secret used to sign backend tokens |
| Backend | `FRONTEND_URL` | Yes | Allowed CORS origin |
| Backend | `PORT` | No | Backend listen port |
| Frontend | `NEXT_PUBLIC_NETWORK` | Yes | Public network name |
| Frontend | `NEXT_PUBLIC_BACKEND_URL` | Yes | Backend origin for browser requests |
| Frontend | `NEXT_PUBLIC_GOVERNOR_ADDRESS` | Yes | Governor contract address |
| Frontend | `NEXT_PUBLIC_TIMELOCK_ADDRESS` | Yes | Timelock contract address |
| Frontend | `NEXT_PUBLIC_VOTES_ADDRESS` | Yes | Token-votes contract address |
| Frontend | `NEXT_PUBLIC_RPC_URL` | Recommended | Public RPC endpoint |
| Frontend | `NEXT_PUBLIC_INDEXER_URL` | Recommended | Indexer origin |
| Indexer | `DATABASE_URL` | Yes | PostgreSQL connection string |
| Indexer | `STELLAR_RPC_URL` | Yes | Dedicated RPC endpoint |
| Indexer | `GOVERNOR_ADDRESS` | Yes | Governor contract address to index |
| Indexer | `TREASURY_ADDRESS` | No | Treasury contract address to index |
| Indexer | `TREASURY_SIMULATION_ACCOUNT` | With treasury | Funded account used for treasury read simulations |
| Indexer | `STELLAR_NETWORK_PASSPHRASE` | No | Network passphrase for treasury read simulations |
| Indexer | `TIMELOCK_ADDRESS` | No | Timelock contract address to index |
| Indexer | `POLL_INTERVAL_MS` | No | Polling interval |
| Indexer | `PORT` | No | Indexer listen port |

## 9. Common deployment errors

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| Contract deployment fails with a wasm target error | The target was not installed or is unsupported | Install the required Rust target and re-run the build |
| Backend cannot connect to Postgres | Bad connection string or network rules | Verify `DATABASE_URL`, host reachability, and firewall rules |
| Frontend shows missing contract configuration | `NEXT_PUBLIC_*` values were not set | Rebuild and redeploy with the correct public contract addresses |
| CORS errors appear in browser dev tools | The backend `FRONTEND_URL` or CORS policy does not match the deployed origin | Update the allowed origin to the production frontend URL |
| Indexer has stale data | RPC endpoint or polling settings are incorrect | Confirm the RPC endpoint and monitor indexer lag |
| TLS or proxy errors appear | Reverse proxy is not forwarding headers correctly | Check proxy configuration and certificate chain |

## 10. Security checklist

- Use strong, randomly generated secrets and rotate them regularly.
- Keep deployment identities in a dedicated secret store; do not store private keys in the repository.
- Restrict database access to the application subnet or service network.
- Only expose the frontend and API through HTTPS.
- Set CORS to the exact production origin(s), not `*`, unless you have a clear reason.
- Verify CSP headers and disable unnecessary debug endpoints in production.
- Restrict admin actions to a small number of authorized operators.
- Enable audit logging and alerting for deployments, config changes, and failed auth events.

## 11. Rollback plan

If the deployment causes issues:

1. Revert the frontend and backend releases.
2. Restore the previous database snapshot if data integrity is affected.
3. Revert any contract initialization changes that should not remain live.
4. Confirm monitoring and health checks return to normal before resuming traffic.
