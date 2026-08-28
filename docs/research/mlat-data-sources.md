# MLAT Data Source Research — AirJedi

> Research date: 2026-08-27
> Scope: Public ADS-B aggregator networks that provide multilateration (MLAT) derived
> position data, and how a Rust hobbyist desktop app (AirJedi) could integrate one.

## Background / Problem Statement

AirJedi decodes ADS-B/Mode-S directly from a local Beast-protocol receiver
(`adsb-client` crate) fed by an RTL-SDR / readsb-style instance. It has no
multilateration capability. Aircraft that only reply to secondary radar interrogation
with DF=4/5/11/20/21 (no lat/lon payload — common for military aircraft with ADS-B Out
disabled) never get a position and are invisible in AirJedi today. MLAT — triangulating
position from time-difference-of-arrival (TDOA) of the same Mode-S reply heard by
multiple, precisely time-synchronized nearby receivers — is how community networks
(ADSBExchange, airplanes.live, adsb.lol) and commercial networks (FlightAware, FR24)
recover positions for these targets.

This document is a source-by-source primary-source audit of the public options, plus
the mechanics of `mlat-client`, the tool that actually performs the client-side
multilateration correlation.

---

## 1. ADS-B Exchange (adsbexchange.com)

**Computes MLAT?** Yes. The v2 API's `type` field enumerates a `mlat` value:
"mlat: MLAT, position calculated [via] arrival time differences using multiple
receivers, outliers and varying ac[curacy]" compared to true ADS-B — confirmed on the
[Version 2 API Fields page](https://www.adsbexchange.com/version-2-api/). ADS-B
Exchange describes itself as "the world's largest community of unfiltered ADS-B/Mode
S/MLAT feeders."

**Access method:**
- **API (read-only):** `aircraft.json`-style v2 API, documented at
  [gateway.adsbexchange.com/api/aircraft/v2/docs](https://gateway.adsbexchange.com/api/aircraft/v2/docs/swagger/index.html?url=%2Fapi%2Faircraft%2Fv2%2Fdocs%2Fopenapi.json)
  and resold via [RapidAPI](https://rapidapi.com/adsbx/api/adsbexchange-com1). Each
  aircraft record's `type` field distinguishes `mlat` from `adsb_icao`, `tisb_icao`,
  `adsc`, `mode_s`, `other`, etc. `rr_lat`/`rr_lon` provide a rough
  receiver-location fallback when neither ADS-B nor MLAT position is available.
- **Feeder client (upload path):** [`ADSBexchange/feedclient`](https://github.com/ADSBexchange/feedclient)
  is the official install script for feeding an existing readsb/dump1090/PiAware
  install to ADS-B Exchange (`curl -L -o /tmp/axfeed.sh https://adsbexchange.com/feed.sh
  && sudo bash /tmp/axfeed.sh`). It configures MLAT automatically and generates a UUID
  (`create-uuid.sh`) during setup. Per the docs, once configured, MLAT results directed
  back to the local decoder land on **127.0.0.1:30104** (`RESULTS="--results
  beast,connect,127.0.0.1:30104"`, disabled by setting `RESULTS=""`).
- The underlying MLAT client/server are forks of the original `mutability/mlat-client`:
  [`adsbxchange/mlat-client`](https://github.com/ADSBexchange/mlat-client) (client) and
  [`adsbxchange/mlat-server`](https://github.com/adsbxchange/mlat-server) (server). The
  mlat-server README states you need "a bunch of receivers running mlat-client;" PiAware
  installs auto-detect `fa-mlat-client` on `$PATH`.

**Data format:** v2 API JSON aircraft objects (`type: "mlat"`); locally, Beast-format
synced results via mlat-client's `--results beast,...` on 30104 (see §7 for the general
mlat-client wiring, which ADS-B Exchange's fork follows).

**Auth / rate limits / ToS:**
- The RapidAPI product is **paid only today** — one plan documented at "$10 USD per
  month, 10,000 requests," per third-party integrator docs (LiveTraffic/X-Plane
  plugin author, citing the RapidAPI listing directly). ADS-B Exchange's "ADSBx Flight
  Sim Traffic API" free/legacy RapidAPI product was **discontinued 1-Mar-2025** per the
  same source.
  ([twinfan.gitbook.io/livetraffic — ADS-B Exchange setup page](https://twinfan.gitbook.io/livetraffic/setup/installation/ads-b-exchange))
- The [Data Products page](https://www.adsbexchange.com/data-products/) describes
  enterprise-tier products as "ongoing subscription services with minimum annual
  commitments," with an "Acceptable Use Policy" linked but not itself fetched here.
  No one-time/project-based access is offered; "historical backfills … available to
  subscription customers only."
- Feeding is still culturally/functionally expected ("all API users are asked to set
  up a feeder" per community guides), but there is **no current officially-documented
  free tier gated purely on feeding** — the earlier "feed and get a free key" model
  has been phased out on the commercial RapidAPI product (confirmed by multiple
  concordant secondary sources referencing the RapidAPI listing's current terms; the
  Developer Hub / API-lite page at adsbexchange.com/api-lite/ was not independently
  re-verified for a hobbyist-tier carve-out and should be checked directly before
  relying on it).

**Must you be a feeder for local MLAT coverage?** Yes in effect — MLAT triangulation
needs multiple independent nearby receivers hearing the same aircraft; the value of
"your own local airspace" MLAT coverage from any of these networks is a function of how
many other feeders are near you, not just whether you personally feed. Feeding your own
receiver is necessary (to contribute geometry) but not sufficient (other nearby feeders
must also exist) for good local MLAT coverage.

**Reference implementation:** feedclient wires the same `mlat-client` binary from the
`adsbxchange/mlat-client` fork; PiAware's `fa-mlat-client` (a similarly-derived fork,
see §5) is the most thoroughly documented consumer wiring (see §7).

---

## 2. airplanes.live

**Computes MLAT?** Yes — same architecture as adsb.lol/ADSBX; example live API
response objects explicitly carry an `mlat` array field (empty list when not
MLAT-derived) alongside `tisb`, matching the ADSBExchange v2 schema (see
[airplanes-live/api README](https://github.com/airplanes-live/api/blob/main/README.md)):
```json
{"hex":"a9cee9","type":"adsb_icao","flight":"N731BP ","alt_baro":38000, ...
 "mlat":[], "tisb":[], "messages":24844, "seen":0.7, "rssi":-15.8}
```
The README states: "all responses conform to the ADSBExchange v2 API" — so the `type`
field's `mlat` value and its semantics are identical to ADS-B Exchange's (§1).

**Access method:**
- **API (read-only):** `https://api.airplanes.live/v2/point/<lat>/<lon>/<radiusKM>`,
  `/v2/icao/<hex>`, `/v2/callsign/<callsign>` per the
  [airplanes-live/api README](https://github.com/airplanes-live/api/blob/main/README.md).
  **No API key required**, and feeding is **not currently required** to query it
  ("Access does not currently require a feeder, though that might change in the
  future" — per README/community documentation). Shortest documented poll interval
  is 1 second.
- **Feeder client (upload path):** [`airplanes-live/feed`](https://github.com/airplanes-live/feed)
  — one-line install (`curl -L -o /tmp/feed.sh
  https://raw.githubusercontent.com/airplanes-live/feed/main/install.sh && sudo bash
  /tmp/feed.sh`), requires a decoder (readsb/dump1090-fa) already running. Managed via
  systemd units `airplanes-feed` and `airplanes-mlat`; config lives in
  `/etc/default/airplanes`. To get local MLAT-synced Beast results back:
  `sudo sed --follow-symlinks -i -e 's/RESULTS=.*/RESULTS="--results
  beast,connect,127.0.0.1:30104"/' /etc/default/airplanes` then restart both services
  (per [airplanes.live how-to-feed](https://airplanes.live/how-to-feed/) /
  [get-started](https://airplanes.live/get-started/) docs surfaced via search — verify
  wording directly on those pages before relying on the exact sed command, as it may
  drift).
- Underlying client is based on `airnavsystems/mlat-client`, itself another fork
  lineage of `mutability/mlat-client`.

**Data format:** Same ADSBExchange-v2-compatible JSON for the API; local Beast-format
sync results via the `--results beast,connect,...` mechanism identical in shape to §1/§7.

**Auth / rate limits / ToS:** No key currently required for the public API. However,
third-party integrations explicitly flag a **non-commercial restriction**: one MCP
server project states "This project uses the airplanes.live API which is provided for
educational and non-commercial purposes only. Please respect their terms of service,"
pointing at [airplanes.live/api-guide/](https://airplanes.live/api-guide/) (this page
returned HTTP 403 to automated fetch during this research — its exact legal text should
be read manually in a browser before committing to it for AirJedi, a *distributed*
desktop app, since distribution to third parties raises the stakes on "non-commercial /
educational only" wording). airplanes.live's [About page](https://airplanes.live/about/)
frames the project as enthusiast-run and commits contractually to not selling equity or
transferring ownership — a favorable signal for longevity, not a substitute for reading
the ToS.

**Must you be a feeder for local coverage?** Not required to *read* the API, but same
physical-triangulation caveat as §1 applies: local MLAT quality depends on feeder
density near you, not on your own read-access method.

---

## 3. adsb.lol

**Computes MLAT?** Yes — by design, feeding defaults to sending both ADS-B and MLAT
raw data: "by default, it feeds MLAT+ADSB to adsb.lol" per the
[adsblol/feed README](https://github.com/adsblol/feed). Backend infra
([adsblol org repos](https://github.com/orgs/adsblol/repositories)) explicitly includes
an `mlat-server` component alongside `readsb`, `tar1090`, and the API.

**Access method:**
- **API (read-only):** `https://api.adsb.lol`, documented interactively at
  [api.adsb.lol/docs](https://api.adsb.lol/docs). Per the
  [adsblol/api README](https://github.com/adsblol/api): "This API is compatible with
  the ADSBExchange Rapid API and is a drop-in replacement" — i.e., same `type: "mlat"`
  schema semantics as §1/§2. **No API key today**; the README states "in the future you
  will require an API key which you can obtain by feeding adsb.lol" — i.e. a
  feeder-gated key is *planned*, not yet enforced. Rate limits are described as
  "dynamic based on environment load."
- **Feeder client (upload path):** [`adsblol/feed`](https://github.com/adsblol/feed) —
  container-based multi-feed client; also feeds FlightAware, FR24, RadarBox, etc.
  simultaneously if desired. Sample direct `mlat-client` invocation shown in the repo:
  ```
  /usr/bin/python3.9 /usr/bin/mlat-client \
    --user yourname --lat 00.00000 --lon -00.00000 --alt 231m \
    --input-type dump1090 --input-connect localhost:30005 \
    --server feed.adsb.lol:31090 \
    --results beast,listen,32005 --uuid=UUID --privacy
  ```
  This is a useful concrete illustration of the generic mlat-client argument shape
  (see §7).
- **Bulk MLAT re-consumption (feeders-only, explicitly unsupported/experimental):** per
  [adsb.lol "BEAST MLAT Out"](https://www.adsb.lol/docs/feeders-only/beast-mlat-out/),
  active feeders can pull the *aggregated network* MLAT+Beast stream back out via
  `out.adsb.lol:1365` (Beast) and `:1366` (MLAT/SBS), e.g.:
  `readsb --net-only --net-connector=out.adsb.lol,1365,beast_in
  --net-connector=out.adsb.lol,1366,sbs_in_mlat`. The docs explicitly warn: **"Do not
  use your feeder for this. You need to spin up a new readsb,"** and: **"This feature is
  experimental. In particular, no support or guarantee of any kind is provided."**
  Access is gated on "must be an active feeder."
- A public MLAT sync visualization exists at `mlat.adsb.lol`
  ([MLAT Map docs](https://www.adsb.lol/docs/feeders-only/mlat-map/)), which "respects
  the privacy flag" so `--privacy` feeders are excluded from the visible graph.

**Data format:** ADSBExchange-v2-compatible JSON via the API; raw Beast/SBS sync
streams via the feeders-only bulk-out ports.

**Auth / rate limits / ToS:** Per
[adsb.lol Open Data / API docs](https://www.adsb.lol/docs/open-data/api/), the API and
underlying data are **licensed under ODbL 1.0** (Open Database License) — an
attribution-required, share-alike-style open license, which is explicit and
machine-checkable, unlike the vaguer "non-commercial" wording seen at ADSBX/airplanes.live.
No API key currently enforced; dynamic rate limiting only. This is the
most redistribution-friendly ToS posture of the three community networks surveyed.

**Must you be a feeder for local coverage?** Not required for the basic read API today
(same TDOA-density caveat as always for local coverage quality). Required for the
feeders-only bulk MLAT-out re-ingestion path.

---

## 4. OpenSky Network (opensky-network.org)

**Computes MLAT?** Yes, currently — confirmed by two independent parts of the current
docs:
1. The [REST API](https://openskynetwork.github.io/opensky-api/rest.html) state-vector
   schema has a `position_source` field: "origin of this state's position: 0 = ADS-B,
   1 = ASTERIX, 2 = MLAT, 3 = FLARM."
2. The [Trino historical-data docs](https://openskynetwork.github.io/opensky-api/trino.html)
   describe a dedicated MLAT table: "This table contains multilateration (MLAT) state
   vectors collected via mlat-client, mlat-server, and Readsb. MLAT uses
   time-difference-of-arrival measurements from multiple ground receivers to locate
   aircraft that do not transmit ADS-B position messages. Coverage depends on the
   density of participating receivers."

Note: an older cached [FAQ](https://opensky-network.org/about/faq) reportedly states "As
we currently do not offer multilateration (MLAT)" — that page returned HTTP 403 to
automated fetch during this research and could not be directly re-verified; treat it as
**stale relative to the current REST/Trino docs**, which are unambiguous about MLAT
support. Confirm directly in-browser before depending on either statement.

**Access method:** Pure read-only REST API — no feeder-client-based local re-injection
path is documented; OpenSky is oriented at research/data-science consumption
(state vectors, flights, tracks), not at improving your own local live picture. There is
a `sensors` filter parameter to query state vectors "as seen by given sensor(s)" if you
run an OpenSky sensor, and an authenticated `/states/own` endpoint for your own
sensor's un-rate-limited feed, but no equivalent of a "beast,connect,127.0.0.1:NNNN"
local sync-back mechanism the way mlat-client/readsb/PiAware provide.

**Data format:** State-vector JSON arrays over REST
([rest.html](https://openskynetwork.github.io/opensky-api/rest.html)); 18 indexed
fields per vector (`icao24, callsign, origin_country, time_position, last_contact,
longitude, latitude, baro_altitude, on_ground, velocity, true_track, vertical_rate,
sensors, geo_altitude, squawk, spi, position_source, category`).

**Auth / rate limits:**
- Anonymous: current-only state vectors (`time` param ignored, snapped to 10s
  resolution), **400 daily credits**.
- Authenticated (OAuth2 client-credentials, Bearer token, 30-min expiry — "Basic
  authentication with username and password is no longer accepted"): up to 1 hour of
  history, 5s resolution, **4,000 daily credits** (8,000-14,400 on higher tiers).
- `/states/own` (your own sensor's data) is documented as **not rate-limited**, but
  requires authentication.

**Must you be a feeder to get local MLAT coverage benefit?** OpenSky's MLAT coverage is
a research-network artifact of wherever OpenSky's own volunteer sensor network is dense
— becoming an OpenSky sensor host does feed into that network, but there's no
documented local re-sync path analogous to mlat-client's `--results` output, so even as
a feeder you would not get a low-latency local MLAT feed back into your own pipeline —
you'd have to poll the REST API for your own sensor's state vectors, at whatever
freshness/latency OpenSky's backend provides (not documented as real-time-guaranteed).

**Assessment:** Best suited for research/backfill use, weakest fit for a live desktop
tracker wanting sub-second local MLAT for a specific home region.

---

## 5. FlightAware (PiAware / Firehose / AeroAPI)

**Computes MLAT?** Yes, and MLAT is one of several fused sources inside FlightAware's
proprietary HyperFeed engine: "FlightAware's Firehose ADS-B data feed is powered by
HyperFeed, a machine learning and rules engine that … fus[es] data from terrestrial
ADS-B, radar, Mode S multilateration (MLAT), datalink, and Aireon space-based ADS-B" —
[Firehose product page](https://www.flightaware.com/commercial/firehose/). FlightAware's
own blog explains the mechanism generically: "Because data needs to be received by at
least 4 different sites to perform the MLAT calculations, many receivers are needed in
a given area" ([blog.flightaware.com/what-is-mlat](https://blog.flightaware.com/what-is-mlat)).

**Access method:**
- **Feeder client (upload path) — the free/hobbyist-relevant one:** PiAware bundles
  `fa-mlat-client` (a FlightAware-maintained fork in the same
  `mutability/mlat-client` lineage). Per FlightAware's own
  [PiAware Advanced Configuration docs](https://www.flightaware.com/adsb/piaware/advanced_configuration)
  and corroborating community/forum documentation, the wiring is:
  - `fa-mlat-client` runs as: `--input-connect localhost:30005 --input-type dump1090
    --results beast,connect,localhost:30104 --results beast,listen,30105 --results
    ext_basestation,listen,30106 --udp-transport`
  - Port **30104**: results connected *back into* the local decoder (dump1090-fa) so
    the local web map/pipeline sees synthesized MLAT tracks merged with ADS-B — this is
    the local re-integration path that matters for AirJedi.
  - Port **30105**: Beast-format MLAT results, listen mode, for external consumers
    (e.g. Virtual Radar Server).
  - Port **30106**: Extended-Basestation-format MLAT results, listen mode.
  - `mlat-results-format` is the PiAware config knob controlling this list, format
    `<protocol>,connect,host:port` or `<protocol>,listen,port`.
  This is the **most concretely documented local-loopback wiring of any network
  surveyed** and is architecturally identical to what an AirJedi integration would need
  to replicate (see §7 — general pattern).
- **Read APIs (paid, enterprise):** Firehose (streaming, JSON Lines over TCP/TLS) and
  AeroAPI (on-demand REST, 2-week history) are FlightAware's commercial products; access
  to the MLAT data *layer* specifically is subscription-gated: "your subscription may
  consist of any combination of data layers… ask a FlightAware representative if you
  would like access to additional data layers," and "pricing is based on a monthly
  rate… dependent on what data layers you choose to access and the scope of how you
  repurpose/redistribute the data within your application"
  ([Firehose documentation](https://www.flightaware.com/commercial/firehose/documentation)).
  A free/limited trial exists for historical data only; full real-time trial requires
  contacting FlightAware directly.

**Data format:** Local: Beast binary (30104/30105) or Extended-Basestation text
(30106) — both standard formats already speakable by readsb/dump1090-fa/AirJedi's Beast
parser lineage. Firehose: JSON Lines, UTF-8, newline-delimited.

**Auth / ToS for embedding in a distributed desktop app:** Not published in
self-serve form — "specific licensing terms, pricing details, or contractual language
regarding MLAT data redistribution rights… detailed terms are typically customized per
account," i.e. you'd need a direct commercial negotiation with FlightAware to legally
ship their MLAT data inside a redistributed third-party app. **Not a fit for a
freely-distributed hobbyist app** via the API route. The PiAware feeder-client route,
by contrast, only returns MLAT results for *your own local receiver's* traffic and does
not implicate FlightAware's redistribution terms in the same way (you're only getting
back a computed position for message data you contributed) — but note `fa-mlat-client`
is FlightAware's software and its own license/ToS should be checked before bundling it.

**Must you be a feeder for local coverage?** Yes for the free path — the only
no-cost, real-time way to get FlightAware-computed MLAT results for your own area is to
run PiAware + `fa-mlat-client` and consume the 30104/30105/30106 loopback locally, which
inherently requires you to be an active feeder within reach of enough other FlightAware
feeders for the ≥4-site TDOA requirement.

---

## 6. Flightradar24

**Computes MLAT?** FR24 is well known in the community to derive positions from
multiple ground stations (historically marketed as its own "multilateration" claims),
but this research found **no primary-source FR24 document that names an `mlat` field or
publishes an MLAT methodology** the way ADSBX/airplanes.live/adsb.lol/OpenSky/FlightAware
do. The official [FR24 API](https://fr24api.flightradar24.com/) and
[Getting Started docs](https://fr24api.flightradar24.com/docs/getting-started) advertise
"real-time flight data… powered by the largest independent ADS-B surveillance network in
the world" without a documented per-record source/type flag distinguishing MLAT
from ADS-B in the public API schema (not verified beyond the marketing/overview pages
fetched here — the full field-level API reference was not exhaustively reviewed and
should be checked directly if FR24 is pursued further).

**Access method:** Official REST API only (no raw feeder-client re-sync/loopback
mechanism is documented akin to mlat-client/PiAware). Access requires an
FR24.com account, subscribing to a paid plan ("Explorer," "Essential," "Advanced") or
using a free "Sandbox" for testing, then generating an API key from a key-management
page — per [FR24 API Overview](https://fr24api.flightradar24.com/) /
[Getting Started](https://fr24api.flightradar24.com/docs/getting-started).
**Confirmed: regular FR24 Premium subscriptions do NOT include API access** — it's a
wholly separate paid product line.

**ToS (from flightradar24.com/terms-of-service, per search-indexed summary — read
directly before relying on this):**
- **Flight-critical/safety-critical use prohibited.**
- **No competitive harm** — explicitly bars using API data to "develop competing
  services/products based on the data" or remove FR24 trademarks/notices.
- **Storage limits** — "storing the data beyond restrictions specified in the API
  documentation is prohibited, including creating persistent local copies."
- Commercial use of the *non-API* Premium product additionally requires the
  "Business" tier.

**Assessment:** Confirmed mostly closed/commercial, exactly as the task anticipated.
The "no persistent local copies" storage restriction is particularly hostile to a
desktop tracker's typical use pattern (caching recent positions locally for trails,
history, replay) and there is no MLAT-specific field or feeder-loopback path documented
at all. **Not recommended to pursue further for AirJedi** without a direct
paid-tier legal review.

---

## 7. `mlat-client` — the shared underlying protocol/tool

All community networks above (ADSBX, airplanes.live, adsb.lol) and FlightAware's
PiAware distribute **forks of the same original client**,
[`mutability/mlat-client`](https://github.com/mutability/mlat-client) (original author
"mutability," also the original `mlat-server` author). Confirmed fork lineage found in
this research:

| Fork | Used by |
|---|---|
| [`ADSBexchange/mlat-client`](https://github.com/ADSBexchange/mlat-client) | ADS-B Exchange |
| `airnavsystems/mlat-client` | airplanes.live |
| `fa-mlat-client` (FlightAware-maintained) | PiAware |
| [`wiedehopf/mlat-client`](https://github.com/wiedehopf/mlat-client) | adsb.lol infra / generic modern fork, actively maintained |

**Confirmed CLI arguments** (from `wiedehopf/mlat-client`'s `mlat/client/options.py`,
read directly):
- `--input-type {auto, dump1090, beast, radarcape_12mhz, radarcape_gps, radarcape, sbs,
  avrmlat}` — default `dump1090`.
- `--input-connect host:port` — **default `localhost:30005`** (i.e. the exact port
  AirJedi's own Beast connection already targets, per
  `crates/adsb-client/examples/df_diag.rs`'s default of
  `airjedi.custine.com:30005`). Required argument.
- `--results <protocol>,<mode>,<address>` — repeatable. `<protocol>` ∈
  `{basestation, ext_basestation, beast}`; `<mode>` ∈ `{listen, connect}`; `<address>`
  is `host:port` for `connect` or a bare port number for `listen`.
- `--no-anon-results` / `--no-modeac-results` — flags to suppress anonymized-aircraft
  or Mode-A/C-only results.
- (Not present in the fetched excerpt of `options.py` but confirmed via the
  `adsblol/feed` sample invocation: `--server host:port` for the upstream MLAT
  correlation server, `--lat/--lon/--alt` for your receiver's precise surveyed
  position — required for TDOA math — `--user <name>` and `--uuid <uuid>` for feeder
  identification, and `--privacy` to opt out of public MLAT-map visualization.)

**Concrete example (adsb.lol, directly from
[adsblol/feed](https://github.com/adsblol/feed)):**
```
/usr/bin/python3.9 /usr/bin/mlat-client \
  --user yourname --lat 00.00000 --lon -00.00000 --alt 231m \
  --input-type dump1090 --input-connect localhost:30005 \
  --server feed.adsb.lol:31090 \
  --results beast,listen,32005 --uuid=UUID --privacy
```

**Concrete example (PiAware/`fa-mlat-client`, from FlightAware advanced-config +
corroborating forum/log evidence):**
```
fa-mlat-client \
  --input-connect localhost:30005 --input-type dump1090 \
  --results beast,connect,localhost:30104 \
  --results beast,listen,30105 \
  --results ext_basestation,listen,30106 \
  --udp-transport
```

**Local output format:** `--results beast,...` emits **standard Beast binary frames**
(the same wire format AirJedi's `adsb-client` crate already parses for live ADS-B) —
these are synthetic/synced Beast messages representing the MLAT-computed position, not
raw Mode-S replies. `ext_basestation`/`basestation` emit text-based SBS-1-style lines
instead. **No custom binary protocol** — this is good news for integration effort,
since AirJedi's existing Beast decoder is the natural consumer.

**Default/documented ports observed across networks:** There is no single universal
default port — each network's feed script picks its own (PiAware: 30104/30105/30106;
adsb.lol sample: an arbitrary 32005; ADS-B Exchange feedclient: 30104 for the
loopback-to-dump1090 case). The `--results` argument is fully operator-configurable;
**30105 is a common convention (inherited from the PiAware/dump1090-fa scheme) but not
a protocol-mandated default** — verify per-network.

**Minimum receiver requirement for a resolvable MLAT fix:** FlightAware states
"data needs to be received by at least 4 different sites" for its MLAT calculation
([blog.flightaware.com/what-is-mlat](https://blog.flightaware.com/what-is-mlat)); this
is a reasonable general TDOA baseline (3 independent TDOA measurements from 4 receiver
pairs to solve for a 2D+bias position) though other networks did not restate the exact
minimum in the documents fetched here.

---

## 8. Comparison Table

| Source | Computes MLAT? | Access method (API vs feeder-client) | Requires being a feeder for local coverage? | Data format | Auth / rate limits | ToS friendliness for embedding in a distributed hobbyist desktop app | Integration complexity |
|---|---|---|---|---|---|---|---|
| **ADS-B Exchange** | Yes (`type: mlat`) | Both: v2 REST API (paid, ~$10/mo/10k req via RapidAPI) or feeder-client (`feedclient` + `mlat-client` fork, local Beast loopback on 30104) | Yes, for physical TDOA coverage; free-tier "feed-for-key" model largely phased out | JSON (API) / Beast binary (local loopback) | Paid API, key required; feeder registration + UUID for feed path | Poor-to-medium: commercial API has an AUP and paid tiers, redistribution terms not fully published; feeder-loopback path only returns your own contributed traffic (cleaner) | Medium (feeder-client setup) / Low (API, if paid) |
| **airplanes.live** | Yes (`mlat` field, ADSBX-v2-compatible schema) | Both: free keyless REST API, or feeder-client (`airplanes-live/feed`, local Beast loopback via `RESULTS` config) | No for API reads today; yes for good physical local MLAT density | JSON (API) / Beast binary (local loopback) | No key today ("might change"); no published hard rate limit found | Caveated: community docs explicitly call the API "educational and non-commercial… only" — risky for a *distributed* app without direct confirmation from the project | Low (API) / Medium (feeder-client) |
| **adsb.lol** | Yes (feeds "MLAT+ADSB" by default) | Both: free keyless REST API (ADSBX-v2-compatible), feeder-client (`adsblol/feed`), plus experimental feeders-only bulk MLAT-out (out.adsb.lol:1365/1366) | No for API reads; yes for feeders-only bulk-out; yes for physical local density in general | JSON (API) / Beast+SBS (feeder loopback, bulk-out) | No key today (planned); dynamic rate limits; **ODbL 1.0** license (attribution required, explicit and open) | **Best of the three community networks**: explicit open license, no commercial-use ban found | Low (API) / Medium (feeder-client) / High (bulk MLAT-out, explicitly experimental/unsupported) |
| **OpenSky Network** | Yes (`position_source: 2`, dedicated Trino MLAT table) | API only — no feeder-loopback/local re-sync mechanism documented | N/A for coverage benefit (no local sync path even as a sensor host) | JSON state vectors (REST); Parquet/SQL via Trino for historical | OAuth2 client-credentials; 400 (anon) / 4,000+ (auth) daily credits; `/states/own` unlimited but requires auth | Research-oriented; no explicit desktop-app redistribution ban found in fetched docs, but terms not exhaustively reviewed | Low (REST) but weak for live local use case |
| **FlightAware** | Yes (HyperFeed fuses MLAT + others) | Both: free feeder-client (`fa-mlat-client` via PiAware, local Beast/ext-Basestation loopback on 30104/30105/30106 — best-documented wiring found) or paid enterprise API (Firehose/AeroAPI) | Yes for the free path (need ≥4 nearby sites per FlightAware's own MLAT explainer) | Beast binary / ext-Basestation text (local); JSON Lines (Firehose) | Free for feeder loopback; enterprise API needs custom subscription, undisclosed self-serve pricing/redistribution terms | Poor for the API route (custom commercial contract required for redistribution); feeder-loopback route only returns your own contributed data, much cleaner | Medium (feeder-client) / High (enterprise API, sales negotiation) |
| **Flightradar24** | Unconfirmed/undocumented publicly (no `mlat` field found in fetched docs) | API only, no feeder-loopback path documented | N/A | JSON (assumed; field-level schema not exhaustively reviewed) | Paid tiers (Explorer/Essential/Advanced) + free Sandbox; API key required | Poor: explicit "no persistent local copies," "no competing product," flight-critical-use ban | High (paid, restrictive ToS, no MLAT-specific hook found) |

---

## 9. Correction re: AirJedi's Current Feed Configuration

The task brief for this research initially assumed a hostname of `adsb.custine.com`.
That is incorrect. The actual reference in the codebase
(`crates/adsb-client/examples/df_diag.rs`, `bds50_diag.rs`) is:

```rust
let host = std::env::args()
    .nth(1)
    .unwrap_or_else(|| "airjedi.custine.com:30005".to_string());
```

This is `airjedi.custine.com:30005` — the user's own private DNS name for what is
almost certainly a personally-operated dump1090/readsb instance, forwarded to a public
hostname, using port 30005 (standard Beast protocol port). This is **not** an
aggregator-provided feeder hostname or convention associated with any of the networks
surveyed above (ADSBX, airplanes.live, adsb.lol, FlightAware, or FR24 all use their own
distinct `feed.<network>` / `out.<network>` hostnames for their upload/download
endpoints — none of them is `airjedi.custine.com`). The checked-in `config.toml` at the
repo root reinforces this: its `[feed].endpoint_url` is `"192.168.1.10:30003"` — a local
LAN IP on SBS-1 port 30003, i.e. a plain local dev/default value, not any aggregator
config. **There is no evidence in the repository of an existing feeder relationship
with any of the aggregator networks discussed in this document.** Any MLAT integration
would start from zero — AirJedi is a pure local-receiver consumer today, not a
registered feeder anywhere.

---

## 10. Recommendation

Given AirJedi already runs its own local Beast/RTL-SDR receiver (reachable at
`airjedi.custine.com:30005`, standard Beast port, already the exact default
`--input-connect` port `mlat-client` expects) but has **no existing aggregator feeder
relationship**, the practical path is:

1. **Start a feeder relationship with `adsb.lol`.** It has the most redistribution-friendly,
   explicit terms found in this research (ODbL 1.0, no commercial-use ban, no API key
   currently required), a free ADSBX-v2-compatible read API, and its `adsblol/feed`
   container toolkit is a low-friction way to run `mlat-client` against the existing
   receiver without disturbing AirJedi's own decode path. As a bonus, the same feeder
   toolkit can simultaneously feed ADS-B Exchange, airplanes.live, FlightAware, and FR24
   if the user later wants broader MLAT coverage or cross-checking, since none of these
   are mutually exclusive at the feeder level.

2. **Consume MLAT results the way PiAware/dump1090-fa do: via local `mlat-client`
   loopback, not a remote read API.** Run `mlat-client` (or `wiedehopf/mlat-client`)
   pointed at `--input-connect airjedi.custine.com:30005 --input-type beast --server
   feed.adsb.lol:... --results beast,listen,<local-port>`, then add a second Beast
   connection in AirJedi's `adsb-client`/`AdsbPlugin` layer to that local results port,
   merging MLAT-derived aircraft into the existing ICAO-keyed aircraft map exactly as
   multiple live feeds are already merged today. This sidesteps every read-API ToS
   question entirely — you would only ever be receiving back synthesized Beast frames
   for traffic your *own* receiver contributed, computed from a network your own
   receiver is a member of. This is architecturally identical to the wiring already
   documented for `dump1090-fa`/PiAware (§7) and requires zero new wire-format work
   since Beast is Beast.

3. **Treat airplanes.live as a secondary/complementary feeder** for the same reason —
   free API, no key, and a second independent MLAT solution to compare against — but
   read its `api-guide` terms directly in-browser first (this research could not fetch
   that page programmatically; it returned HTTP 403) since third-party integrators
   describe its API as "educational and non-commercial… only," which is a meaningfully
   different posture from adsb.lol's ODbL license if AirJedi is ever distributed beyond
   personal use (it already is, via Homebrew).

4. **Avoid ADS-B Exchange's read API, OpenSky, FlightAware's commercial API, and
   Flightradar24 for this use case.** ADSBX's free API tier is gone; OpenSky has no
   local re-sync mechanism and is research/backfill-oriented; FlightAware's MLAT-bearing
   commercial products require a custom sales contract with unclear redistribution
   terms; FR24 has an explicit "no persistent local copies" clause that conflicts with
   a desktop tracker's normal caching behavior and no documented MLAT field at all.
   FlightAware's *free* feeder-loopback path (`fa-mlat-client` via PiAware) is
   technically excellent and the best-documented of all six sources, but adopting it
   means running PiAware specifically rather than a lighter feeder toolkit — worth
   revisiting later purely for its port-per-format loopback design as a reference, even
   if adsb.lol is the first integration target.

**Bottom line:** adsb.lol (primary) + optionally airplanes.live (secondary, pending ToS
re-check), both consumed via local `mlat-client` Beast loopback merged into AirJedi's
existing multi-feed aircraft model — not via any remote read API — is the lowest-risk,
lowest-integration-cost path to real local MLAT coverage for AirJedi.
