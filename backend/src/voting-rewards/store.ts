import pool from "../db/pool";

export interface StoredEpoch {
  epochId: bigint;
  startLedger: number;
  endLedger: number;
  merkleRoot: string | null;
  totalRewardAmount: bigint;
  publishedAt: Date | null;
  /** Governance proposal carrying this epoch's `publish_epoch_root`, if that route was taken. */
  publishProposalId: bigint | null;
}

export interface StoredClaim {
  epochId: bigint;
  claimantAddress: string;
  amount: bigint;
  merkleProof: string[];
  claimed: boolean;
}

interface EpochRow {
  epoch_id: string;
  start_ledger: number;
  end_ledger: number;
  merkle_root: string | null;
  total_reward_amount: string;
  published_at: Date | null;
  publish_proposal_id: string | null;
}

interface ClaimRow {
  epoch_id: string;
  claimant_address: string;
  amount: string;
  merkle_proof: string[];
  claimed: boolean;
}

function toEpoch(row: EpochRow): StoredEpoch {
  return {
    epochId: BigInt(row.epoch_id),
    startLedger: row.start_ledger,
    endLedger: row.end_ledger,
    merkleRoot: row.merkle_root,
    totalRewardAmount: BigInt(row.total_reward_amount),
    publishedAt: row.published_at,
    publishProposalId:
      row.publish_proposal_id === null ? null : BigInt(row.publish_proposal_id),
  };
}

function toClaim(row: ClaimRow): StoredClaim {
  return {
    epochId: BigInt(row.epoch_id),
    claimantAddress: row.claimant_address,
    amount: BigInt(row.amount),
    merkleProof: row.merkle_proof,
    claimed: row.claimed,
  };
}

/**
 * Persist a computed epoch and the per-claimant proofs derived from the very
 * same tree, in one transaction.
 *
 * Recomputing an epoch that already has rows replaces them wholesale, which
 * only ever happens before the root reaches the chain — once `published_at`
 * is set the root is immutable on-chain and {@link markEpochPublished} is the
 * only thing that touches the row again.
 */
export async function saveComputedEpoch(
  epoch: Omit<StoredEpoch, "publishedAt" | "publishProposalId">,
  claims: Omit<StoredClaim, "claimed">[],
): Promise<void> {
  const client = await pool.connect();
  try {
    await client.query("BEGIN");
    await client.query(
      `INSERT INTO voting_reward_epochs (epoch_id, start_ledger, end_ledger, merkle_root, total_reward_amount)
       VALUES ($1, $2, $3, $4, $5)
       ON CONFLICT (epoch_id) DO UPDATE SET
         start_ledger = EXCLUDED.start_ledger,
         end_ledger = EXCLUDED.end_ledger,
         merkle_root = EXCLUDED.merkle_root,
         total_reward_amount = EXCLUDED.total_reward_amount`,
      [
        epoch.epochId.toString(),
        epoch.startLedger,
        epoch.endLedger,
        epoch.merkleRoot,
        epoch.totalRewardAmount.toString(),
      ],
    );
    await client.query(`DELETE FROM voting_reward_claims WHERE epoch_id = $1`, [
      epoch.epochId.toString(),
    ]);
    for (const claim of claims) {
      await client.query(
        `INSERT INTO voting_reward_claims (epoch_id, claimant_address, amount, merkle_proof)
         VALUES ($1, $2, $3, $4)`,
        [
          claim.epochId.toString(),
          claim.claimantAddress,
          claim.amount.toString(),
          JSON.stringify(claim.merkleProof),
        ],
      );
    }
    await client.query("COMMIT");
  } catch (err) {
    await client.query("ROLLBACK");
    throw err;
  } finally {
    client.release();
  }
}

export async function recordPublishProposal(
  epochId: bigint,
  proposalId: bigint,
): Promise<void> {
  await pool.query(
    `UPDATE voting_reward_epochs SET publish_proposal_id = $2 WHERE epoch_id = $1`,
    [epochId.toString(), proposalId.toString()],
  );
}

