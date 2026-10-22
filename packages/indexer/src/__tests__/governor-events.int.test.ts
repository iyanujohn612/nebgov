import { SorobanRpc, StrKey, nativeToScVal, xdr } from "@stellar/stellar-sdk";
import { initDb, pool } from "../db";
import { processEvents } from "../events";

class FakeServer {
  constructor(private events: SorobanRpc.Api.EventResponse[]) {}
  async getEvents() {
    return { events: this.events };
  }
}

function encodeValue(value: unknown): xdr.ScVal {
  if (Array.isArray(value)) {
    return xdr.ScVal.scvVec(value.map(encodeValue));
  }
  return nativeToScVal(value);
}

function makeEvent(params: {
  contractId: string;
  ledger: number;
  type: string;
  topicArgs?: any[];
  value: unknown;
  ledgerClosedAt?: string;
}): SorobanRpc.Api.EventResponse {
  const topic = [
    nativeToScVal(params.type, { type: "symbol" }),
    ...(params.topicArgs ?? []).map((a) =>
      nativeToScVal(a, { type: "address" }),
    ),
  ];
  const value = encodeValue(params.value);
  return {
    type: "contract",
    ledger: params.ledger,
    ledgerClosedAt: params.ledgerClosedAt ?? "2026-01-01T00:00:00Z",
    contractId: params.contractId as any,
    topic,
    value,
  } as any;
}

