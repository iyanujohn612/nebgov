import { nativeToScVal, SorobanRpc, xdr } from "@stellar/stellar-sdk";
import { processEvents } from "../events";
import { pool } from "../db";

jest.mock("../db", () => ({ pool: { query: jest.fn() } }));
jest.mock("../cache", () => ({ invalidate: jest.fn(), invalidatePattern: jest.fn() }));
jest.mock("../ws", () => ({ broadcast: jest.fn() }));

const CONTRACT = "CCOSPONSORSHIP";
const config = { rpcUrl: "http://fake", governorAddress: "CGOVERNOR", coSponsorshipAddress: CONTRACT, pollIntervalMs: 1 };
function event(type: string, value: unknown, topic = "GSPONSOR", ledger = 100): SorobanRpc.Api.EventResponse {
  const encoded = Array.isArray(value) ? xdr.ScVal.scvVec(value.map((v) => nativeToScVal(v))) : nativeToScVal(value);
  return { id: `${type}-${ledger}`, type: "contract", ledger, contractId: CONTRACT, txHash: "tx", topic: [nativeToScVal(type, { type: "symbol" }), nativeToScVal(topic)], value: encoded } as unknown as SorobanRpc.Api.EventResponse;
}
async function run(events: SorobanRpc.Api.EventResponse[]) {
  (pool.query as jest.Mock).mockResolvedValue({ rows: [] });
  return processEvents({ getEvents: jest.fn().mockResolvedValue({ events }) } as unknown as SorobanRpc.Server, config, 100);
}

describe("co-sponsorship event indexing", () => {
  beforeEach(() => jest.clearAllMocks());
  it.each([
    ["DraftCreated", { draft_id: 1n, description_hash: new Uint8Array([1, 2]), metadata_uri: "ipfs://draft", created_ledger: 90, expiry_ledger: 200 }, "INSERT INTO drafts"],
    ["CoSponsored", [1n, 25n, 100n], "INSERT INTO draft_co_sponsors"],
    ["CoSponsorshipWithdrawn", [1n, 25n, 75n], "UPDATE draft_co_sponsors"],
    ["DraftFinalized", [1n, 9n], "UPDATE drafts SET finalized"],
    ["DraftCancelled", [1n], "UPDATE drafts SET cancelled"],
    ["DraftExpired", [1n], "UPDATE drafts SET expired"],
  ])("handles %s", async (type, value, sql) => {
    await run([event(type, value as unknown[])]);
    expect((pool.query as jest.Mock).mock.calls.some(([query]) => String(query).includes(sql))).toBe(true);
  });
  it("supports the complete draft lifecycle", async () => {
    await run([event("DraftCreated", { draft_id: 1n, description_hash: new Uint8Array([1]), metadata_uri: "uri", created_ledger: 90, expiry_ledger: 200 }), event("CoSponsored", [1n, 25n, 25n]), event("DraftCancelled", [1n])]);
    expect((pool.query as jest.Mock).mock.calls.filter(([query]) => String(query).includes("drafts")).length).toBeGreaterThanOrEqual(3);
  });
});
