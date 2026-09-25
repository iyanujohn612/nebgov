import { nativeToScVal, SorobanRpc, xdr } from "@stellar/stellar-sdk";
import { processEvents } from "../events";
import { pool } from "../db";
jest.mock("../db", () => ({ pool: { query: jest.fn() } }));
jest.mock("../cache", () => ({ invalidate: jest.fn(), invalidatePattern: jest.fn() }));
jest.mock("../ws", () => ({ broadcast: jest.fn() }));
const CONTRACT = "CBONDS";
const config = { rpcUrl: "http://fake", governorAddress: "CGOVERNOR", proposalBondsAddress: CONTRACT, pollIntervalMs: 1 };
function event(type: string, values: unknown[], ledger: number): SorobanRpc.Api.EventResponse {
  return { id: `${type}-${ledger}`, type: "contract", ledger, contractId: CONTRACT, txHash: "tx", topic: [nativeToScVal(type, { type: "symbol" }), nativeToScVal("GPROPOSER")], value: xdr.ScVal.scvVec(values.map((v) => nativeToScVal(v))) } as unknown as SorobanRpc.Api.EventResponse;
}
async function run(events: SorobanRpc.Api.EventResponse[]) {
  (pool.query as jest.Mock).mockResolvedValue({ rows: [] });
  return processEvents({ getEvents: jest.fn().mockResolvedValue({ events }) } as unknown as SorobanRpc.Server, config, 100);
}
describe("proposal bond event indexing", () => {
  beforeEach(() => jest.clearAllMocks());
  it.each([["BondLocked", "INSERT INTO proposal_bonds"], ["BondRefunded", "UPDATE proposal_bonds SET state = 'refunded'"], ["BondSlashed", "UPDATE proposal_bonds SET state = 'slashed'"]])("handles %s", async (type, sql) => {
    await run([event(type, [new Uint8Array([1, 2]), 100n, "GRECIPIENT"], 100)]);
    expect((pool.query as jest.Mock).mock.calls.some(([query]) => String(query).includes(sql))).toBe(true);
  });
  it("transitions a bond from locked to refunded and is replay safe", async () => {
    await run([event("BondLocked", [new Uint8Array([1]), 100n], 100), event("BondRefunded", [new Uint8Array([1]), 100n], 101), event("BondRefunded", [new Uint8Array([1]), 100n], 101)]);
    expect((pool.query as jest.Mock).mock.calls.filter(([query]) => String(query).includes("UPDATE proposal_bonds SET state = 'refunded'")).length).toBe(2);
  });
});
