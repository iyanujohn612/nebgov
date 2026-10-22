import {
  TreasuryStreamsError,
  TreasuryStreamsErrorCode,
  mapTreasuryStream,
  parseTreasuryStreamsError,
} from "../treasuryStreams";

describe("mapTreasuryStream", () => {
  it("maps the contract struct to camelCase with bigint amounts", () => {
    const stream = mapTreasuryStream({
      id: 3,
      recipient: "GRECIPIENT",
      asset: "CASSET",
      amount_per_interval: "1000000000000",
      interval_ledgers: 100,
      start_ledger: 10,
      end_ledger: 5000,
      last_streamed_ledger: 210,
      total_streamed: 2000,
      paused: false,
      cancelled: true,
      created_by: "CGOVERNOR",
    });

    expect(stream).toEqual({
      id: 3,
      recipient: "GRECIPIENT",
      asset: "CASSET",
      amountPerInterval: 1_000_000_000_000n,
      intervalLedgers: 100,
      startLedger: 10,
      endLedger: 5000,
      lastStreamedLedger: 210,
      totalStreamed: 2000n,
      paused: false,
      cancelled: true,
      createdBy: "CGOVERNOR",
    });
  });
});

describe("parseTreasuryStreamsError", () => {
  it("maps a contract error number to its typed code", () => {
    const error = parseTreasuryStreamsError("HostError: Error(Contract, #12)");

    expect(error).toBeInstanceOf(TreasuryStreamsError);
    expect(error.code).toBe(TreasuryStreamsErrorCode.StreamNotDue);
    expect(error.message).toBe("No interval has come due on this stream yet");
  });

  it("falls back to a simulation failure when no contract code is present", () => {
    const error = parseTreasuryStreamsError("network unreachable");

    expect(error.code).toBe(TreasuryStreamsErrorCode.SimulationFailed);
    expect(error.message).toContain("network unreachable");
  });
});
