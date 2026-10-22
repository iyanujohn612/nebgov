/**
 * @jest-environment jsdom
 */

import React from "react";
import { render, screen } from "@testing-library/react";
import { NavBar } from "../NavBar";

jest.mock("next/navigation", () => ({
  usePathname: () => "/",
}));

jest.mock("../../lib/wallet-context", () => ({
  useWallet: () => ({
    address: null,
    publicKey: null,
    isConnected: false,
    isConnecting: false,
    connect: jest.fn(),
    disconnect: jest.fn(),
  }),
}));

jest.mock("../../hooks/useTheme", () => ({
  useTheme: () => ({
    theme: "light",
    setTheme: jest.fn(),
  }),
}));

jest.mock("next-intl", () => ({
  useTranslations: () => () => "",
}));

jest.mock("react-hot-toast", () => ({
  __esModule: true,
  default: {
    success: jest.fn(),
    error: jest.fn(),
  },
}));

jest.mock("../../lib/governance-notifications", () => ({
  loadNotificationHistory: () => [],
}));

jest.mock("../../lib/use-governance-balance", () => ({
  useGovernanceBalance: () => ({
    loading: false,
    baseVotes: null,
    votingPower: null,
    delegatee: null,
  }),
}));

describe("NavBar", () => {
  it("renders Bonds and Security navigation links", () => {
    render(<NavBar />);

    expect(screen.getByRole("link", { name: "Bonds" }).getAttribute("href")).toBe("/bonds");
    expect(screen.getByRole("link", { name: "Security" }).getAttribute("href")).toBe("/security");
  });

  it("renders a Guardian navigation link", () => {
    render(<NavBar />);

    expect(screen.getByRole("link", { name: "Guardian" }).getAttribute("href")).toBe("/guardian");
  });

  it("keeps existing core links in the same order and href", () => {
    const { container } = render(<NavBar />);
    const nav = container.querySelector('[aria-label="Main"]');
    const links = Array.from(nav?.querySelectorAll("a") ?? []);
    const names = links.map((link) => link.textContent?.trim() ?? "");
    const hrefs = links.map((link) => link.getAttribute("href"));

    expect(names).toEqual([
      "Proposals",
      "Conviction",
      "Optimistic",
      "Governors",
      "Notifications",
      "Signals",
      "Rewards",
      "Bonds",
      "Guardian",
      "Drafts",
      "Delegates",
      "Analytics",
      "Leaderboard",
      "Treasury",
      "Strategies",
      "Governance Tuning",
      "Security",
      "Settings",
    ]);

    expect(hrefs).toEqual([
      "/",
      "/conviction",
      "/optimistic",
      "/governors",
      "/notifications",
      "/signals",
      "/rewards",
      "/bonds",
      "/guardian",
      "/drafts",
      "/delegates",
      "/analytics",
      "/leaderboard",
      "/treasury",
      "/treasury/strategies",
      "/governance-tuning",
      "/security",
      "/settings",
    ]);
  });

  it("does not render a Vote Escrow navigation link", () => {
    render(<NavBar />);
    expect(screen.queryByRole("link", { name: "Vote Escrow" })).toBeNull();
  });
});
