# k3s feeder deployment for `<PI_HOST>`

> **Placeholders in this doc:** `<PI_HOST>`, `<PI_SSH_USER>`. Real values
> live in a gitignored project-local skill — see
> `.claude/skills/homelab-vars.local.md` (create it from
> `.claude/skills/homelab-vars.local.md.example` if missing). Never commit
> the resolved values back into this file.

Replaces the systemd `readsb.service` on the Pi with a k3s-managed
deployment, and adds MLAT correlation via adsb.lol. AirNav RadarBox feeding
is included as a ready-to-apply manifest but deliberately **not** part of
this cutover — see "Adding AirNav RadarBox later" below. The k3s cluster
already exists on this box (`airjedi` node) — this just adds workloads to
it.

## What's here

| File | Purpose |
|---|---|
| `namespace.yaml` | `feeders` namespace |
| `configmap.yaml` | Non-secret config: receiver location, SDR settings, extra readsb args |
| `secret.example.yaml` | Template for UUID/MLAT_USER/RadarBox key — copy to `secret.yaml`, fill in, apply on the Pi. `secret.yaml` is gitignored, never commit it. |
| `readsb-deployment.yaml` | `ultrafeeder` (readsb + mlat-client + multi-aggregator), owns the RTL-SDR |
| `airnavradar-deployment.yaml` | `rbfeeder`, feeds AirNav RadarBox, writes MLAT results back into readsb — **not applied yet, see below** |

## Before applying

1. `configmap.yaml`'s `FEEDER_ALT` is set to `421.24m` (surveyed).
2. `secret.yaml` (gitignored, already created locally) has `UUID` and
   `MLAT_USER` filled in for adsb.lol, and `ULTRAFEEDER_CONFIG` set to feed
   adsb.lol ADS-B + MLAT. `RADARBOX_SHARING_KEY` is left as `REPLACE_ME` —
   harmless since `airnavradar-deployment.yaml` isn't applied in this pass.

## Cutover

Run from a machine with SSH access to the Pi (or directly on the Pi):

```bash
# 1. Apply readsb only, while systemd readsb is still running.
#    Pod will crash-loop on port conflicts until step 2 — expected.
kubectl apply -f namespace.yaml -f configmap.yaml -f secret.yaml \
  -f readsb-deployment.yaml

# 2. Stop and disable the old service (keep the unit file for rollback).
ssh <PI_SSH_USER>@<PI_HOST> "sudo systemctl stop readsb && sudo systemctl disable readsb"

# 3. Confirm the pod comes up.
kubectl -n feeders get pods -o wide
kubectl -n feeders logs deploy/readsb

# 3a. IMPORTANT: confirm the actual readsb command line the container ran
#     matches the intended flags (device/gain/max-range/net-*-port/etc. were
#     passed through READSB_EXTRA_ARGS unverified against this image's
#     entrypoint script — confirm here rather than assume). readsb prints
#     its full invocation in the startup banner in the logs above, and/or:
kubectl -n feeders exec deploy/readsb -- ps aux | grep '[r]eadsb'
#     Compare against the original: --device 0 --device-type rtlsdr
#     --gain -10 --ppm 0 --max-range 450 --write-json-every 1 --net
#     --net-heartbeat 60 --net-ro-size 1250 --net-ro-interval 0.05
#     --net-ri-port 30001 --net-ro-port 30002 --net-sbs-port 30003
#     --net-bi-port 30004,30104 --net-bo-port 30005
#     --json-location-accuracy 2 --range-outline-hours 24
#     If any flag is missing/duplicated/wrong, fix configmap.yaml and
#     `kubectl rollout restart deploy/readsb` before proceeding to step 4.

# 4. Confirm ports are bound by the new pod, not a stray readsb process.
ssh <PI_SSH_USER>@<PI_HOST> "ss -tlnp | grep -E '3000[1-5]|30104'"

# 5. Confirm decode is live.
ssh <PI_SSH_USER>@<PI_HOST> "nc localhost 30003" # should show SBS lines
```

Then verify from AirJedi itself (no app config changes needed — same
`<PI_HOST>:30005`): existing traffic should look identical.

The real end-to-end proof MLAT is working: watch for a previously
position-less Mode-S-only contact (the kind diagnosed earlier — ADS-B-Out
disabled aircraft replying only to secondary radar) to gain a plotted
position in AirJedi once MLAT results start flowing back through the Beast
port. Also check adsb.lol's feeder status page for station `airjedi` showing
online.

