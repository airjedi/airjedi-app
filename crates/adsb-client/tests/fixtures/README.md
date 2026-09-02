# Raw I/Q capture fixtures

Large binary fixtures are gitignored (`*.bin`, see repo `.gitignore`) and must
be obtained separately - they are not stored in git history.

## adsb_1090_2p4msps_iq_20260902.bin

Raw RTL-SDR I/Q capture for exercising ADS-B decode paths against real
Mode S traffic instead of synthetic frames.

- **Center frequency:** 1090 MHz
- **Sample rate:** 2.4 Msps
- **Gain:** 49.6 dB
- **Format:** raw 8-bit unsigned interleaved I/Q (`rtl_sdr`'s native output
  format - `I0 Q0 I1 Q1 ...`, one byte per sample component). Compatible
  with `dump1090 --ifile`-style consumers and any tool that reads
  `rtl_sdr`-format capture files directly.
- **Duration:** ~299.4s actual (300s requested; file is
  1,437,335,552 bytes = 718,667,776 I/Q sample pairs at 2.4 Msps * 2 bytes/sample)
- **Size:** 1,437,335,552 bytes
- **SHA-256:** `413ba9cb725c12b85e0117baef1723e5aaf0ab49b5ddf621982a8ca699d77947`
- **Captured:** 2026-09-02, from the RTL-SDR dongle attached to the
  `airjedi` k3s node (`airjedi.custine.com`). The `readsb` Deployment
  (namespace `feeders`, see `infra/k3s-pi/`) was scaled to 0 replicas to
  release exclusive USB access to the dongle for the duration of the
  capture, then scaled back to 1 replica and confirmed healthy/decoding
  live SBS traffic afterward.
- **Command:**
  ```
  rtl_sdr -f 1090000000 -s 2400000 -g 49.6 adsb_1090_2p4msps_iq_capture.bin
  ```

Verify integrity after copying with:
```
shasum -a 256 adsb_1090_2p4msps_iq_20260902.bin
```
