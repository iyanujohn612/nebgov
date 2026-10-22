"use client";

import Link from "next/link";
import { useState, useEffect, useRef } from "react";
import { usePathname } from "next/navigation";
import { useWallet } from "../lib/wallet-context";
import {
  Menu,
  X,
  ChevronDown,
  Copy,
  User,
  LogOut,
  LayoutDashboard,
  Users,
  BarChart3,
  Wallet2,
  Sun,
  Moon,
  Bell,
  Settings,
  Activity,
  Radio,
  SlidersHorizontal,
  TrendingUp,
  FastForward,
  Gift,
  Lock,
  FileText,
  Shield,
  ShieldCheck,
  Trophy,
} from "lucide-react";
import { useTheme } from "../hooks/useTheme";
import { useTranslations } from "next-intl";
import toast from "react-hot-toast";
import { loadNotificationHistory } from "../lib/governance-notifications";
import { useGovernanceBalance } from "../lib/use-governance-balance";
import { NotificationBell } from "./NotificationBell";

function formatGovernanceAmount(v: bigint): string {
  const n = Number(v);
  if (!Number.isFinite(n)) return `${v.toString()} GOV`;
  if (n >= 10_000) {
    return `${new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 2 }).format(n)} GOV`;
  }
  return `${new Intl.NumberFormat("en").format(n)} GOV`;
}

// Route audit: /health is an API-only endpoint (no user page), so it remains intentionally unlinked.
const NAV_LINKS = [
  { name: "Proposals", href: "/", icon: LayoutDashboard },
  { name: "Conviction", href: "/conviction", icon: Activity },
  { name: "Optimistic", href: "/optimistic", icon: FastForward },
  { name: "Governors", href: "/governors", icon: LayoutDashboard },
  { name: "Notifications", href: "/notifications", icon: Bell },
  { name: "Signals", href: "/signals", icon: Radio },
  { name: "Rewards", href: "/rewards", icon: Gift },
  { name: "Vote Escrow", href: "/vote-escrow", icon: Lock },
  { name: "Bonds", href: "/bonds", icon: Wallet2 },
  { name: "Guardian", href: "/guardian", icon: ShieldCheck },
  { name: "Drafts", href: "/drafts", icon: FileText },
  { name: "Delegates", href: "/delegates", icon: Users },
  { name: "Analytics", href: "/analytics", icon: BarChart3 },
  { name: "Leaderboard", href: "/leaderboard", icon: Trophy },
  { name: "Treasury", href: "/treasury", icon: Wallet2 },
  { name: "Strategies", href: "/treasury/strategies", icon: TrendingUp },
  { name: "Governance Tuning", href: "/governance-tuning", icon: SlidersHorizontal },
  { name: "Security", href: "/security", icon: Shield },
  { name: "Settings", href: "/settings", icon: Settings },
];

const VOTE_ESCROW_ENABLED = Boolean(process.env.NEXT_PUBLIC_VOTE_ESCROW_ADDRESS);

const VISIBLE_NAV_LINKS = NAV_LINKS.filter(
  (link) => link.href !== "/vote-escrow" || VOTE_ESCROW_ENABLED
);