## Rollback

```bash
kubectl -n feeders scale deployment readsb --replicas=0
ssh <PI_SSH_USER>@<PI_HOST> "sudo systemctl enable --now readsb"
```

## Adding AirNav RadarBox later

Once you have a RadarBox sharing key: fill in `RADARBOX_SHARING_KEY` in
`secret.yaml`, re-apply it (`kubectl apply -f secret.yaml`), then
`kubectl apply -f airnavradar-deployment.yaml`. Nothing else changes —
`readsb` doesn't need to be touched or restarted.

## Capturing raw I/Q for test fixtures

The RTL-SDR dongle can only be held by one process at a time, so capturing
a raw I/Q dump for `adsb-client` test fixtures means briefly taking the
dongle away from `readsb`. This is disruptive (readsb/MLAT/adsb.lol feed
all go down for the duration), so only do it when needed and keep captures
short.

```bash
# 1. Scale readsb to 0 to release the USB device.
ssh <PI_SSH_USER>@<PI_HOST> "sudo kubectl -n feeders scale deployment readsb --replicas=0"

# 2. Confirm the pod is actually gone (USB release isn't instant).
ssh <PI_SSH_USER>@<PI_HOST> "sudo kubectl -n feeders get pods -l app=readsb"
#    Expect "No resources found". lsusb should still show the RTL2832U,
#    just no longer held by readsb:
ssh <PI_SSH_USER>@<PI_HOST> "lsusb"

# 3. Capture. -f/-s match readsb's own tuning (1090 MHz, 2.4 Msps); -g 49.6
#    is max gain — adjust down if the capture clips. `timeout N` bounds the
#    capture to N seconds regardless of sample-count math.
ssh <PI_SSH_USER>@<PI_HOST> \
  "nohup timeout 300 rtl_sdr -f 1090000000 -s 2400000 -g 49.6 \
    /home/<PI_SSH_USER>/tmp/adsb_capture.bin \
    > /home/<PI_SSH_USER>/tmp/rtl_sdr.log 2>&1 &"

# 4. Wait for it to finish (poll, don't just sleep-and-hope), then restore readsb.
ssh <PI_SSH_USER>@<PI_HOST> "ps aux | grep '[r]tl_sdr' || echo done"
ssh <PI_SSH_USER>@<PI_HOST> "sudo kubectl -n feeders scale deployment readsb --replicas=1"

# 5. Confirm readsb actually came back up and is decoding before moving on
#    — a crash-loop here (e.g. stale USB claim) is easy to miss otherwise.
ssh <PI_SSH_USER>@<PI_HOST> "sudo kubectl -n feeders get pods -o wide"
ssh <PI_SSH_USER>@<PI_HOST> "timeout 3 nc localhost 30003 | head -3"

# 6. Get a checksum on the source before transferring anything, so the
#    transfer itself can be verified byte-for-byte afterward.
ssh <PI_SSH_USER>@<PI_HOST> "sha256sum /home/<PI_SSH_USER>/tmp/adsb_capture.bin"
```

Output format is `rtl_sdr`'s native raw 8-bit unsigned interleaved I/Q
(`I0 Q0 I1 Q1 ...`) — the same format `dump1090 --ifile`-style tools expect.
File size in bytes = duration_seconds * sample_rate * 2 (2 bytes per I/Q pair).

### Transferring the capture (macOS gotchas)

This link (home WAN to the Pi) has been flaky and slow enough that both
`scp` and naive `rsync` failed in practice:

- **`scp` can silently truncate.** It reported exit 0 while having copied
  only a fraction of the file. Always verify the destination's size and
  checksum against the source — never trust a transfer tool's own exit
  code for a large file over an unreliable link.
- **macOS ships `openrsync`, not GNU rsync.** Flags like `--append-verify`
  don't exist in `openrsync` and it fails immediately (not a network
  issue) if you pass them. Check with `rsync --version` — if it prints
  `openrsync: protocol version ...` instead of a normal GNU rsync banner,
  stick to `--partial --append --compress --timeout=N`, which both
  implementations support.
