# Canonical identity boundary

Clients own canonical Feed and Episode key generation. The server treats `entity_key` as an opaque, non-empty protocol identifier and does not fetch Feed URLs or recalculate hashes.

For compatibility, the server enforces equality when an iOS payload already contains `feed_key` or `episode_key`. When an Android payload omits that duplicate field, the server copies the envelope `entity_key` into the stored payload before assigning `server_seq`.

This normalization is deterministic, so retrying the same `(device_id, local_seq)` remains idempotent. Reusing the sequence with any other HLC, entity type, key or normalized payload returns `409 sequence_conflict`.

