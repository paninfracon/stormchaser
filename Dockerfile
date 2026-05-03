# Multi-stage Dockerfile for Stormchaser
FROM rust:1.91-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libgit2-dev \
    zlib1g-dev \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the entire workspace
COPY . .

# Run the ratatui-form patch script
RUN ./scripts/patch-ratatui-form.sh

# Build argument to specify which binary to build
ARG BINARY=stormchaser-engine

# Build the specified binary and copy it out of the cache mount
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    if [ "$BINARY" = "stormchaser-agent" ]; then \
        apt-get update && apt-get install -y musl-tools && \
        rustup target add x86_64-unknown-linux-musl && \
        SQLX_OFFLINE=true cargo build --release -p ${BINARY} --target x86_64-unknown-linux-musl && \
        cp /app/target/x86_64-unknown-linux-musl/release/${BINARY} /app/${BINARY}; \
    else \
        SQLX_OFFLINE=true cargo build --release -p ${BINARY} && \
        cp /app/target/release/${BINARY} /app/${BINARY}; \
    fi

# Final runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    jq \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install oras CLI
RUN curl -LO "https://github.com/oras-project/oras/releases/download/v1.2.0/oras_1.2.0_linux_amd64.tar.gz" && \
    mkdir -p oras-install/ && \
    tar -zxf oras_1.2.0_linux_amd64.tar.gz -C oras-install/ && \
    mv oras-install/oras /usr/local/bin/ && \
    rm -rf oras_1.2.0_linux_amd64.tar.gz oras-install/

WORKDIR /app

# Re-declare ARG for the second stage
ARG BINARY=stormchaser-engine
ENV BINARY_NAME=${BINARY}

# Copy the binary from the builder
COPY --from=builder /app/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}

# Copy migrations (needed by both API and Engine)
COPY --from=builder /app/migrations /app/migrations

# Run the binary
CMD ["/bin/sh", "-c", "/usr/local/bin/${BINARY_NAME}"]
