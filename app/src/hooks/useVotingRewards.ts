"use client";

import { useCallback, useEffect, useState } from "react";
import { VotingRewardsClient, type VotingRewardsEpoch } from "@nebgov/sdk";
import { backendFetch } from "../lib/backend";
import { readGovernorConfig } from "../lib/nebgov-env";

/** One epoch as served by `GET /voting-rewards/epochs`. */
export interface RewardEpochSummary {
  epochId: bigint;
  startLedger: number;
  endLedger: number;
  merkleRoot: string | null;
  totalRewardAmount: bigint;
  publishedAt: string | null;
}

/** One of the connected wallet's rewards, with the proof needed to claim it. */
export interface ClaimableRewardRow {
  epochId: bigint;
  amount: bigint;
  merkleProof: string[];
  claimed: boolean;
}

export interface LeaderboardRow {
  address: string;
  amount: bigint;
  claimed: boolean;
}

interface EpochResponseRow {
  epoch_id: string;
  start_ledger: number;
  end_ledger: number;
  merkle_root: string | null;
  total_reward_amount: string;
  published_at: string | null;
}

function toEpochSummary(row: EpochResponseRow): RewardEpochSummary {
  return {
    epochId: BigInt(row.epoch_id),
    startLedger: row.start_ledger,
    endLedger: row.end_ledger,
    merkleRoot: row.merkle_root,
    totalRewardAmount: BigInt(row.total_reward_amount),
    publishedAt: row.published_at,
  };
}

export function buildVotingRewardsClient(): VotingRewardsClient | null {
  const config = readGovernorConfig();
  if (!config || !config.votingRewardsAddress) return null;
  return new VotingRewardsClient(config);
}

/** The epochs the backend has computed, newest first. */
export function useRewardEpochs(limit = 20, offset = 0) {
  const [epochs, setEpochs] = useState<RewardEpochSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refetchToken, setRefetchToken] = useState(0);
  const [total, setTotal] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    backendFetch<{ data: EpochResponseRow[]; total: number }>(
      `/voting-rewards/epochs?limit=${limit}&offset=${offset}`,
    )
      .then((res) => {
        if (!cancelled) {
          setEpochs(res.data.map(toEpochSummary));
          setTotal(res.total);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [limit, offset, refetchToken]);

  return { epochs, total, loading, error, refetch: useCallback(() => setRefetchToken((t) => t + 1), []) };
}

/**
 * The epoch currently accepting votes, read straight from the contract.
 *
 * Deliberately on-chain rather than from the backend: the live epoch has no
 * database row until it closes and its eligibility is computed, and the
 * page's progress bar needs its ledger window before then.
 */
export function useCurrentRewardEpoch() {
  const [epoch, setEpoch] = useState<VotingRewardsEpoch | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const client = buildVotingRewardsClient();
    if (!client) {
      setError("Voting rewards are not configured for this deployment.");
      setLoading(false);
      return;
    }

    client
      .getCurrentEpochId()
      .then((id) => client.getEpoch(id))
      .then((result) => {
        if (!cancelled) setEpoch(result);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return { epoch, loading, error };
}

/** Everything `address` has earned, with claimed/unclaimed status and proofs. */
export function useClaimableRewards(address: string | null) {
  const [rewards, setRewards] = useState<ClaimableRewardRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refetchToken, setRefetchToken] = useState(0);

  useEffect(() => {
    if (!address) {
      setRewards([]);
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    backendFetch<{
      data: { epoch_id: string; amount: string; merkle_proof: string[]; claimed: boolean }[];
    }>(`/voting-rewards/claims/${address}`)
      .then((res) => {
        if (cancelled) return;
        setRewards(
          res.data.map((row) => ({
            epochId: BigInt(row.epoch_id),
            amount: BigInt(row.amount),
            merkleProof: row.merkle_proof,
            claimed: row.claimed,
          })),
        );
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [address, refetchToken]);

  const unclaimed = rewards.filter((reward) => !reward.claimed);
  const totalUnclaimed = unclaimed.reduce((acc, reward) => acc + reward.amount, 0n);
  const totalEarned = rewards.reduce((acc, reward) => acc + reward.amount, 0n);

  return {
    rewards,
    unclaimed,
    totalUnclaimed,
    totalEarned,
    loading,
    error,
    refetch: useCallback(() => setRefetchToken((t) => t + 1), []),
  };
}

/**
 * Top earners for one epoch.
 *
 * Returns `refetch` so callers can refresh the claimed/unclaimed flags
 * immediately after successful reward claims.
 */
export function useEpochLeaderboard(epochId: bigint | null, limit = 10, offset = 0) {
  const [rows, setRows] = useState<LeaderboardRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refetchToken, setRefetchToken] = useState(0);
  const [total, setTotal] = useState(0);

  useEffect(() => {
    if (epochId === null) {
      setRows([]);
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    backendFetch<{
      data: { claimant_address: string; amount: string; claimed: boolean }[];
      total: number;
    }>(
      `/voting-rewards/epochs/${epochId}/leaderboard?limit=${limit}&offset=${offset}`,
    )
      .then((res) => {
        if (cancelled) return;
        setRows(
          res.data.map((row) => ({
            address: row.claimant_address,
            amount: BigInt(row.amount),
            claimed: row.claimed,
          })),
        );
        setTotal(res.total);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [epochId, limit, offset, refetchToken]);

  return {
    rows,
    total,
    loading,
    error,
    refetch: useCallback(() => setRefetchToken((t) => t + 1), []),
  };
}