- **Retry with resume, automatically.** A loop of `rsync --partial
  --append --compress` re-invocations resumes from wherever the last
  attempt left off instead of restarting, which matters a lot when the
  connection resets mid-transfer every minute or two:
  ```bash
  DEST=crates/adsb-client/tests/fixtures/adsb_capture.bin
  for attempt in $(seq 1 40); do
    rsync -av --partial --append --compress --timeout=30 --progress \
      <PI_SSH_USER>@<PI_HOST>:/home/<PI_SSH_USER>/tmp/adsb_capture.bin "$DEST"
    [ $? -eq 0 ] && break
    sleep 3
  done
  shasum -a 256 "$DEST"   # compare against step 6's sha256sum output
  ```

### Where fixtures live

Raw captures are too large for git (hundreds of MB to low GB). Drop them
directly in `crates/adsb-client/tests/fixtures/` (gitignored via
`crates/adsb-client/tests/fixtures/*.bin`) and document each one — center
frequency, sample rate, gain, duration, capture date, and sha256 — in
`crates/adsb-client/tests/fixtures/README.md` so they can be verified and
regenerated later.

## Capturing correlated BEAST + NDJSON (ingest fixtures)

Distinct from the raw I/Q capture above. The I/Q capture takes the **dongle**
(readsb must be scaled to 0), so it can never include readsb's NDJSON. This
recipe instead **taps readsb's TCP outputs while readsb keeps running** - it is
non-disruptive (no feed downtime, dongle untouched) and is the only way to get
correlated MLAT/TIS-B source tags, since those are computed by the aggregator
and injected back into the live feed.

These fixtures feed the ingest-simulation test
(`crates/adsb-client/tests/ingest_replay.rs`). Two ports, captured
simultaneously so they join by ICAO + time:

| Port | Stream | readsb flag |
|---|---|---|
| `30005` | BEAST binary (Mode-S frames + 12 MHz receiver timestamps) | `--net-bo-port 30005` |
| `30047` | streaming NDJSON with `type` (`adsb_icao`/`tisb_icao`/`mlat`), `mlat[]`, `tisb[]`, `nic`/`nac_p`, ... | `--net-json-port` |

> **Port 30047 is served by the ultrafeeder image even though it is not in
> `READSB_EXTRA_ARGS`.** No reconfiguration is needed. If a future image stops
> exposing it, add `--net-json-port 30047` to `configmap.yaml`'s
> `READSB_EXTRA_ARGS` and `kubectl rollout restart deploy/readsb`.

Run from any machine that can reach the feeder over the LAN (no SSH needed):

```bash
# 1. Confirm both ports are live and streaming (quick, non-destructive).
#    BEAST should show 0x1a frame markers; NDJSON should show JSON lines.
timeout 3 nc <PI_HOST> 30005 | xxd | head -3
timeout 3 nc <PI_HOST> 30047 | head -2

# 2. GOTCHA: do not rapidly open many probe connections back-to-back. readsb
#    drops/starves connections under rapid churn, which silently truncates a
#    capture to a few KB. Pause a few seconds between probes and the capture.

# 3. Capture ~5 min of BOTH streams at once, into a staging dir.
D=/tmp/ingest_cap; mkdir -p "$D"; cd "$D"
date -u +%Y-%m-%dT%H:%M:%SZ > capture_start.txt
timeout 300 nc <PI_HOST> 30005 > beast_30005.bin     2>/dev/null &  BP=$!
timeout 300 nc <PI_HOST> 30047 > readsb_30047.ndjson 2>/dev/null &  JP=$!

# 4. Health-gate at ~8s: abort and retry if either stream stalled early.
sleep 8
BB=$(wc -c < beast_30005.bin); NL=$(wc -l < readsb_30047.ndjson)
echo "[t=8s] beast=${BB}B ndjson=${NL}L"
if [ "$BB" -lt 2000 ] || [ "$NL" -lt 5 ]; then
  echo "EARLY STALL - kill and retry"; kill "$BP" "$JP" 2>/dev/null; exit 1
fi
wait "$BP"; wait "$JP"
date -u +%Y-%m-%dT%H:%M:%SZ > capture_end.txt

# 5. Inspect: how many aircraft, and did any MLAT/TIS-B targets appear?
grep -o '"type":"[a-z_]*"' readsb_30047.ndjson | sort | uniq -c | sort -rn
echo "distinct aircraft: $(grep -o '"hex":"[a-f0-9]*"' readsb_30047.ndjson | sort -u | wc -l)"
echo "populated mlat[]: $(grep -c '"mlat":\[[0-9]' readsb_30047.ndjson)  tisb[]: $(grep -c '"tisb":\[[0-9]' readsb_30047.ndjson)"
```

