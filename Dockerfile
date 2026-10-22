# Multi-stage Dockerfile for Stormchaser
FROM rust:1.98-slim-bookworm@sha256:ff521445a372125ed4f76e1453a1f8098f2d05332d1601d30db1c1f62757e730 AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libgit2-dev \
    zlib1g-dev \
    curl \
    git \
    make \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the entire workspace
COPY . .



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
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    jq \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install oras CLI
RUN curl -LO "https://github.com/oras-project/oras/releases/download/v1.2.0/oras_1.2.0_linux_amd64.tar.gz" && \
    echo "5b3f1cbb86d869eee68120b9b45b9be983f3738442f87ee5f06b00edd0bab336  oras_1.2.0_linux_amd64.tar.gz" | sha256sum -c - && \
    mkdir -p oras-install/ && \
    tar -zxf oras_1.2.0_linux_amd64.tar.gz -C oras-install/ && \
    mv oras-install/oras /usr/local/bin/ && \
    rm -rf oras_1.2.0_linux_amd64.tar.gz oras-install/

WORKDIR /app

# Re-declare ARG for the second stage
ARG BINARY=stormchaser-engine
ENV BINARY_NAME=${BINARY}

# The engine decrypts workflows' SOPS-encrypted secrets with the sops CLI; without it
# every such run fails
RUN if [ "${BINARY_NAME}" = "stormchaser-engine" ]; then \
      curl -fsSLo /usr/local/bin/sops "https://github.com/getsops/sops/releases/download/v3.13.3/sops-v3.13.3.linux.amd64" && \
      echo "e5bec3346a873ae91d871550f3e698c1aad962aff462a080e40f25fde17fef6b  /usr/local/bin/sops" | sha256sum -c - && \
      chmod 0755 /usr/local/bin/sops; \
    fi

# Copy the binary from the builder
COPY --from=builder /app/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}

# Copy migrations (needed by both API and Engine)
COPY --from=builder /app/migrations /app/migrations

# Run the binary
# Services run as an unprivileged user. The engine's git cache is created owned by it,
# so a fresh volume mounted there is writable. The Docker runner (which needs the
# Docker socket) and the agent (which writes into steps' volumes) are run as root by
# what starts them.
RUN groupadd --system --gid 65532 stormchaser && \
    useradd --system --uid 65532 --gid 65532 --no-create-home --home-dir /nonexistent stormchaser && \
    mkdir -p /tmp/stormchaser/git-cache && chown -R 65532:65532 /tmp/stormchaser
USER 65532:65532

CMD ["/bin/sh", "-c", "/usr/local/bin/${BINARY_NAME}"]
