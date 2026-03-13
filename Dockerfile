FROM rust:1.85-slim AS builder

WORKDIR /usr/src/gstd-bridge
COPY . .

# Install required build dependencies
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/gstd-bridge/target/release/gstd-bridge .
COPY --from=builder /usr/src/gstd-bridge/Cargo.toml .

# Create a data directory
RUN mkdir -p /app/data

EXPOSE 4001
EXPOSE 9090

# We don't specify ENTRYPOINT directly here so we can optionally do `--init`
CMD ["./gstd-bridge"]