/** The newest epoch already confirmed published on-chain, or `null` if none is. */
export async function getHighestPublishedEpochId(): Promise<bigint | null> {
  const result = await pool.query<{ epoch_id: string }>(
    `SELECT epoch_id FROM voting_reward_epochs
       WHERE published_at IS NOT NULL ORDER BY epoch_id DESC LIMIT 1`,
  );
  return result.rows[0] ? BigInt(result.rows[0].epoch_id) : null;
}

export async function markEpochPublished(epochId: bigint): Promise<void> {
  await pool.query(
    `UPDATE voting_reward_epochs SET published_at = NOW() WHERE epoch_id = $1 AND published_at IS NULL`,
    [epochId.toString()],
  );
}

export async function markClaimed(epochId: bigint, claimant: string): Promise<void> {
  await pool.query(
    `UPDATE voting_reward_claims SET claimed = TRUE WHERE epoch_id = $1 AND claimant_address = $2`,
    [epochId.toString(), claimant],
  );
}

export async function getEpoch(epochId: bigint): Promise<StoredEpoch | null> {
  const result = await pool.query<EpochRow>(
    `SELECT epoch_id, start_ledger, end_ledger, merkle_root, total_reward_amount, published_at, publish_proposal_id
       FROM voting_reward_epochs WHERE epoch_id = $1`,
    [epochId.toString()],
  );
  return result.rows[0] ? toEpoch(result.rows[0]) : null;
}

export async function listEpochs(
  limit: number,
  offset: number,
): Promise<{ rows: StoredEpoch[]; total: number }> {
  const result = await pool.query<EpochRow>(
    `SELECT epoch_id, start_ledger, end_ledger, merkle_root, total_reward_amount, published_at, publish_proposal_id
       FROM voting_reward_epochs ORDER BY epoch_id DESC LIMIT $1 OFFSET $2`,
    [limit, offset],
  );
  const count = await pool.query<{ total: string }>(
    `SELECT COUNT(*) AS total FROM voting_reward_epochs`,
  );
  return { rows: result.rows.map(toEpoch), total: Number(count.rows[0]?.total ?? 0) };
}

/** Every epoch `address` earned something in, newest first. */
export async function getClaimsForAddress(address: string): Promise<StoredClaim[]> {
  const result = await pool.query<ClaimRow>(
    `SELECT epoch_id, claimant_address, amount, merkle_proof, claimed
       FROM voting_reward_claims WHERE claimant_address = $1 ORDER BY epoch_id DESC`,
    [address],
  );
  return result.rows.map(toClaim);
}

export async function getEpochLeaderboard(
  epochId: bigint,
  limit: number,
  offset: number,
): Promise<{ rows: StoredClaim[]; total: number }> {
  const result = await pool.query<ClaimRow>(
    `SELECT epoch_id, claimant_address, amount, merkle_proof, claimed
       FROM voting_reward_claims WHERE epoch_id = $1
       ORDER BY amount DESC, claimant_address ASC LIMIT $2 OFFSET $3`,
    [epochId.toString(), limit, offset],
  );
  const count = await pool.query<{ total: string }>(
    `SELECT COUNT(*) AS total FROM voting_reward_claims WHERE epoch_id = $1`,
    [epochId.toString()],
  );
  return { rows: result.rows.map(toClaim), total: Number(count.rows[0]?.total ?? 0) };
}

/** Epochs computed but not yet confirmed published on-chain, oldest first. */
export async function listUnpublishedEpochs(): Promise<StoredEpoch[]> {
  const result = await pool.query<EpochRow>(
    `SELECT epoch_id, start_ledger, end_ledger, merkle_root, total_reward_amount, published_at, publish_proposal_id
       FROM voting_reward_epochs WHERE published_at IS NULL ORDER BY epoch_id ASC`,
  );
  return result.rows.map(toEpoch);
}
