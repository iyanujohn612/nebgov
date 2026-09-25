import { Response, Router } from "express";
import { z } from "zod";
import { validate } from "../middleware/validate";
import { logger } from "../logger";
import { stellarAddressSchema } from "../validation/stellar";
import { refreshClaimStatuses } from "../voting-rewards/claim-status";
import {
  getClaimsForAddress,
  getEpoch,
  getEpochLeaderboard,
  listEpochs,
  type StoredClaim,
  type StoredEpoch,
} from "../voting-rewards/store";

const router = Router();

function serializeEpoch(epoch: StoredEpoch) {
  return {
    epoch_id: epoch.epochId.toString(),
    start_ledger: epoch.startLedger,
    end_ledger: epoch.endLedger,
    merkle_root: epoch.merkleRoot,
    total_reward_amount: epoch.totalRewardAmount.toString(),
    published_at: epoch.publishedAt,
    publish_proposal_id:
      epoch.publishProposalId === null ? null : epoch.publishProposalId.toString(),
  };
}

function serializeClaim(claim: StoredClaim) {
  return {
    epoch_id: claim.epochId.toString(),
    claimant_address: claim.claimantAddress,
    amount: claim.amount.toString(),
    merkle_proof: claim.merkleProof,
    claimed: claim.claimed,
  };
}

const listQuerySchema = z.object({
  limit: z.coerce.number().int().min(1).max(100).optional().default(20),
  offset: z.coerce.number().int().min(0).optional().default(0),
});

const epochParamSchema = z.object({
  epochId: z.coerce.bigint().nonnegative(),
});

// Validate the full StrKey checksum before querying with a claimant address.
const addressParamSchema = z.object({
  address: stellarAddressSchema,
});

function errorResponse(error: unknown): { error: string; code: string } {
  const databaseError =
    typeof error === "object" && error !== null && "code" in error && /^[0-9A-Z]{5}$/.test(String(error.code));
  return databaseError
    ? { error: "Unable to read voting rewards", code: "VOTING_REWARDS_DATABASE_ERROR" }
    : { error: "Unable to process voting rewards", code: "VOTING_REWARDS_INTERNAL_ERROR" };
}

// GET /voting-rewards/epochs?limit=20
router.get(
  "/epochs",
  validate({ query: listQuerySchema }),
  async (req, res: Response): Promise<void> => {
    const { limit, offset } = req.query as unknown as z.infer<typeof listQuerySchema>;
    try {
      const result = await listEpochs(limit, offset);
      res.json({ data: result.rows.map(serializeEpoch), total: result.total });
    } catch (error) {
      logger.error({ err: error }, "Error in GET /voting-rewards/epochs");
      res.status(500).json(errorResponse(error));
    }
  },
);

// GET /voting-rewards/epochs/:epochId
router.get(
  "/epochs/:epochId",
  validate({ params: epochParamSchema }),
  async (req, res: Response): Promise<void> => {
    const { epochId } = req.params as unknown as z.infer<typeof epochParamSchema>;
    try {
      const epoch = await getEpoch(epochId);
      if (!epoch) {
        res.status(404).json({ error: "Epoch not found", code: "EPOCH_NOT_FOUND" });
        return;
      }
      res.json(serializeEpoch(epoch));
    } catch (error) {
      logger.error({ err: error }, "Error in GET /voting-rewards/epochs/:epochId");
      res.status(500).json(errorResponse(error));
    }
  },
);

// GET /voting-rewards/epochs/:epochId/leaderboard?limit=20
router.get(
  "/epochs/:epochId/leaderboard",
  validate({ params: epochParamSchema, query: listQuerySchema }),
  async (req, res: Response): Promise<void> => {
    const { epochId } = req.params as unknown as z.infer<typeof epochParamSchema>;
    const { limit, offset } = req.query as unknown as z.infer<typeof listQuerySchema>;
    try {
      const result = await getEpochLeaderboard(epochId, limit, offset);
      // The leaderboard is public, so it deliberately omits `merkle_proof` —
      // a proof is only ever useful to its own claimant.
      res.json({
        data: result.rows.map((row) => ({
          claimant_address: row.claimantAddress,
          amount: row.amount.toString(),
          claimed: row.claimed,
        })),
        total: result.total,
      });
    } catch (error) {
      logger.error({ err: error }, "Error in GET /voting-rewards/epochs/:epochId/leaderboard");
      res.status(500).json(errorResponse(error));
    }
  },
);

// GET /voting-rewards/claims/:address — every epoch this address earned in,
// each unclaimed one carrying the ready-to-submit Merkle proof.
router.get(
  "/claims/:address",
  validate({ params: addressParamSchema }),
  async (req, res: Response): Promise<void> => {
    const { address } = req.params as unknown as z.infer<typeof addressParamSchema>;
    try {
      const claims = await refreshClaimStatuses(await getClaimsForAddress(address));
      const unclaimedTotal = claims
        .filter((claim) => !claim.claimed)
        .reduce((acc, claim) => acc + claim.amount, 0n);

      res.json({
        data: claims.map(serializeClaim),
        total_unclaimed: unclaimedTotal.toString(),
      });
    } catch (error) {
      logger.error({ err: error }, "Error in GET /voting-rewards/claims/:address");
      res.status(500).json(errorResponse(error));
    }
  },
);

export default router;
