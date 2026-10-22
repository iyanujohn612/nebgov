import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ProposerLeaderboard } from "@/components/ProposerLeaderboard";

jest.mock("@/components/AddressDisplay", () => ({
  AddressDisplay: ({ address }: { address: string }) => <span>{address}</span>,
}));

describe("ProposerLeaderboard sortable headers", () => {
  const originalFetch = global.fetch;
  const originalEnv = process.env.NEXT_PUBLIC_INDEXER_URL;

  beforeEach(() => {
    process.env.NEXT_PUBLIC_INDEXER_URL = "http://indexer.test";
    global.fetch = jest.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        leaderboard: [
          { rank: 1, address: "GAAAA", reputation_score: 10, last_updated_ledger: 5 },
          { rank: 2, address: "GBBBB", reputation_score: 40, last_updated_ledger: 9 },
        ],
      }),
    }) as unknown as typeof fetch;
  });

  afterEach(() => {
    global.fetch = originalFetch;
    process.env.NEXT_PUBLIC_INDEXER_URL = originalEnv;
  });

  it("exposes sortable headers as buttons with aria-sort reflecting the current sort", async () => {
    render(<ProposerLeaderboard />);

    await waitFor(() => expect(screen.getByRole("table")).toBeInTheDocument());

    // Default sort is by rank, ascending.
    expect(screen.getByRole("columnheader", { name: /Rank/ })).toHaveAttribute(
      "aria-sort",
      "ascending",
    );
    expect(screen.getByRole("columnheader", { name: /Score/ })).toHaveAttribute(
      "aria-sort",
      "none",
    );

    const scoreButton = screen.getByRole("button", { name: "Score" });
    fireEvent.click(scoreButton);

    // First activation of a new column sorts descending.
    expect(screen.getByRole("columnheader", { name: /Score/ })).toHaveAttribute(
      "aria-sort",
      "descending",
    );
    expect(screen.getByRole("columnheader", { name: /Rank/ })).toHaveAttribute(
      "aria-sort",
      "none",
    );

    fireEvent.click(scoreButton);
    expect(screen.getByRole("columnheader", { name: /Score/ })).toHaveAttribute(
      "aria-sort",
      "ascending",
    );
  });

  it("keeps the sort buttons reachable by keyboard", async () => {
    render(<ProposerLeaderboard />);

    await waitFor(() => expect(screen.getByRole("table")).toBeInTheDocument());

    const buttons = screen.getAllByRole("button", { name: /Rank|Score/ });
    expect(buttons).toHaveLength(2);
    buttons.forEach((button) => expect(button.tagName).toBe("BUTTON"));
  });
});
