FROM rust:1.97-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl sqlite3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin statesync
COPY --from=builder /build/target/release/podcast-sync-server /usr/local/bin/podcast-sync-server

USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/podcast-sync-server"]
