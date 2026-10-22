"use client";

import { useState, useEffect, useCallback, useMemo } from "react";
import Link from "next/link";
import { ProposerLeaderboardEntry } from "@nebgov/sdk";
import { AddressDisplay } from "./AddressDisplay";

type SortKey = "rank" | "reputationScore";

interface ProposerLeaderboardProps {
  className?: string;
  limit?: number;
}

/**
 * Sortable table of the top proposers by reputation score (Issue #771).
 * Backed by the indexer's `GET /reputation/leaderboard` — ranking is
 * computed off-chain from indexed `ReputationUpdated` events rather than
 * on-chain, since sorting a growing address list is cheap off-chain and
 * unnecessarily expensive (gas and contract size) on it.
 */
export function ProposerLeaderboard({ className = "", limit = 50 }: ProposerLeaderboardProps) {
  const [entries, setEntries] = useState<ProposerLeaderboardEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>("rank");
  const [sortAsc, setSortAsc] = useState(true);

  const fetchLeaderboard = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const indexerUrl = process.env.NEXT_PUBLIC_INDEXER_URL;
      if (!indexerUrl) {
        throw new Error("Missing NEXT_PUBLIC_INDEXER_URL");
      }

      const resp = await fetch(`${indexerUrl}/reputation/leaderboard?limit=${limit}`, {
        cache: "no-store",
      });
      if (!resp.ok) throw new Error("Failed to load leaderboard");
      const json = await resp.json();
      const rows = (json.leaderboard ?? []) as Array<{
        rank: number;
        address: string;
        reputation_score: number;
        last_updated_ledger: number | null;
      }>;
      setEntries(
        rows.map((r) => ({
          rank: r.rank,
          proposer: r.address,
          reputationScore: r.reputation_score,
          lastUpdatedLedger: r.last_updated_ledger,
        })),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load leaderboard");
    } finally {
      setLoading(false);
    }
  }, [limit]);

  useEffect(() => {
    fetchLeaderboard();
  }, [fetchLeaderboard]);

  function toggleSort(key: SortKey) {
    if (key === sortKey) {
      setSortAsc((a) => !a);
    } else {
      setSortKey(key);
      setSortAsc(key === "rank");
    }
  }

  const sorted = useMemo(() => {
    const copy = [...entries];
    copy.sort((a, b) => (sortAsc ? a[sortKey] - b[sortKey] : b[sortKey] - a[sortKey]));
    return copy;
  }, [entries, sortKey, sortAsc]);

  const columns: { key: SortKey; label: string }[] = [
    { key: "rank", label: "Rank" },
    { key: "reputationScore", label: "Score" },
  ];

  if (loading) {
    return (
      <div className={`space-y-2 ${className}`}>
        {Array.from({ length: 5 }).map((_, i) => (
          <div key={i} className="h-8 bg-gray-200 dark:bg-gray-700 animate-pulse rounded" />
        ))}
      </div>
    );
  }

  if (error) {
    return <p className="text-sm text-red-600 dark:text-red-400">{error}</p>;
  }

  if (entries.length === 0) {
    return <p className="text-sm text-gray-500 dark:text-gray-400">No ranked proposers yet.</p>;
  }

  return (
    <div className={`overflow-x-auto ${className}`}>
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-gray-200 dark:border-gray-700 text-left">
            <th className="pb-2 pr-4 font-medium text-gray-500 dark:text-gray-400">
              Proposer
            </th>
            {columns.map((col) => (
              <th
                key={col.key}
                aria-sort={
                  sortKey === col.key ? (sortAsc ? "ascending" : "descending") : "none"
                }
                className="pb-2 px-4 font-medium text-gray-500 dark:text-gray-400"
              >
                <button
                  type="button"
                  onClick={() => toggleSort(col.key)}
                  className="inline-flex items-center gap-1 cursor-pointer select-none rounded hover:text-gray-900 dark:hover:text-white focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500"
                >
                  {col.label}
                  {sortKey === col.key && (
                    <span aria-hidden="true">{sortAsc ? "▲" : "▼"}</span>
                  )}
                </button>
              </th>
            ))}
            <th className="pb-2 px-4 font-medium text-gray-500 dark:text-gray-400">
              Last Updated
            </th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((entry) => (
            <tr
              key={entry.proposer}
              className="border-b border-gray-100 dark:border-gray-800 last:border-0"
            >
              <td className="py-2 pr-4">
                <Link
                  href={`/profile/${entry.proposer}`}
                  className="font-mono text-indigo-600 dark:text-indigo-400 hover:text-indigo-700 dark:hover:text-indigo-300"
                >
                  <AddressDisplay address={entry.proposer} />
                </Link>
              </td>
              <td className="py-2 px-4">{entry.rank}</td>
              <td className="py-2 px-4">{entry.reputationScore}</td>
              <td className="py-2 px-4">
                {entry.lastUpdatedLedger !== null ? `#${entry.lastUpdatedLedger}` : "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
