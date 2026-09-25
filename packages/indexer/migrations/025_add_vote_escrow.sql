-- Up Migration
CREATE TABLE vote_escrow_locks (
  owner_address TEXT PRIMARY KEY,
  amount NUMERIC(39, 0) NOT NULL,
  start_ledger BIGINT NOT NULL,
  end_ledger BIGINT NOT NULL,
  initial_voting_power NUMERIC(39, 0) NOT NULL,
  withdrawn BOOLEAN NOT NULL DEFAULT false,
  updated_ledger BIGINT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_vote_escrow_locks_end_ledger ON vote_escrow_locks(end_ledger);
CREATE INDEX idx_vote_escrow_locks_active ON vote_escrow_locks(withdrawn, end_ledger);
CREATE INDEX idx_vote_escrow_locks_amount ON vote_escrow_locks(amount DESC);

-- Down Migration
DROP TABLE IF EXISTS vote_escrow_locks;