export function NavBar() {
  const t = useTranslations("nav");
  const pathname = usePathname();
  const { address, publicKey, isConnected, isConnecting, connect, disconnect } =
    useWallet();
  const gov = useGovernanceBalance(isConnected ? publicKey : null);

  const [isMenuOpen, setIsMenuOpen] = useState(false);
  const [isWalletMenuOpen, setIsWalletMenuOpen] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  const [unread, setUnread] = useState(0);
  const drawerRef = useRef<HTMLDivElement>(null);
  const { theme, setTheme } = useTheme();

  const toggleTheme = () => setTheme(theme === "dark" ? "light" : "dark");

  useEffect(() => {
    setIsMenuOpen(false);
    setIsWalletMenuOpen(false);
  }, [pathname]);

  useEffect(() => {
    const handleScroll = () => setScrolled(window.scrollY > 10);
    window.addEventListener("scroll", handleScroll, { passive: true });
    return () => window.removeEventListener("scroll", handleScroll);
  }, []);

  useEffect(() => {
    document.body.style.overflow = isMenuOpen ? "hidden" : "";
    return () => {
      document.body.style.overflow = "";
    };
  }, [isMenuOpen]);

  useEffect(() => {
    const compute = () => {
      const rows = loadNotificationHistory();
      setUnread(rows.filter((r) => !r.read).length);
    };
    compute();
    window.addEventListener("nebgov-notify-history", compute);
    return () => window.removeEventListener("nebgov-notify-history", compute);
  }, []);

  useEffect(() => {
    if (!isWalletMenuOpen) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") setIsWalletMenuOpen(false);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [isWalletMenuOpen]);

  const copyAddress = async () => {
    if (!publicKey) return;
    try {
      await navigator.clipboard.writeText(publicKey);
      toast.success("Address copied!", {
        style: { borderRadius: "10px", background: "#1e1b4b", color: "#fff" },
        iconTheme: { primary: "#818cf8", secondary: "#fff" },
      });
    } catch {
      toast.error("Failed to copy address.");
    } finally {
      setIsWalletMenuOpen(false);
    }
  };

  const handleDisconnect = () => {
    disconnect();
    setIsWalletMenuOpen(false);
    setIsMenuOpen(false);
  };

  return (
    <nav
      className={`fixed top-0 left-0 right-0 bg-white dark:bg-gray-900 border-b z-50 h-16 transition-all duration-200 ${
        scrolled ? "shadow-sm border-gray-200 dark:border-gray-800" : "border-gray-100 dark:border-gray-800"
      }`}
    >
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 h-full flex items-center justify-between">
        <div className="flex items-center gap-10">
          <Link
            href="/"
            className="flex items-center gap-2 group"
            aria-label="NebGov home"
          >
            <div className="w-8 h-8 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold transform group-hover:rotate-6 transition-transform select-none">
              N
            </div>
            <span className="text-xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-gray-900 to-gray-600 dark:from-white dark:to-gray-400 mr-4">
              NebGov
            </span>
          </Link>

          <div
            className="hidden md:flex items-center gap-1"
            role="navigation"
            aria-label="Main"
          >
            {VISIBLE_NAV_LINKS.map((link) => {
              const isActive = pathname === link.href;
              return (
                <Link
                  key={link.name}
                  href={link.href}
                  aria-current={isActive ? "page" : undefined}
                  className={`px-4 py-2 rounded-full text-sm font-medium transition-all ${
                    isActive
                      ? "text-indigo-600 bg-indigo-50 dark:bg-indigo-900/20"
                      : "text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white hover:bg-gray-50 dark:hover:bg-gray-800"
                  }`}
                >
                  <span className="inline-flex items-center gap-2">
                    {link.name}
                    {link.href === "/notifications" && unread > 0 && (
                      <span className="min-w-5 h-5 px-1.5 rounded-full bg-indigo-600 text-white text-[11px] leading-5 text-center">
                        {unread > 99 ? "99+" : unread}
                      </span>
                    )}
                  </span>
                </Link>
              );
            })}
          </div>
        </div>

        <div className="flex items-center gap-4">
          <NotificationBell />
          <button
            onClick={toggleTheme}
            aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
            className="p-2 rounded-xl text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors"
          >
            {theme === "dark" ? (
              <Sun className="w-5 h-5" aria-hidden />
            ) : (
              <Moon className="w-5 h-5" aria-hidden />
            )}
          </button>
          <select
            aria-label="Select language"
            className="p-2 rounded-xl text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors text-xs font-medium bg-transparent border-none cursor-pointer"
            defaultValue="en"
            onChange={(e) => {
              const selectedLocale = e.target.value;
              const cookies = document.cookie.split(";").reduce((acc, cookie) => {
                const [key, value] = cookie.trim().split("=");
                if (key !== "locale") acc.push(`${key}=${value}`);
                return acc;
              }, [] as string[]);
              document.cookie = [...cookies, `locale=${selectedLocale}`].join("; ") + "; path=/";
              window.location.reload();
            }}
          >
            <option value="en">EN</option>
            <option value="es" disabled>ES</option>
          </select>
          <div className="hidden md:block relative">
            {isConnected ? (
              <div className="relative" ref={drawerRef}>
                <button
                  onClick={() => setIsWalletMenuOpen((v) => !v)}
                  aria-expanded={isWalletMenuOpen}
                  aria-haspopup="menu"
                  className="flex items-center gap-2 pl-3 pr-2 py-1.5 rounded-full border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 hover:border-gray-300 dark:hover:border-gray-600 hover:shadow-sm transition-all text-sm font-medium text-gray-700 dark:text-gray-200"
                >
                  <div
                    className="w-6 h-6 rounded-full bg-gradient-to-br from-indigo-500 to-purple-500 flex-shrink-0"
                    aria-hidden
                  />
                  <span className="max-w-[9rem] truncate">{address}</span>
                  {gov.loading ? (
                    <span className="ml-1 inline-flex items-center">
                      <span className="h-4 w-14 rounded-full bg-gray-200 dark:bg-gray-700 animate-pulse" />
                    </span>
                  ) : gov.baseVotes !== null && gov.votingPower !== null ? (
                    <span
                      className="ml-1 px-2 py-0.5 rounded-full bg-indigo-50 dark:bg-indigo-900/20 text-indigo-700 dark:text-indigo-200 text-xs font-semibold"
                      title={[
                        `Raw: ${formatGovernanceAmount(gov.baseVotes)}`,
                        `Voting power: ${formatGovernanceAmount(gov.votingPower)}`,
                        `Delegatee: ${gov.delegatee ?? "—"}`,
                      ].join("\n")}
                    >
                      {formatGovernanceAmount(gov.baseVotes)}
                    </span>
                  ) : null}
                  <ChevronDown
                    className={`w-4 h-4 text-gray-400 transition-transform duration-200 ${
                      isWalletMenuOpen ? "rotate-180" : ""
                    }`}
                    aria-hidden
                  />
                </button>

                {isWalletMenuOpen && (
                  <>
                    <div
                      className="fixed inset-0 z-[199] pointer-events-auto"
                      onClick={() => setIsWalletMenuOpen(false)}
                      aria-hidden
                    />

                    <div
                      role="menu"
                      className="absolute right-0 mt-2 w-56 bg-white rounded-2xl shadow-xl border border-gray-100 py-2 z-[200] overflow-hidden"
                    >
                      <div className="px-4 py-3 border-b border-gray-50 mb-1">
                        <p className="text-xs text-gray-400 font-medium uppercase tracking-wider mb-1">
                          Active Wallet
                        </p>
                        <p
                          className="text-sm font-mono text-gray-600 truncate"
                          title={publicKey ?? ""}
                        >
                          {publicKey}
                        </p>
                      </div>

                      <button
                        role="menuitem"
                        onClick={copyAddress}
                        className="w-full flex items-center gap-3 px-4 py-2.5 text-sm text-gray-600 hover:bg-gray-50 transition-colors"
                      >
                        <Copy className="w-4 h-4" aria-hidden />
                        Copy address
                      </button>

                      <Link
                        role="menuitem"
                        href={publicKey ? `/profile/${publicKey}` : "/"}
                        className="flex items-center gap-3 px-4 py-2.5 text-sm text-gray-600 hover:bg-gray-50 transition-colors"
                      >
                        <User className="w-4 h-4" aria-hidden />
                        My Profile
                      </Link>

                      <div className="h-px bg-gray-100 my-1" aria-hidden />

                      <button
                        role="menuitem"
                        onClick={handleDisconnect}
                        className="w-full flex items-center gap-3 px-4 py-2.5 text-sm text-red-600 hover:bg-red-50 transition-colors"
                      >
                        <LogOut className="w-4 h-4" aria-hidden />
                        Disconnect
                      </button>
                    </div>
                  </>
                )}
              </div>
            ) : (
              <button
                onClick={connect}
                disabled={isConnecting}
                className="text-sm px-6 py-2.5 rounded-full font-semibold transition-all shadow-md shadow-indigo-100 hover:shadow-lg hover:shadow-indigo-200 active:scale-95 bg-indigo-600 text-white hover:bg-indigo-700 disabled:opacity-60 disabled:cursor-not-allowed"
              >
                {isConnecting ? "Connecting…" : "Connect Wallet"}
              </button>
            )}
          </div>

          {/* Mobile hamburger */}
          <button
            onClick={() => setIsMenuOpen(!isMenuOpen)}
            aria-label={isMenuOpen ? "Close menu" : "Open menu"}
            aria-expanded={isMenuOpen}
            className="md:hidden p-2 rounded-xl text-gray-600 hover:bg-gray-100 transition-colors"
          >
            {isMenuOpen ? (
              <X className="w-6 h-6" aria-hidden />
            ) : (
              <Menu className="w-6 h-6" aria-hidden />
            )}
          </button>
        </div>
      </div>

      {/* ── Mobile Drawer ── */}
      {isMenuOpen && (
        <div
          className="fixed inset-0 z-[100] md:hidden"
          role="dialog"
          aria-modal
          aria-label="Navigation menu"
        >
          {/* Backdrop */}
          <div
            className="absolute inset-0 bg-gray-900/40 backdrop-blur-sm"
            onClick={() => setIsMenuOpen(false)}
            aria-hidden
          />

          <div className="absolute right-0 top-0 bottom-0 w-80 bg-white dark:bg-gray-900 shadow-2xl flex flex-col">
            <div className="p-4 flex items-center justify-between border-b border-gray-100 dark:border-gray-800">
              <span className="text-lg font-bold text-gray-900 dark:text-white">Menu</span>
              <button
                onClick={() => setIsMenuOpen(false)}
                aria-label="Close menu"
                className="p-2 rounded-lg text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors"
              >
                <X className="w-6 h-6" aria-hidden />
              </button>
            </div>

            <nav
              className="flex-1 overflow-y-auto p-4 space-y-1"
              aria-label="Mobile navigation"
            >
              <p className="text-xs font-semibold text-gray-400 uppercase tracking-widest px-2 mb-3">
                Navigation
              </p>
              {VISIBLE_NAV_LINKS.map((link) => {
                const isActive = pathname === link.href;
                const LinkIcon = link.icon;
                return (
                  <Link
                    key={link.name}
                    href={link.href}
                    aria-current={isActive ? "page" : undefined}
                    className={`flex items-center gap-4 px-4 py-3.5 rounded-2xl text-base font-medium transition-all ${
                      isActive
                        ? "text-indigo-600 bg-indigo-50 dark:bg-indigo-900/20"
                        : "text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-800"
                    }`}
                  >
                    <LinkIcon
                      className={`w-5 h-5 flex-shrink-0 ${isActive ? "text-indigo-600" : "text-gray-400 dark:text-gray-500"}`}
                      aria-hidden
                    />
                    <span className="flex-1">{link.name}</span>
                    {link.href === "/notifications" && unread > 0 && (
                      <span className="min-w-6 h-6 px-2 rounded-full bg-indigo-600 text-white text-xs leading-6 text-center">
                        {unread > 99 ? "99+" : unread}
                      </span>
                    )}
                  </Link>
                );
              })}

              {isConnected && (
                <Link
                  href={`/profile?address=${publicKey}`}
                  aria-current={pathname === "/profile" ? "page" : undefined}
                  className={`flex items-center gap-4 px-4 py-3.5 rounded-2xl text-base font-medium transition-all ${
                    pathname === "/profile"
                      ? "text-indigo-600 bg-indigo-50 dark:bg-indigo-900/20"
                      : "text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-800"
                  }`}
                >
                  <User
                    className={`w-5 h-5 flex-shrink-0 ${pathname === "/profile" ? "text-indigo-600" : "text-gray-400 dark:text-gray-500"}`}
                    aria-hidden
                  />
                  My Profile
                </Link>
              )}
            </nav>

            {/* Wallet Section - Mobile */}
            <div className="p-4 border-t border-gray-100 dark:border-gray-800 bg-white dark:bg-gray-900 mt-auto">
              {isConnected ? (
                <div className="flex items-center justify-between px-2 py-3">
                  <div className="flex items-center gap-2">
                    <div className="w-6 h-6 rounded-full bg-gradient-to-br from-indigo-500 to-purple-500 flex-shrink-0" />
                    <span className="text-base font-medium text-gray-900 dark:text-white truncate max-w-[140px]">
                      {address || "Connected"}
                    </span>
                    <button
                      onClick={copyAddress}
                      className="p-1.5 rounded-lg text-gray-400 hover:text-indigo-600 hover:bg-indigo-50 dark:hover:bg-indigo-900/20 transition-colors"
                      aria-label="Copy Address"
                    >
                      <Copy className="w-4 h-4" aria-hidden />
                    </button>
                  </div>

                  <button
                    onClick={handleDisconnect}
                    className="text-red-600 hover:text-red-700 font-medium text-base transition-colors flex items-center gap-1.5"
                  >
                    Disconnect
                    <LogOut className="w-4 h-4" aria-hidden />
                  </button>
                </div>
              ) : (
                <button
                  onClick={() => {
                    connect();
                    setIsMenuOpen(false);
                  }}
                  disabled={isConnecting}
                  className="w-full py-4 rounded-2xl bg-indigo-600 text-white font-semibold shadow-md hover:bg-indigo-700 transition-all disabled:opacity-60 disabled:cursor-not-allowed"
                >
                  {isConnecting ? "Connecting…" : "Connect Wallet"}
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </nav>
  );
}
