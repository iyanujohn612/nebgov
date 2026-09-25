import request from "supertest";
import app from "../index";
import pool from "../db/pool";

/**
 * DB-backed route coverage for the voting-rewards endpoints (issue #1011),
 * following the same pattern as the governance-tuning and signaling route
 * tests: real Postgres, real migrations, rows cleaned up afterwards.
 *
 * `VOTING_REWARDS_CONTRACT_ID` is deliberately left unset here, so
 * `refreshClaimStatuses` short-circuits and the stored `claimed` flags stand
 * — these tests are about the HTTP surface, not about RPC reachability.
 */
describe("Voting Rewards Endpoints", () => {
  const EPOCH_A = 900001n;
  const EPOCH_B = 900002n;
  const ALICE = "GA3I6MVQC2EXERDKLVWNFGGYEHII5ZVWFS4ZUQGKAP3XRJWR7P5FUGQJ";
  const BOB = "GAKXV3A3HTPD6VG63VDC7YHEGXFUW64PGVOKPIOYMIXLZRAXLQMG22CN";
  const CAROL = "GAS6QC6USQHIUKQH4EY6KC62IN7Q4NX3M73L3PI7YWQZYHDFQCTB4LZY";
  const PROOF_A = ["aa".repeat(32), "bb".repeat(32)];
  const PROOF_B = ["cc".repeat(32)];

  beforeAll(async () => {
    await pool.query(
      `INSERT INTO voting_reward_epochs
         (epoch_id, start_ledger, end_ledger, merkle_root, total_reward_amount, published_at)
       VALUES ($1, 1000, 2000, $2, '3000', NOW()), ($3, 2000, 3000, NULL, '0', NULL)`,
      [EPOCH_A.toString(), "11".repeat(32), EPOCH_B.toString()],
    );
    await pool.query(
      `INSERT INTO voting_reward_claims (epoch_id, claimant_address, amount, merkle_proof, claimed)
       VALUES ($1, $2, '2000', $3, FALSE),
              ($1, $4, '900', $5, TRUE),
              ($1, $6, '100', $5, FALSE)`,
      [
        EPOCH_A.toString(),
        ALICE,
        JSON.stringify(PROOF_A),
        BOB,
        JSON.stringify(PROOF_B),
        CAROL,
      ],
    );
  });

  afterAll(async () => {
    await pool.query(`DELETE FROM voting_reward_claims WHERE epoch_id = ANY($1)`, [
      [EPOCH_A.toString(), EPOCH_B.toString()],
    ]);
    await pool.query(`DELETE FROM voting_reward_epochs WHERE epoch_id = ANY($1)`, [
      [EPOCH_A.toString(), EPOCH_B.toString()],
    ]);
    await pool.end();
  });

  describe("GET /voting-rewards/epochs", () => {
    it("lists epochs newest first, serializing bigints as strings", async () => {
      const response = await request(app).get("/voting-rewards/epochs?limit=2").expect(200);

      expect(response.body.data).toHaveLength(2);
      expect(response.body.total).toBe(2);
      expect(response.body.data[0].epoch_id).toBe(EPOCH_B.toString());
      expect(response.body.data[1]).toMatchObject({
        epoch_id: EPOCH_A.toString(),
        merkle_root: "11".repeat(32),
        total_reward_amount: "3000",
      });
    });

    it("honours the offset", async () => {
      const response = await request(app).get("/voting-rewards/epochs?limit=1&offset=1").expect(200);

      expect(response.body.data.map((epoch: { epoch_id: string }) => epoch.epoch_id)).toEqual([
        EPOCH_A.toString(),
      ]);
      expect(response.body.total).toBe(2);
    });

    it("rejects a limit outside the allowed range", async () => {
      const response = await request(app).get("/voting-rewards/epochs?limit=0").expect(400);
      expect(response.body).toHaveProperty("errors");
    });
  });

  describe("GET /voting-rewards/epochs/:epochId", () => {
    it("returns one epoch", async () => {
      const response = await request(app)
        .get(`/voting-rewards/epochs/${EPOCH_A}`)
        .expect(200);

      expect(response.body).toMatchObject({
        epoch_id: EPOCH_A.toString(),
        start_ledger: 1000,
        end_ledger: 2000,
        total_reward_amount: "3000",
      });
      expect(response.body.published_at).not.toBeNull();
    });

    it("reports an unpublished epoch with a null root", async () => {
      const response = await request(app)
        .get(`/voting-rewards/epochs/${EPOCH_B}`)
        .expect(200);

      expect(response.body.merkle_root).toBeNull();
      expect(response.body.published_at).toBeNull();
    });

    it("404s for an epoch that has not been computed", async () => {
      await request(app).get("/voting-rewards/epochs/987654321").expect(404);
    });

    it("rejects a non-numeric epoch id", async () => {
      await request(app).get("/voting-rewards/epochs/not-a-number").expect(400);
    });
  });

  describe("GET /voting-rewards/epochs/:epochId/leaderboard", () => {
    it("ranks claimants by amount and omits their proofs", async () => {
      const response = await request(app)
        .get(`/voting-rewards/epochs/${EPOCH_A}/leaderboard`)
        .expect(200);

      expect(response.body.data.map((row: { claimant_address: string }) => row.claimant_address))
        .toEqual([ALICE, BOB, CAROL]);
      expect(response.body.data[0]).toEqual({
        claimant_address: ALICE,
        amount: "2000",
        claimed: false,
      });
      expect(response.body.data[0]).not.toHaveProperty("merkle_proof");
    });

    it("honours the limit", async () => {
      const response = await request(app)
        .get(`/voting-rewards/epochs/${EPOCH_A}/leaderboard?limit=1`)
        .expect(200);

      expect(response.body.data).toHaveLength(1);
      expect(response.body.total).toBe(3);
    });

    it("returns an empty list for an epoch with no claimants", async () => {
      const response = await request(app)
        .get(`/voting-rewards/epochs/${EPOCH_B}/leaderboard`)
        .expect(200);

      expect(response.body.data).toEqual([]);
      expect(response.body.total).toBe(0);
    });
  });

  describe("GET /voting-rewards/claims/:address", () => {
    it("returns an address's rewards with the ready-to-submit proof", async () => {
      const response = await request(app)
        .get(`/voting-rewards/claims/${ALICE}`)
        .expect(200);

      expect(response.body.data).toHaveLength(1);
      expect(response.body.data[0]).toEqual({
        epoch_id: EPOCH_A.toString(),
        claimant_address: ALICE,
        amount: "2000",
        merkle_proof: PROOF_A,
        claimed: false,
      });
      expect(response.body.total_unclaimed).toBe("2000");
    });

    it("excludes an already-claimed epoch from the unclaimed total", async () => {
      const response = await request(app).get(`/voting-rewards/claims/${BOB}`).expect(200);

      expect(response.body.data[0].claimed).toBe(true);
      expect(response.body.total_unclaimed).toBe("0");
    });

    it("returns an empty list for an address that never voted", async () => {
      const response = await request(app)
        .get("/voting-rewards/claims/GCAY5IGF2CMHCG56BR7FBJZOMRGP7UJKSOESULXEJ5XHB3U4ELYJG4IC")
        .expect(200);

      expect(response.body.data).toEqual([]);
      expect(response.body.total_unclaimed).toBe("0");
    });

    it("rejects anything that is not a Stellar address, rather than querying with it", async () => {
      const response = await request(app).get("/voting-rewards/claims/nope").expect(400);
      expect(response.body).toHaveProperty("errors");
    });

    it("rejects an invalid base32 character", async () => {
      const response = await request(app)
        .get("/voting-rewards/claims/GA3I6MVQC2EXERDKLVWNFGGYEHII5ZVWFS4ZUQGKAP3XRJWR7P5FUGQ0")
        .expect(400);
      expect(response.body).toHaveProperty("errors");
    });

    it("rejects a bad checksum even when the shape looks like a Stellar address", async () => {
      const response = await request(app)
        .get("/voting-rewards/claims/GA3I6MVQC2EXERDKLVWNFGGYEHII5ZVWFS4ZUQGKAP3XRJWR7P5FUGQA")
        .expect(400);
      expect(response.body).toHaveProperty("errors");
    });
  });
});
