# syntax=docker/dockerfile:1

FROM rust:1.85 AS build
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin kiln && \
    cp /app/target/release/kiln /usr/local/bin/kiln

FROM debian:bookworm-slim AS runtime
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates git && \
    rm -rf /var/lib/apt/lists/*

# Install buildctl (BuildKit client)
COPY --from=moby/buildkit:latest /usr/bin/buildctl /usr/local/bin/buildctl

COPY --from=build /usr/local/bin/kiln /usr/local/bin/kiln

ENTRYPOINT ["kiln"]
