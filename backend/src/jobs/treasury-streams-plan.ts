/** What the executor knows about one active stream on the current ledger. */
export interface StreamSnapshot {
  id: number;
  paused: boolean;
  cancelled: boolean;
  /** Amount `execute_stream` would pay right now, in base units. */
  claimable: bigint;
}

/**
 * Streams to pay on this tick: live, with at least one interval due, and no
 * uncleared failure recorded against them. A stream with a recorded failure is
 * skipped until an operator clears that failure.
 */
export function selectExecutableStreams(
  streams: readonly StreamSnapshot[],
  failedStreamIds: ReadonlySet<number>,
): StreamSnapshot[] {
  return streams.filter(
    (stream) =>
      !stream.paused &&
      !stream.cancelled &&
      stream.claimable > 0n &&
      !failedStreamIds.has(stream.id),
  );
}
