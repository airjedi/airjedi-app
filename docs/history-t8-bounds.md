# History T8 Bounds

These are the default limits for the selected-history transport. They are
allocation and scheduling budgets, not claims about production traffic.

## Recorder

- Retention: 30 minutes per track
- Sampling interval: 2 seconds
- Per-track samples: 901
- Global retained samples: 100,000
- Preview samples: 256
- Operation log: 4,096 operations
- Correction horizon: 30 seconds

## Wire And Client

- Samples per chunk: 64
- Maximum snapshot samples: 901
- Maximum snapshot chunks: 15
- Client history tracks: 1,000
- Client retained samples: 3,604
- Client active requests: 4
- Buffered operations per request: 256
- Client assembly rejects oversized chunks, duplicate sample identities, stale
  sessions, and superseded request identities

## Agent Transfer Scheduler

- Active requests per client: 4
- Active transfers: 64
- Snapshot samples in flight: 7,208
- Estimated snapshot bytes in flight: 16 MiB
- History messages per update: 8
- Background messages reserved per update: 1
- Selected transfers are scheduled before background transfers, with one
  transfer per client per scheduling round
- Completed background transfers are released after cache fill; selected
  transfers remain subscribed for live operations

The agent reports retained samples and estimated bytes, pending transfers,
buffered operations, retries, cancellations, rejected requests, revision gaps,
session invalidations, snapshot truncations, completed snapshots, and the last
snapshot synchronization latency. The client read model reports the matching
retained and buffered memory, request, retry, stale-response, truncation, and
latency counters.

## Deterministic Profiles

The stress tests exercise 100, 500, and 1,000 track profiles without sleeping
or depending on wall-clock transfer timing. They assert that recorder and client
sample bounds remain fixed and that bounded request/assembly state is released
on cancellation, reconnect invalidation, supersession, and session changes.

The tests run on the development host. No target Raspberry Pi benchmark was
available for this change, so hardware latency and CPU measurements remain an
explicit follow-up for deployment validation.

Recorded test environment: macOS Darwin 25.6.0 arm64 on `ccustine-mac`,
`rustc 1.96.0`, `cargo 1.96.0`. The deterministic profile tests completed
without real-time waits; they report bound assertions rather than wall-clock
throughput benchmarks.
