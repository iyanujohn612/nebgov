-- Up Migration

-- One row per failed `execute_stream` attempt. The executor skips a stream
-- while it has an uncleared row, so a broken stream is not retried on every
-- tick. An operator clears it by setting cleared_at.
CREATE TABLE treasury_stream_failures (
  id SERIAL PRIMARY KEY,
  stream_id BIGINT NOT NULL,
  failed_at_ledger INTEGER NOT NULL,
  error_message TEXT NOT NULL,
  attempt_count INTEGER NOT NULL DEFAULT 1,
  first_failure_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_failure_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  cleared_at TIMESTAMPTZ
);

CREATE INDEX idx_treasury_stream_failures_open
  ON treasury_stream_failures (stream_id)
  WHERE cleared_at IS NULL;

-- Down Migration

DROP TABLE IF EXISTS treasury_stream_failures;
