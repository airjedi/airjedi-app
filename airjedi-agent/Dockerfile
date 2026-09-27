# Headless AirJedi fusion agent image for k3s.
#
# Build from the REPO ROOT (the build needs the whole workspace), targeting the
# node's arch (the airjedi node is arm64):
#
#   docker buildx build --platform linux/arm64 \
#     -f airjedi-agent/Dockerfile -t airjedi-agent:0.1.2 .
#
# `cargo build -p airjedi-agent` compiles only the agent's dependency graph
# (airjedi-core/fusion/net + adsb-client + headless bevy/replicon/renet); the
# desktop app crate and its heavy deps (egui, full bevy, shadow-rs/libgit2) are
# NOT built, so no system libgit2/openssl is needed at build or run time.

FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p airjedi-agent \
    && strip target/release/airjedi-agent

# --- runtime ---
FROM debian:bookworm-slim AS runtime
# Pure-Rust binary: only needs glibc (present in debian-slim). No TLS, no zlib,
# no libgit2. Run as an unprivileged system user.
RUN useradd --system --no-create-home --uid 10001 airjedi
COPY --from=builder /src/target/release/airjedi-agent /usr/local/bin/airjedi-agent
USER 10001
# renet/replicon UDP transport (documentation; hostNetwork exposes it on the node).
EXPOSE 5599/udp
ENTRYPOINT ["/usr/local/bin/airjedi-agent"]
# Default args are supplied by the k8s Deployment (--feed <host:port> --port 5599).
