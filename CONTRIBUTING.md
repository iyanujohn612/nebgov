# Contributing to NebGov

Thank you for contributing to NebGov, the permissionless governance framework for Stellar.

## Prerequisites

- **Rust** (stable toolchain) with `wasm32-unknown-unknown` target
- **Stellar CLI** (`stellar`) for contract building and testing
- **Node.js 20+** with **pnpm 9+**
- **Git**

### Install Prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown

# Stellar CLI
cargo install --locked stellar-cli

# Node.js (via nvm)
nvm install 20
npm install -g pnpm@9
```

## Local Setup

```bash
git clone https://github.com/nebgov/nebgov && cd nebgov

# Build all Soroban contracts
stellar contract build

# Install JS dependencies
pnpm install

# Build the SDK
pnpm build:sdk
```

## Running Tests

Run everything CI checks, in one command:

```bash
make check
```

Or the two halves separately: `make test-contracts` (Rust) and `make test-ts lint-ts build-ts` (every `pnpm` workspace — `sdk/`, `app/`, `backend/`, `packages/cli/`, `packages/indexer/`, `tools/simulation/`).

Individual commands, if you only touched one package:

```bash
# Rust contract tests
cargo test --workspace

# SDK unit tests
pnpm test:sdk

# Frontend tests
pnpm test:app

# CLI unit tests (SDK clients are mocked; build the SDK first so its enums resolve)
pnpm build:sdk && pnpm test:cli

# E2E tests (requires running app)
cd app && npx playwright test
```

## Security Scanning

We use [CoinFabrik Scout](https://github.com/CoinFabrik/scout) to automatically scan our Soroban contracts for security vulnerabilities.

### Local Usage

To run Scout locally, you first need to install the `scout-audit` tool:

```bash
cargo install cargo-scout-audit
```

Then, run the scanner from the repository root:

```bash
cargo scout-audit --output-format html
```

This will generate an `audit_report.html` file with the results.

### Interpreting Results

- **Critical/High**: These findings will fail the CI build and **must** be addressed before merging.
- **Medium/Low/Info**: These findings do not fail the CI but should be reviewed and fixed if applicable.

### Suppressing False Positives

If a finding is confirmed as a false positive, it can be suppressed in `.scout.toml` at the repository root. Each suppression must include a justification:

```toml
[[suppressions]]
detector = "detector_name"
reason = "Suppressed: [reason] — [date] — [author]"
```

## Project Structure

| Directory                    | Description                             |
| ---------------------------- | --------------------------------------- |
| `contracts/governor`         | Core governance contract (Rust/Soroban) |
| `contracts/timelock`         | Delayed execution controller            |
| `contracts/token-votes`      | Voting power with checkpointing, including split delegation ([docs](./docs/split-delegation.md)) |
| `contracts/governor-factory` | Permissionless governor deployer        |
| `contracts/treasury`         | Multi-sig treasury                      |
| `contracts/treasury-strategies` | Governance-controlled yield strategy allocation ([docs](./docs/treasury-strategies.md)) |
| `contracts/proposal-bonds`   | Refundable proposer bonds with slashing ([docs](./docs/proposal-bonds.md)) |
| `contracts/optimistic-governor` | Fast-track proposals with challenge window ([docs](./docs/optimistic-governance.md)) |
| `contracts/conviction-voting` | Self-executing voting based on conviction ([docs](./docs/conviction-voting.md)) |
| `contracts/signal-anchor`    | Off-chain signaling with on-chain anchoring ([docs](./docs/signal-anchor.md)) |
| `contracts/treasury-streams` | Recurring treasury payments, governance-created ([docs](./docs/treasury-streams.md)) |
| `sdk/`                       | TypeScript SDK (`@nebgov/sdk`)          |
| `app/`                       | Next.js governance dashboard            |
| `docs/`                      | Architecture docs and ADRs              |

## Smart Contract Contributions

NebGov's contracts are written in Rust targeting the Soroban VM. Contract changes carry higher risk than frontend or SDK changes because they affect immutable on-chain state. Follow the requirements below for any PR that touches `contracts/`.

### Toolchain Setup

```bash
# Rust stable toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown

# Soroban / Stellar CLI (pin to the version used by CI)
cargo install --locked stellar-cli

# Verify installation
stellar --version
cargo build --target wasm32-unknown-unknown --release
```

### Building Contracts

```bash
# Build all contracts to WASM
stellar contract build

# Build a single contract
cd contracts/governor && cargo build --release --target wasm32-unknown-unknown

# Optimized release build (used for deployment)
stellar contract build --profile release
```

### Running Tests

All three test layers are required before submitting a contract PR:

```bash
# Unit and integration tests for all contracts
cargo test --workspace

# Test a single contract in isolation
cargo test -p sorogov-governor

