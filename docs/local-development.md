# Local Development Setup

This guide walks you through setting up a complete local development environment for NebGov — including contracts, backend, indexer, and frontend.

## Prerequisites

- **Rust** (stable toolchain) with `wasm32-unknown-unknown` target
- **Stellar CLI** (`stellar`) for contract building and testing
- **Node.js 20+** with **pnpm 9+**
- **Docker** and **Docker Compose** for running the local stack
- **Git**

## Installation

### 1. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown
```

### 2. Install Stellar CLI

```bash
cargo install --locked stellar-cli
```

Verify installation:

```bash
stellar --version
```

### 3. Install Node.js

If you use nvm:

```bash
nvm install 20
npm install -g pnpm@9
```

Or download from https://nodejs.org/

### 4. Install Docker

Download Docker Desktop from https://www.docker.com/products/docker-desktop

## Project Setup

### Clone and Initialize

```bash
git clone https://github.com/nebgov/nebgov.git
cd nebgov

# Install all workspace dependencies
pnpm install

# Build the TypeScript SDK (required for CLI and backend)
pnpm build:sdk
```

## Running the Full Stack Locally

### With Docker Compose (Recommended)

The `docker-compose.yml` sets up:
- **Soroban RPC** (localnet Stellar simulation)
- **PostgreSQL** (for indexer and backend)
- **Indexer** (event processor)
- **Backend API** (notifications, relayer, signaling)
- **Frontend** (Next.js app)

#### Start the Stack

```bash
# Copy environment template
cp .env.example .env

# Edit .env and set GOVERNOR_ADDRESS (required for backend to work)
# You can use a testnet address or deploy a local contract first
export GOVERNOR_ADDRESS="CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4"

# Build and run all services
docker compose up --build
```

Wait for all services to be healthy (2-3 minutes):

```
indexer is ready
backend is ready
postgres is healthy
app is running
```

#### Access the Stack

Once running, open:

- **Frontend**: http://localhost:3000
- **Backend API**: http://localhost:3001/health
- **Indexer**: http://localhost:3002/health
- **GraphQL**: http://localhost:3001/graphql (if enabled)

#### Stop the Stack

```bash
docker compose down

# Remove volumes to reset database
docker compose down -v
```

### Manual Development (Without Docker)

If you prefer to run services locally without Docker:

#### Terminal 1: PostgreSQL

```bash
# Install PostgreSQL (via Homebrew on macOS)
brew install postgresql

# Start the server
brew services start postgresql

# Create development database
createdb nebgov_dev
```

#### Terminal 2: Contracts & Build

```bash
cd /Users/mac/drips/nebgov

# Build contracts
stellar contract build

# Build SDK
pnpm build:sdk
```

#### Terminal 3: Indexer

```bash
cd packages/indexer

# Set environment (uses .env)
export DATABASE_URL="postgresql://localhost/nebgov_dev"
export SOROBAN_RPC="https://soroban-testnet.stellar.org"

# Install and run
pnpm install
pnpm start
```

#### Terminal 4: Backend

```bash
cd backend

export DATABASE_URL="postgresql://localhost/nebgov_dev"
export GOVERNOR_ADDRESS="CAD..."  # Set your governor address

pnpm install
pnpm dev
```

#### Terminal 5: Frontend

```bash
cd app

pnpm install
pnpm dev
```

Access at http://localhost:3000

## Running Tests

### All Tests (Rust + TypeScript)

```bash
make check
```

This runs:
- Contract tests: `cargo test --workspace`
- TypeScript tests: `pnpm test:sdk && pnpm test:app && pnpm test:cli`

### Individual Test Suites

**Rust contracts:**

```bash
cargo test --workspace

# Single contract
cargo test -p sorogov-governor

# Single test by name
cargo test -p sorogov-governor test_propose_succeeds
```

**SDK tests:**

```bash
pnpm test:sdk
```

**Frontend tests:**

```bash
cd app
pnpm test
```

**E2E tests (requires running app):**

```bash
cd app
npx playwright test
```

**CLI tests:**

```bash
pnpm build:sdk && pnpm test:cli
```

## Building for Deployment

### Contracts (WASM)

```bash
# Optimized release build
stellar contract build --profile release

# Outputs to target/wasm32-unknown-unknown/release/*.wasm
```

### Frontend

```bash
cd app
pnpm build
pnpm start  # Start production server
```

### Backend & Indexer

```bash
cd backend
pnpm build

cd packages/indexer
pnpm build
```

## Useful Commands

### Format Code

```bash
# Rust
cargo fmt --all

# TypeScript
pnpm lint:fix
```

### Type Check

```bash
pnpm typecheck
```

### View Database Schema

```bash
# With PostgreSQL running
psql nebgov_dev

# List tables
\dt

# View schema for a table
\d <table_name>

# Exit
\q
```

### View Contract Storage

After deploying a contract locally:

```bash
stellar contract read \
  --contract-id <CONTRACT_ID> \
  --network standalone  # or futurenet
```

## Environment Variables

Key environment variables used in development:

| Variable | Purpose | Example |
| --- | --- | --- |
| `DATABASE_URL` | PostgreSQL connection | `postgresql://localhost/nebgov_dev` |
| `SOROBAN_RPC` | Stellar RPC endpoint | `https://soroban-testnet.stellar.org` |
| `GOVERNOR_ADDRESS` | Deployed governor contract | `CAD3...` |
| `SECRET_KEY` | Account funding key (testnet only) | `SAAAA...` |
| `LOG_LEVEL` | Backend logging | `debug`, `info`, `warn` |

See `.env.example` for a complete list.

## Troubleshooting

**Port already in use:**

```bash
# Find and kill process on port
lsof -i :3000
kill -9 <PID>
```

**Contract build fails:**

```bash
# Update Stellar CLI
cargo install --locked stellar-cli --force

# Clean build
rm -rf target
stellar contract build
```

**Database connection error:**

```bash
# Check PostgreSQL is running
brew services list

# Reset database
dropdb nebgov_dev
createdb nebgov_dev
```

**Docker image build fails:**

```bash
# Rebuild without cache
docker compose build --no-cache

# Check logs
docker compose logs indexer  # or backend, app, etc.
```

**GraphQL/SDK generation out of sync:**

```bash
# Regenerate SDK from contract ABI
pnpm build:sdk
```

## Next Steps

- **Deploy to testnet**: See [Tutorial](./tutorial.md)
- **Understand the architecture**: Read [Architecture Guide](./architecture.md)
- **Contributing**: Check [CONTRIBUTING.md](../CONTRIBUTING.md)
- **Contract security**: See [Security Guide](./security.md)

## Getting Help

- **Issues**: https://github.com/nebgov/nebgov/issues
- **Discussions**: https://github.com/nebgov/nebgov/discussions
- **Stellar Dev Docs**: https://developers.stellar.org

Happy hacking! 🚀
