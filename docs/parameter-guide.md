# Parameter Guide

This guide describes the governance parameters that operators and proposers need to understand to reason about proposal thresholds. It currently covers the proposer reputation system.

## Proposer reputation

The governor scales the flat `proposal_threshold` for each proposer by a reputation-based multiplier. A proposer with a strong track record can propose at a discount, and a proposer with repeated failed proposals pays a penalty. The architecture is described in [architecture.md](./architecture.md#proposer-reputation).

### Default scoring constants

Every governor uses the same constants from `default_config()`. These values are fixed in the contract and cannot be changed through governance, so there is no safe range for an operator to choose. The table below describes their effect.

| Constant | Default | When it applies |
| --- | --- | --- |
| `score_for_succeed` | +100 | A proposal passes voting and is queued |
| `score_for_executed` | +50 | A queued proposal is executed |
| `score_for_defeated` | −30 | Voting closes without quorum or a majority |
| `score_for_cancelled` | −20 | A proposal is cancelled |
| `score_for_expired` | −60 | A passed proposal is not executed within the grace period |
| `score_for_high_participation` | +30 | Added on top of `score_for_succeed` when a passed proposal's participation is at least 5,000 bps (50%) of quorum-eligible supply |
| `min_proposals_for_discount` | 3 | Below this many proposals, the multiplier cannot drop below 1x |
| `max_score` / `min_score` | 1000 / −1000 | Bounds on the reputation score |
| `max_threshold_multiplier_bps` | 20,000 (2x) | Multiplier at the minimum score (largest penalty) |
| `min_threshold_multiplier_bps` | 5,000 (0.5x) | Multiplier at the maximum score (largest discount) |
| `decay_rate_per_1000_ledgers` | 10 | Score moves toward zero per 1,000 ledgers since the last update |

### How the score maps to a threshold

The score is clamped to `[min_score, max_score]` and mapped linearly onto the multiplier range. A higher score always gives a lower or equal multiplier. The effective threshold is:

```
effective_threshold = proposal_threshold × threshold_multiplier_bps / 10,000
```

With a flat `proposal_threshold` of 1,000 units and the default constants:

| Score | Multiplier | Effective threshold |
| --- | --- | --- |
| −1000 | 2.0x | 2,000 |
| 0 | 1.25x | 1,250 |
| about +333 | about 1.0x | about 1,000 |
| +1000 | 0.5x | 500 |

A score of `0` is not neutral. Once a proposer has at least three proposals, a score of `0` gives a 1.25x threshold. The 1x point is near a score of +333. A proposer with no recorded outcomes yet stays at 1x.

### Decay

`apply_reputation_decay(proposer)` is permissionless. It moves a nonzero score back toward zero, by `decay_rate_per_1000_ledgers` for every 1,000 ledgers since the record was last updated. For example, a score of −60 with the default rate returns to `0` after about 6,000 ledgers without a new outcome. Calling the function again immediately has no effect.

### What operators can tune

Only the flat `proposal_threshold` in the governor settings is tunable through governance. The reputation multiplier then applies to whatever value is set. The `enabled` flag exists in the reputation config, but it is a compile-time constant set to `true` and has no runtime toggle.
