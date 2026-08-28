# k3s feeder deployment for airjedi.custine.com

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
ssh ccustine@airjedi.custine.com "sudo systemctl stop readsb && sudo systemctl disable readsb"

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
ssh ccustine@airjedi.custine.com "ss -tlnp | grep -E '3000[1-5]|30104'"

# 5. Confirm decode is live.
ssh ccustine@airjedi.custine.com "nc localhost 30003" # should show SBS lines
```

Then verify from AirJedi itself (no app config changes needed — same
`airjedi.custine.com:30005`): existing traffic should look identical.

The real end-to-end proof MLAT is working: watch for a previously
position-less Mode-S-only contact (the kind diagnosed earlier — ADS-B-Out
disabled aircraft replying only to secondary radar) to gain a plotted
position in AirJedi once MLAT results start flowing back through the Beast
port. Also check adsb.lol's feeder status page for station `airjedi` showing
online.

## Rollback

```bash
kubectl -n feeders scale deployment readsb --replicas=0
ssh ccustine@airjedi.custine.com "sudo systemctl enable --now readsb"
```

## Adding AirNav RadarBox later

Once you have a RadarBox sharing key: fill in `RADARBOX_SHARING_KEY` in
`secret.yaml`, re-apply it (`kubectl apply -f secret.yaml`), then
`kubectl apply -f airnavradar-deployment.yaml`. Nothing else changes —
`readsb` doesn't need to be touched or restarted.

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
