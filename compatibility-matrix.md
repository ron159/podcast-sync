# Compatibility matrix

| Component | Contract | Status |
|---|---|---|
| AntennaPod Android StateSync client | Base URL + `/v1/sync`, Bearer Token, payload key omitted, 16-digit fractional index | Compatible through server normalization |
| FennecCast iOS StateSync client | Base URL + `/v1/sync`, Bearer Token, payload key required, 16-digit fractional index | Compatible |
| StateSync Lite v1 server | Three entity types, 500 mutations, 1 MiB, ordered cursor | Implemented |
| Snapshot v1 | Normalized JSON only, no SQLite/media | Implemented; clients may opt in later |
| Device revocation | Device ID tombstone plus shared Token rotation | Implemented |
| End-to-end encryption | Encrypted envelope | Not part of protocol v1 |
