FROM rust:1.98-slim AS build
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release --bin kiln

FROM debian:bookworm-slim AS runtime
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates git && \
    rm -rf /var/lib/apt/lists/*

# Install buildctl (BuildKit client)
COPY --from=moby/buildkit:v0.21.1 /usr/bin/buildctl /usr/local/bin/buildctl

COPY --from=build /app/target/release/kiln /usr/local/bin/kiln

EXPOSE 8080
ENTRYPOINT ["kiln"]