# Run a specific test by name
cargo test -p sorogov-liquidity test_swap_reverse_direction_finds_pool -- --nocapture
```

Every new function must have at least one unit test. Cross-contract interactions require an integration test in the contract's `integration_tests.rs` or `tests.rs` module. Tests that involve the full governance lifecycle (propose → vote → queue → execute) should live in integration test files.

### Test Snapshots

Soroban's test harness writes a JSON snapshot of every test's ledger state to `contracts/<name>/test_snapshots/` on each `cargo test` run. These files are generated output: they are ignored by `.gitignore`, not tracked in the repository, and not read by CI.

- **When a snapshot changes:** do nothing. Leave the regenerated files out of your commit; `git status` will not show them.
- **Do not force-add them** (`git add -f`) to "pin" behaviour. Assert the state you care about in the test itself.
- **If an old checkout still tracks some:** run `git rm -r --cached contracts/*/test_snapshots` before committing.

Captured command output (for example `cargo test > test_output.txt`) likewise stays out of the tree. `*.txt` under `contracts/*/src/` is ignored.

### Code Style for Contracts

**Authorization**: Every function that mutates state on behalf of a caller must call `caller.require_auth()` as the very first statement. Never rely on the caller parameter alone to infer identity.

**Events**: Emit a structured event for every meaningful state transition. Use the helpers in `events.rs` and follow the existing naming pattern (`emit_<action>`).

**Errors**: Use the contract's typed error enum (e.g., `LiquidityError`, `GovernorError`) and `env.panic_with_error(...)`. Never use bare `panic!()` strings for user-facing error paths — reserve those for programming errors caught in development.

**Arithmetic**: Use `checked_add` / `checked_mul` (or the contract's own helpers) wherever overflow is possible. Document the invariant if you deliberately use unchecked arithmetic.

**No `unsafe` code**: The `#![no_std]` environment does not allow `unsafe`. Any dependency that requires it must be justified in the PR description.

### Security Checklist for Contract PRs

Before requesting review, verify:

- [ ] `require_auth()` called for every privileged entry point
- [ ] State mutations happen before external calls (checks-effects-interactions)
- [ ] All arithmetic uses overflow-safe operations
- [ ] Events emitted for every state change
- [ ] No new persistent storage keys that can conflict with existing keys
- [ ] Tests cover the happy path, at least one error path, and any edge cases mentioned in the issue
- [ ] `cargo scout-audit` passes locally (see [Security Scanning](#security-scanning) below)

### Security Review Process

All contract changes require a security sign-off from a maintainer before merge. Add the label `needs-security-review` to your PR. Changes to access control logic, arithmetic, or storage layout receive mandatory dual review.

### Testing on Futurenet Before Mainnet

Deploy to Futurenet and run a smoke test before opening a mainnet-targeted PR:

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/sorogov_governor.wasm \
  --network futurenet \
  --source <SECRET_KEY>
```

Record the deployed address and verify the initialization flow end-to-end using `stellar contract invoke`.

### Architecture Docs

- [docs/architecture.md](./docs/architecture.md) — contract interaction diagram and storage layout
- [docs/security.md](./docs/security.md) — known threat model and mitigations
- [tools/simulation/README.md](./tools/simulation/README.md) — governance simulation testing framework and how to add scenarios

---

## How to Contribute

### 1. Find an issue

Browse [open issues](https://github.com/nebgov/nebgov/issues). Each issue is tagged with:

- `complexity: trivial` - small, well-scoped change
- `complexity: medium` - moderate implementation work
- `complexity: high` - significant feature or architectural change

When opening a new issue, please choose one of the repository's issue templates so your report includes all required details.

Issues tagged `good first issue` are recommended for first-time contributors.

## Branch Naming

- `feat/<description>` for features
- `fix/<description>` for bug fixes
- `docs/<description>` for documentation

Example: `feat/governor-parameter-tuning`, `fix/quorum-calculation-edge-case`, `docs/contract-architecture`

## Commit Messages

Use imperative mood with conventional prefixes:

- `feat: add vote delegation`
- `fix: correct quorum calculation`
- `docs: update architecture diagram`
- `test: add governor edge cases`
- `chore: update dependencies`

## Pull Request Process

1. Fork the repo and create a branch from `main`
2. Make your changes with tests for new features
3. Ensure all CI checks pass: `make check` (aggregates the Rust contract suite and every TypeScript workspace — see [`test-ts` / `lint-ts` / `build-ts`](./Makefile))
4. Update docs if you changed any public API
5. **Update the changelog**: Add your changes to the `[Unreleased]` section of `CHANGELOG.md` under the appropriate subsection (`Added`, `Changed`, `Fixed`, `Security`, etc.). Breaking changes must be noted with a `**Breaking:**` prefix. Do not create a new version section — maintainers handle that at release time.
6. Open a PR referencing the issue: `Closes #<number>`
7. Wait for maintainer review. Note that reviewers are automatically assigned based on our [CODEOWNERS](.github/CODEOWNERS) configuration. For more details on our branch protection rules, see [Branch Protection](docs/contributing/branch-protection.md).

## Security Issues

If you find a security vulnerability, do not open a public issue. Follow the private reporting process in [SECURITY.md](./SECURITY.md) instead.

## Code Standards

### Rust (contracts)

- Format with `cargo fmt`
- No `unsafe` code
- All public functions must have doc comments
- Tests live in `#[cfg(test)]` modules

### TypeScript (SDK + frontend)

- Strict TypeScript, no `any` types
- Run `pnpm lint` before pushing
- Use named exports

## Issue Scope

Each issue is scoped to be completable in **under one week** by a single contributor. If you find an issue is larger than expected, comment on it so it can be split.

## Questions?

Open a discussion on GitHub or comment on the relevant issue.