describe("governor event indexing (integration)", () => {
  const dbUrl = process.env.DATABASE_URL;
  if (!dbUrl) {
    it.skip("DATABASE_URL not set", () => undefined);
    return;
  }

  const GOVERNOR = StrKey.encodeContract(Buffer.alloc(32, 1));
  const CALLER = StrKey.encodeEd25519PublicKey(Buffer.alloc(32, 2));
  const PROPOSER = StrKey.encodeEd25519PublicKey(Buffer.alloc(32, 3));

  beforeAll(async () => {
    await initDb();
    await pool.query("DELETE FROM config_updates");
    await pool.query("DELETE FROM governor_upgrades");
    try {
      await pool.query("DELETE FROM proposer_reputation");
    } catch {
      // Table may not exist if migration hasn't run
    }
    try {
      await pool.query("DELETE FROM effective_threshold_history");
    } catch {
      // Table may not exist if migration hasn't run
    }
  });

  afterAll(async () => {
    await pool.end();
  });

  it("indexes ConfigUpdated event into config_updates", async () => {
    const configUpdated = makeEvent({
      contractId: GOVERNOR,
      ledger: 200,
      type: "ConfigUpdated",
      ledgerClosedAt: "2026-06-01T12:00:00Z",
      value: {
        old_settings: {
          voting_delay: 1,
          voting_period: 2,
          quorum_numerator: 30,
          proposal_threshold: 1000n,
          guardian: "GOLDGUARDIAN1111111111111111111111111111",
          proposal_grace_period: 3,
        },
        new_settings: {
          voting_delay: 2,
          voting_period: 3,
          quorum_numerator: 35,
          proposal_threshold: 2000n,
          guardian: "GNEWGUARDIAN111111111111111111111111111",
          proposal_grace_period: 4,
        },
      },
    });

    const server = new FakeServer([configUpdated]) as unknown as SorobanRpc.Server;
    const latest = await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    expect(latest).toBe(200);

    const rows = await pool.query(
      "SELECT ledger, old_settings, new_settings, ledger_closed_at FROM config_updates ORDER BY id DESC LIMIT 1",
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].ledger).toBe(200);
    expect(rows.rows[0].old_settings.voting_delay).toBe(1);
    const settings = rows.rows[0].new_settings;
    expect(settings.voting_delay).toBe(2);
    expect(settings.voting_period).toBe(3);
    expect(settings.quorum_numerator).toBe(35);
    expect(rows.rows[0].ledger_closed_at.toISOString()).toBe("2026-06-01T12:00:00.000Z");
  });

  it("indexes GovernorUpgraded event into governor_upgrades", async () => {
    const newHashBytes = new Uint8Array(32);
    for (let i = 0; i < 32; i++) {
      newHashBytes[i] = i;
    }

    const upgraded = makeEvent({
      contractId: GOVERNOR,
      ledger: 201,
      type: "GovernorUpgraded",
      value: {
        old_hash: new Uint8Array([0, 1, 2]),
        new_hash: newHashBytes,
      },
    });

    const server = new FakeServer([upgraded]) as unknown as SorobanRpc.Server;
    const latest = await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    expect(latest).toBe(201);

    const rows = await pool.query(
      "SELECT ledger, new_wasm_hash FROM governor_upgrades ORDER BY id DESC LIMIT 1",
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].ledger).toBe(201);
    expect(rows.rows[0].new_wasm_hash).toBe(
      "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    );
  });

  it("indexes legacy config_updated short-form event", async () => {
    const configUpdatedLegacy = makeEvent({
      contractId: GOVERNOR,
      ledger: 202,
      type: "config_updated",
      topicArgs: [],
      value: {
        new_settings: {
          voting_delay: 3,
          voting_period: 4,
          quorum_numerator: 40,
          proposal_threshold: 3000n,
          guardian: "GLEGACYGUARDIAN1111111111111111111111111",
          proposal_grace_period: 5,
        },
      },
    });

    const server = new FakeServer([configUpdatedLegacy]) as unknown as SorobanRpc.Server;
    const latest = await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    expect(latest).toBe(202);

    const rows = await pool.query(
      "SELECT old_settings, new_settings FROM config_updates WHERE ledger = 202 ORDER BY id DESC LIMIT 1",
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].old_settings).toBeNull();
    expect(rows.rows[0].new_settings.voting_delay).toBe(3);
  });

  it("indexes legacy upgraded short-form event", async () => {
    const upgradedLegacy = makeEvent({
      contractId: GOVERNOR,
      ledger: 203,
      type: "upgraded",
      value: {
        new_hash: new Uint8Array([255, 254, 253]),
      },
    });

    const server = new FakeServer([upgradedLegacy]) as unknown as SorobanRpc.Server;
    const latest = await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    expect(latest).toBe(203);

    const rows = await pool.query(
      "SELECT new_wasm_hash FROM governor_upgrades WHERE ledger = 203 ORDER BY id DESC LIMIT 1",
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].new_wasm_hash).toBe("fffefd");
  });

  it("indexes ProposalCancelled from cancel_queued() — value is a tuple (proposal_id, queue_time, current_ledger)", async () => {
    // Insert a proposal row so the UPDATE has something to affect
    await pool.query(
      `INSERT INTO proposals (id, proposer, description, start_ledger, end_ledger)
       VALUES ('77', $1, 'cancel_queued test', 1, 100)
       ON CONFLICT (id) DO NOTHING`,
      [CALLER],
    );

    // cancel_queued() raw event shape:
    //   topic[0] = Symbol("ProposalCancelled")
    //   topic[1] = caller address
    //   value    = array [proposal_id, queue_time, current_ledger]
    const cancelQueuedEvent = {
      type: "contract",
      ledger: 210,
      ledgerClosedAt: "2026-06-01T00:00:00Z",
      contractId: GOVERNOR,
      topic: [
        nativeToScVal("ProposalCancelled", { type: "symbol" }),
        nativeToScVal(CALLER, { type: "address" }),
      ],
      value: nativeToScVal([77n, 50n, 210n]),
    } as unknown as SorobanRpc.Api.EventResponse;

    const server = new FakeServer([cancelQueuedEvent]) as unknown as SorobanRpc.Server;
    await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    const rows = await pool.query(
      "SELECT cancelled FROM proposals WHERE id = '77'",
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].cancelled).toBe(true);
  });

  it("indexes ProposalCancelled from cancel() — value is an object { proposal_id, caller }", async () => {
    await pool.query(
      `INSERT INTO proposals (id, proposer, description, start_ledger, end_ledger)
       VALUES ('78', $1, 'cancel test', 1, 100)
       ON CONFLICT (id) DO NOTHING`,
      [PROPOSER],
    );

    // cancel() / emit_proposal_cancelled event shape:
    //   topic[0] = Symbol("ProposalCancelled")
    //   value    = { proposal_id, caller }
    const cancelEvent = {
      type: "contract",
      ledger: 211,
      ledgerClosedAt: "2026-06-01T00:00:00Z",
      contractId: GOVERNOR,
      topic: [
        nativeToScVal("ProposalCancelled", { type: "symbol" }),
      ],
      value: nativeToScVal({ proposal_id: 78n, caller: PROPOSER }),
    } as unknown as SorobanRpc.Api.EventResponse;

    const server = new FakeServer([cancelEvent]) as unknown as SorobanRpc.Server;
    await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    const rows = await pool.query(
      "SELECT cancelled FROM proposals WHERE id = '78'",
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].cancelled).toBe(true);
  });

  it("includes treasuryAddress in contractIds when configured", async () => {
    const TREASURY = "CTREASURYTESTADDRESS000000000000000000000000000000000000";
    let capturedContractIds: string[] = [];

    class ServerWithCapture {
      async getEvents(opts: any) {
        capturedContractIds = opts.filters[0].contractIds;
        return { events: [] };
      }
    }

    const server = new ServerWithCapture() as unknown as SorobanRpc.Server;
    await processEvents(
      server,
      {
        rpcUrl: "http://fake",
        governorAddress: GOVERNOR,
        treasuryAddress: TREASURY,
        pollIntervalMs: 1,
      },
      1,
    );

    expect(capturedContractIds).toContain(GOVERNOR);
    expect(capturedContractIds).toContain(TREASURY);
  });

  it("indexes ReputationUpdated event into proposer_reputation", async () => {
    const proposer = StrKey.encodeEd25519PublicKey(Buffer.alloc(32, 4));
    const reputationUpdated = makeEvent({
      contractId: GOVERNOR,
      ledger: 204,
      type: "ReputationUpdated",
      topicArgs: [proposer],
      value: { proposer, old_score: 100, new_score: 150, reason: "active_proposal_passed" },
    });

    const server = new FakeServer([reputationUpdated]) as unknown as SorobanRpc.Server;
    const latest = await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    expect(latest).toBe(204);

    const rows = await pool.query(
      "SELECT proposer_address, reputation_score, last_updated_ledger FROM proposer_reputation WHERE proposer_address = $1",
      [proposer],
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].reputation_score).toBe(150);
    expect(rows.rows[0].last_updated_ledger).toBe(204);
  });

  it("indexes EffectiveThresholdChanged event into effective_threshold_history (issue #919)", async () => {
    const proposer = StrKey.encodeEd25519PublicKey(Buffer.alloc(32, 5));
    const thresholdChanged = makeEvent({
      contractId: GOVERNOR,
      ledger: 205,
      type: "EffectiveThresholdChanged",
      topicArgs: [proposer],
      value: { proposer, old_threshold: 1000n, new_threshold: 1200n },
    });

    const server = new FakeServer([thresholdChanged]) as unknown as SorobanRpc.Server;
    const latest = await processEvents(
      server,
      { rpcUrl: "http://fake", governorAddress: GOVERNOR, pollIntervalMs: 1 },
      1,
    );

    expect(latest).toBe(205);

    const rows = await pool.query(
      "SELECT proposer_address, ledger, old_threshold, new_threshold FROM effective_threshold_history WHERE ledger = 205",
    );
    expect(rows.rows.length).toBe(1);
    expect(rows.rows[0].proposer_address).toBe(proposer);
    expect(Number(rows.rows[0].old_threshold)).toBe(1000);
    expect(Number(rows.rows[0].new_threshold)).toBe(1200);
  });
});
