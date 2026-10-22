import { ProposerLeaderboard } from "../../components/ProposerLeaderboard";

export default function LeaderboardPage() {
  return (
    <div className="max-w-4xl mx-auto px-4 py-8">
      <div className="mb-8">
        <h1 className="text-3xl font-bold text-gray-900 dark:text-white">Proposer leaderboard</h1>
        <p className="mt-2 text-gray-600 dark:text-gray-400">
          Proposers ranked by reputation score. A higher score lowers the threshold needed to create a proposal.
        </p>
      </div>
      <ProposerLeaderboard />
    </div>
  );
}