> **MLAT/TIS-B are opportunistic.** They only appear for Mode-S-only aircraft
> being multilaterated (or TIS-B ground uplink). A given 5-min window may be
> pure `adsb_icao` (the 2026-09-02 fixture was). To catch a real MLAT sample,
> capture when step 5 shows non-empty `mlat[]` / a `"type":"mlat"` line - retry
> at a busier time or once RadarBox MLAT writeback is active (see "Adding AirNav
> RadarBox later").

Then install the fixtures (gzipped, committable - unlike the I/Q `.bin`):

```bash
DEST=crates/adsb-client/tests/fixtures/ingest   # in the repo
gzip -c beast_30005.bin     > "$DEST/beast_30005_$(date +%Y%m%d).bin.gz"
gzip -c readsb_30047.ndjson > "$DEST/readsb_30047_$(date +%Y%m%d).ndjson.gz"
shasum -a 256 "$DEST"/*.gz   # record in that dir's README.md
```

Document each capture (sizes, sha256, source-type breakdown, aircraft count) in
`crates/adsb-client/tests/fixtures/ingest/README.md` - kept separate from the
raw I/Q fixtures README.

## AirJedi fusion agent

`airjedi-agent-deployment.yaml` runs the headless design-b fusion agent on the
`airjedi` node: it ingests readsb's BEAST output (`localhost:30005`), runs the
multi-sensor fusion + display projection, and replicates render-ready
`DisplayTrack`s over renet/UDP `5599` to thin clients (the desktop app built
`--features thin-client`). `hostNetwork` means it reaches readsb locally and is
reachable at `<PI_HOST>:5599` with no port-forward.

### Build the image (arm64 - the node is arm64)

From the repo root:

```bash
docker buildx build --platform linux/arm64 \
  -f airjedi-agent/Dockerfile -t airjedi-agent:0.1.2 --load .
```

The build compiles only the agent's dependency graph (no desktop app / egui /
libgit2), so it stays lean.

### Get it onto the node

**Option A - local import (no registry).** The image is a few hundred MB; over
the flaky home WAN prefer a LAN transfer if you have one, and always verify the
transfer (see the scp/rsync caveats under "Transferring the capture" above).

```bash
docker save airjedi-agent:0.1.2 | gzip > /tmp/airjedi-agent.tar.gz
scp /tmp/airjedi-agent.tar.gz <PI_SSH_USER>@<PI_HOST>:/tmp/
ssh <PI_SSH_USER>@<PI_HOST> \
  "gunzip -c /tmp/airjedi-agent.tar.gz | sudo k3s ctr images import -"
```

The manifest uses `image: airjedi-agent:0.1.2` with `imagePullPolicy:
IfNotPresent`, so k3s uses the imported image without trying to pull.

**Option B - registry (avoids the WAN transfer).** Push to a registry the node
can pull from, and set `image:` in the manifest accordingly:

```bash
docker buildx build --platform linux/arm64 \
  -f airjedi-agent/Dockerfile -t ghcr.io/<owner>/airjedi-agent:0.1.2 --push .
# then edit airjedi-agent-deployment.yaml: image: ghcr.io/<owner>/airjedi-agent:0.1.2
# (add an imagePullSecret if the package is private)
```

### Apply and verify

```bash
kubectl apply -f airjedi-agent-deployment.yaml
kubectl -n feeders rollout status deploy/airjedi-agent
# Expect: "live ingest connected to localhost:30005" and "listening ... udp/5599"
kubectl -n feeders logs deploy/airjedi-agent

# The UDP port is bound on the host (hostNetwork):
ssh <PI_SSH_USER>@<PI_HOST> "ss -ulnp | grep 5599"

# End-to-end from any machine on the LAN, using the built-in probe client
# (same replicon transport the app uses):
cargo run -p airjedi-agent -- --probe --connect <PI_HOST>:5599 --secs 8
# Expect a growing DisplayTrack count.
```

### Point the desktop thin client at it

Edit the installed bundle's agent address (no rebuild needed):

```bash
/usr/libexec/PlistBuddy -c "Set :LSEnvironment:AIRJEDI_THIN_AGENT <PI_HOST>:5599" \
  /Applications/AirJedi.app/Contents/Info.plist
```

