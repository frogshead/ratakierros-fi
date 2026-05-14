FROM rust:1.95-slim AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y pkg-config libssl-dev build-essential && rm -rf /var/lib/apt/lists/*
ARG GIT_COMMIT=unknown
ENV GIT_COMMIT=$GIT_COMMIT
COPY Cargo.toml Cargo.lock* build.rs ./
COPY src/ src/
RUN cargo build --release

FROM ubuntu:26.04
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
# ubuntu:26.04 ships Canonical's Go-based service manager at /usr/bin/pebble.
# We run our own CMD, so pebble is dead weight — and its vendored Go stdlib
# trips the Grype "high+/only-fixed" gate in CI, blocking the deploy job.
RUN rm -f /usr/bin/pebble
RUN mkdir -p /data
COPY --from=builder /app/target/release/ratakierros-api /usr/local/bin/
EXPOSE 3000
CMD ["ratakierros-api"]
