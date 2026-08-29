# Conflict rules

The NAS server is an authenticated operation relay. It orders accepted mutations with a monotonically increasing `server_seq`, but it does not merge subscription, playback or queue business state.

Android and iOS apply the v1 rules locally:

- Subscription and queue presence are HLC last-writer-wins registers with tombstones.
- Playback `seek`, `mark_played` and `mark_unplayed` outrank ordinary checkpoints; otherwise HLC decides.
- Queue order uses fractional index tokens, then entity key as a deterministic tie-breaker.
- HLC order is `(wall_ms, counter, device_id)`.

The server never echoes a device's own operations back to that device. Its `next_cursor` still advances over those rows so retries converge.

