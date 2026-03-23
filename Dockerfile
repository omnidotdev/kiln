FROM rust:1.85 AS build
WORKDIR /app
COPY . .
RUN cargo build --release --bin kiln

FROM debian:bookworm-slim AS runtime
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates git && \
    rm -rf /var/lib/apt/lists/*

# Install buildctl (BuildKit client)
COPY --from=moby/buildkit:latest /usr/bin/buildctl /usr/local/bin/buildctl

COPY --from=build /app/target/release/kiln /usr/local/bin/kiln

ENTRYPOINT ["kiln"]
