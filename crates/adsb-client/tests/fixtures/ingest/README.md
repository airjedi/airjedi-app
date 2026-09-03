# Correlated ingest fixtures (BEAST + NDJSON)

These fixtures drive the **ingest-simulation** test (`tests/ingest_replay.rs`):
a real feeder capture replayed through the full pipeline
(`BeastFramer -> Rs1090Decoder -> AircraftTracker`), cross-referenced against
the correlated readsb enrichment stream.

> **Separate from the raw I/Q fixtures.** The `*.bin` I/Q captures one level up
> (`../README.md`) are raw RTL-SDR samples for a *future* AirJedi-native
> demodulator and are gitignored/large. The files here are **decoded network
> output** (BEAST frames + readsb NDJSON), small enough to commit gzipped, and
> exercise a different layer: framing/decode/track/enrichment, not
> demodulation. Do not conflate the two.

## Why a *pair*

Two streams captured **simultaneously** from the live feeder, so they join by
ICAO + time:

- `beast_30005_*.bin.gz` - raw BEAST binary from readsb `--net-bo-port 30005`
  (Mode-S frames with 12 MHz receiver timestamps). This is the decode input.
- `readsb_30047_*.ndjson.gz` - readsb's streaming JSON (`--net-json-port`,
  observed on `:30047`). Carries the `type` field
  (`adsb_icao` / `tisb_icao` / `mlat` / ...) that AirJedi's
  `src/adsb/enrichment.rs` uses for position-source tagging. This is the
  enrichment/ground-truth side.

MLAT and TIS-B source tags live **only** in the NDJSON, never in BEAST - which
is why both halves are needed to test source tagging end to end.

## Fixture: 2026-09-02

| | BEAST | NDJSON |
|---|---|---|
| File | `beast_30005_20260902.bin.gz` | `readsb_30047_20260902.ndjson.gz` |
| Compressed | 1,023,055 B | 685,888 B |
| Uncompressed | 2,024,918 B | 9,098,304 B (17,289 lines) |
| sha256 (`.gz`) | `cf5ee7a27e15a0d55c649058657a49d02fe6eb7bb07f664945811ecec6fe4835` | `0ce7f89079fda701640bf1e6a525ce2e769cf4bc7ddffec3a8a800f21b1e4f5d` |
| sha256 (raw) | `dc16ce3478d04c251c0818dbfc91af065382439e49ace41c8564b9631b7988f4` | `f741ee90311d14156dbd0272ec8204804cb25a83a273bb520d80630cb9f796b2` |

- **Captured:** 2026-09-02, ~5 min (18:59:30Z-19:04:30Z), from the live feeder
  at `192.168.1.10` over the LAN (readsb kept running - non-disruptive; the
  dongle was *not* taken, unlike the I/Q captures).
- **Receiver:** 37.7139, -97.1364, 421.24 m (from `infra/k3s-pi/configmap.yaml`).
- **Traffic:** 86 distinct aircraft. BEAST holds ~105,270 Mode-S frames
  (~57k short + ~48k long).
- **Source types present:** `adsb_icao` **only**. No `mlat`, no `tisb` in this
  window - every aircraft had ADS-B out, so nothing needed multilateration.
  **A future capture is still needed to get a real MLAT/TIS-B sample** (they are
  sparse; capture opportunistically when a Mode-S-only contact is being
  multilaterated). See the capture recipe in `infra/k3s-pi/README.md`.

### What the replay decodes (regression baseline)

Via `Rs1090Decoder` (the crate default): ~17,133 positions, ~17,166 velocities,
~1,774 identifications, ~59k altitude replies; 98 aircraft tracked, all 86
NDJSON aircraft matched. The test asserts on robust bands and set-overlap, not
exact counts (exact-value `insta` snapshots arrive with the design-b
display-component layer).

> **Position decode depends on frame timestamps.** CPR even/odd pairing is
> time-windowed and must be driven by each frame's BEAST receiver timestamp,
> not wall-clock. Replaying this fixture originally decoded only 90 positions
> because `Rs1090Decoder` timestamped frames with `Utc::now()`; threading the
> BEAST frame timestamp through fixed it (90 -> 17,133). This is why the
> capture must preserve raw BEAST bytes (timestamps intact), not a reframed
> copy.

## Fixture: 2026-09-02 (MLAT)

The MLAT-bearing counterpart to the ADS-B-only fixture above - the one that
exercises the `mlat` source-tag path end to end. Captured via SSH tunnel to the
Pi's localhost (readsb kept running - non-disruptive), because direct remote
access to the ports was wedged at the time; see the recipe in
`infra/k3s-pi/README.md`.

| | BEAST | NDJSON |
|---|---|---|
| File | `beast_30005_20260902_mlat.bin.gz` | `readsb_30047_20260902_mlat.ndjson.gz` |
| Compressed | 6,188,936 B | 4,199,909 B |
| Uncompressed | 12,230,316 B | 54,587,943 B (104,328 lines) |
| sha256 (`.gz`) | `18305c3e3812266e008f2890b776fc3c0f097f3bcc631c4f1c52397418d20394` | `e2c6c3ffd86e4e525cc483887e987a05868b61ec27ce9730491a725f9b20fbce` |
| sha256 (raw) | `8b18769a4eb9da7da7aa0714e483cdfccab39be6320059bd5e0d0af35c16cba5` | `348bef23f872e1e36aa3de5e0768f76d2507d4cc25e34c75f1ad8685ec8b543c` |

- **Captured:** 2026-09-02, ~30 min (21:38-22:08Z), SSH tunnel to the feeder's
  localhost. Kept full/uncurated by choice (~9.9 MB gz, ~6x the ADS-B fixture) -
  MLAT hits are spread across the whole window, so it is not time-trimmable
  without losing targets.
- **Traffic:** 164 aircraft, 104,328 NDJSON lines.
- **Source types:** `adsb_icao` 104,273, **`mlat` 55** (0 `tisb`).
- **MLAT targets: 17 distinct aircraft** -
  `a93257` SKW3595, `a94ff9` TFF900, `ab15a5` N813MS, `a59dbb` N461EE,
  `a9f652`, `ae5e13`, `a083cb` TOG132, `a26412` EJA253, `a191a8` RAX270,
  `a24de8` MXY1695, `a9ab0b` XSR722, `a7907c` LXJ587, `a2de1f` N284PC,
  `ab24f7` SKW4821, `ab56ee` N83KM, `a388af` DAL2062, `4403bd` IJM572.
- **Still no TIS-B** in this window; a `tisb`-tagged sample remains outstanding.

Use this pair to assert the enrichment join tags these 17 ICAOs as MLAT while
the rest decode as ADS-B.

## Regenerating / adding a capture

See **"Capturing correlated BEAST + NDJSON"** in `infra/k3s-pi/README.md`.
Gzip both files, drop them here with a datestamped name, and add a row above
with sizes + sha256.
