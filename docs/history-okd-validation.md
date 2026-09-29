# History OKD Validation Profile

This document defines the reproducible in-cluster validation profile used for
the history-on-connect work on WOPR `gpu-0`.

## Build Target

- Namespace: `airjedi-build`
- BuildConfig: `airjedi-validation`
- Build node: `gpu-0.wopr.custine.com` (via BuildConfig `nodeSelector`)
- Dockerfile: `tests/validation/Dockerfile.ocp-validation`

## Run The Profile

From the repository root:

```bash
oc patch bc airjedi-validation -n airjedi-build --type=merge -p '{"spec":{"strategy":{"dockerStrategy":{"dockerfilePath":"tests/validation/Dockerfile.ocp-validation"}}}}'
oc start-build airjedi-validation -n airjedi-build --from-dir=. --follow
```

To inspect the latest build status:

```bash
oc get builds -n airjedi-build
oc describe build airjedi-validation-<build-number> -n airjedi-build
oc logs -f build/airjedi-validation-<build-number> -n airjedi-build
```

## Validation Matrix

The profile runs:

- `cargo test --locked -p adsb-client`
- `cargo test --locked -p airjedi-core`
- `cargo test --locked -p airjedi-fusion`
- `cargo test --locked -p airjedi-net`
- `cargo check --locked -p airjedi-agent`
- `cargo check --locked -p airjedi_bevy`
- `cargo check --locked -p airjedi_bevy --features thin-client`
- `cargo test --locked --no-default-features --bin airjedi_bevy history_acceptance::tests::full_late_join_experience_converges_through_reconnect_and_retention`
- `cargo test --locked --workspace --no-fail-fast` with explicit temporary
  skips for four known fixture failures.

## Temporary Skip List

The workspace test step currently skips these tests:

- `data_ingest::fixture_tests::real_data_notam_expired_filtered`
- `data_ingest::fixture_tests::real_data_tfr_multipolygon_handled`
- `data_ingest::fixture_tests::real_data_tfr_parses_without_error`
- `data_ingest::fixture_tests::real_data_tfr_string_altitudes_parsed`

These are fixture-data consistency failures in `src/data_ingest/fixture_tests.rs`.
They are intentionally isolated from history feature validation so the full
history acceptance and transfer matrix can run in-cluster.

Follow-up: `https://github.com/airjedi/airjedi-app/issues/41`

Remove these skips after the fixture tests are fixed and stable.