or launch with `AIRJEDI_THIN_AGENT=<PI_HOST>:5599 open -a AirJedi`. On a code
change, bump the tag (`0.1.3`, ...), rebuild/import, update the manifest image,
and `kubectl -n feeders rollout restart deploy/airjedi-agent`.

### Diagnostics and troubleshooting

The agent emits diagnostic lines every 30 seconds. Monitor these fields when
investigating memory growth:

- `rss_bytes`: Linux process RSS.
- `position_history`: retained aircraft position samples.
- `stored_observations`: fusion timeline observations, retained for 60 seconds.
- `tracks` and `display_entities`: active fused and replicated tracks.

The deployment enables `RUST_BACKTRACE=1` and
`terminationMessagePolicy: FallbackToLogsOnError`, so panic backtraces and the
tail of a failed container's output remain available in pod status even when
the container is restarted.

The live ingest tracker evicts stale aircraft every five seconds and retains
position history for 120 seconds. The agent deployment also sets
`AIRJEDI_AGENT_PUBLIC_ADDR` so netcode advertises the LAN address rather than
loopback.

The client transport must bind its UDP socket to `0.0.0.0:0`; binding to
`127.0.0.1:0` prevents replies from a remote agent from reaching the client.
The corresponding server and client fixes are included in `airjedi-agent:0.1.2`.

The observed steady-state baseline on `airjedi.custine.com` is approximately
`0.4` CPU cores, `43Mi` Kubernetes memory, and `55.8MiB` process RSS, with no
restarts. Track and observation counts vary with local traffic.

### Persistent crash logs

The device's default `/var/log` is a small `tmpfs`, so its journal and k3s
container logs disappear after a reboot. Install `airjedi-journald.conf` into
the device's journald drop-in directory, remove the volatile `/var/log` mount,
and let `/var/log` use the persistent NVMe-backed root filesystem:

```bash
ssh <PI_SSH_USER>@<PI_HOST> 'sudo mkdir -p /etc/systemd/journald.conf.d'
scp infra/k3s-pi/airjedi-journald.conf <PI_SSH_USER>@<PI_HOST>:/tmp/airjedi-journald.conf
ssh <PI_SSH_USER>@<PI_HOST> \
  'sudo install -o root -g systemd-journal -m 0644 /tmp/airjedi-journald.conf /etc/systemd/journald.conf.d/airjedi.conf && \
   sudo cp -a /etc/fstab /etc/fstab.airjedi-before-persistent-logs && \
   sudo sed -i "\\|^tmpfs /var/log tmpfs |s|^|# AirJedi persistent logging: |" /etc/fstab && \
   sudo systemctl daemon-reload'
```

Reboot once to apply the `/var/log` mount change. The backup at
`/etc/fstab.airjedi-before-persistent-logs` allows the mount change to be
reversed if needed. After the reboot, verify journald is persistent and the
k3s container log paths are on the root filesystem:

```bash
ssh <PI_SSH_USER>@<PI_HOST> \
  'mount | grep " /var/log " && \
   sudo journalctl --flush && \
   sudo journalctl --disk-usage && \
   sudo find /var/log/pods -type f -maxdepth 3 -printf "%p %s bytes\\n"'
```

After installation, collect the previous boot and workload evidence with:

```bash
ssh <PI_SSH_USER>@<PI_HOST> \
  'sudo journalctl --list-boots; \
   sudo journalctl -b -1 -k --no-pager; \
   sudo journalctl -b -1 -u k3s --no-pager; \
   sudo k3s kubectl -n feeders describe pod -l app=airjedi-agent; \
   sudo k3s kubectl -n feeders logs deploy/airjedi-agent --previous --timestamps'
```

## Notes

- Both deployments use `hostNetwork: true` and are pinned to node `airjedi`
  via `nodeSelector`, so every port matches the old systemd setup exactly —
  no router/port-forward changes required.
- The `readsb` container runs `privileged: true` for raw USB access to the
  RTL-SDR. The dongle already has a permissive udev rule
  (`MODE="0666"` in `/etc/udev/rules.d/20-rtlsdr.rules` and `rtl-sdr.rules`),
  so a non-privileged variant may be possible later if a tighter security
  posture is wanted — not attempted here since it's a single trusted
  homelab node.
- The `dietpi` node in this k3s cluster is `NotReady` and unused by this
  deployment; not addressed here.
- The disabled `airjedi-sensor.service` (the custom FutureSDR-based decoder)
  is untouched — out of scope for this change.
