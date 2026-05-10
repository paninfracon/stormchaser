# Release How-To

This document outlines the complete checklist for preparing a new release and provides a contingency plan for manually building and publishing the release if the automated CI pipeline fails.

## 1. Release Preparation

Before cutting a new release tag, ensure the following steps are meticulously followed to prepare the codebase:

1. **Update `CHANGELOG.md`:**
    * Add a new `## [X.Y.Z] - YYYY-MM-DD` section under `## [Unreleased]`.
    * Summarize the major additions, changes, and fixes based on the recent Git history.

2. **Bump Cargo Workspace Versions:**
    * In the root `Cargo.toml`, update `workspace.package.version = "X.Y.Z"`.
    * In the root `Cargo.toml` under `[workspace.dependencies]`, update the versions of internal crates (e.g., `stormchaser-model`, `stormchaser-dsl`, `stormchaser-tls`) to `"X.Y.Z"`.

3. **Bump Individual Crate Dependencies:**
    * Search through all `crates/*/Cargo.toml` files for hardcoded version strings of internal dependencies (e.g., `stormchaser-opa = { version = "0.1.0", path = ... }`).
    * Update these to point to the new `"X.Y.Z"` version.

4. **Update Helm Chart Versions:**
    * In all `Chart.yaml` files under `deploy/charts/` (e.g., `stormchaser`, `stormchaser-orchestration`, `stormchaser-runner-k8s`, `stormchaser-runner-docker`, `stormchaser-telemetry`), update the `version` and `appVersion` fields to `"X.Y.Z"`.
    * Run `helm dependency update deploy/charts/stormchaser` to regenerate the `Chart.lock` file.

5. **Update GitHub Issue Templates:**
    * In `.github/ISSUE_TEMPLATE/bug_report.md`, bump the version placeholder (e.g., `- Stormchaser Version: [e.g. X.Y.Z]`).

6. **Update README Badges:**
    * In `README.md`, update the `Crates.io` and `Docs.rs` static badges to reflect the new `"vX.Y.Z"` version.

7. **Verify Project Integrity (Crucial before check-in):**
    * Format code: `cargo fmt`
    * Linting: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
    * Unit Tests: `./scripts/test-unit.sh`
    * Integration Tests: `./scripts/test-integration.sh`
    * Test Coverage: `./scripts/coverage.sh` (Verify coverage has not significantly degraded).

## 2. Docker and Vergen Configuration

Stormchaser uses `vergen` to embed Git information into the compiled binaries. When building via Docker, ensure that the `.dockerignore` file allows `vergen` to access the `.git` directory without bloating the build context.

The `.dockerignore` should include exactly these lines for Git:

```gitignore
.git/*
!.git/HEAD
!.git/config
!.git/refs/
!.git/objects/
```

Ensure that `vergen` is listed in `[build-dependencies]` and a `build.rs` executing `vergen::EmitBuilder` exists for all crates that need to embed version info (including runners and the agent).

## 3. Manual Release Contingency Plan

If the automated GitHub Actions CI (`release.yml`) fails, follow these steps to manually build, package, and push the release artifacts.

### Step 1: Build the Release Binaries

```bash
# 1. Apply UI patches required before building


# 2. Build the standard dynamic binaries across the workspace
SQLX_OFFLINE=true cargo build --release --workspace

# 3. Build the statically linked agent (Requires musl tools)
# Setup: rustup target add x86_64-unknown-linux-musl && sudo apt-get install -y musl-tools
SQLX_OFFLINE=true cargo build --release -p stormchaser-agent --target x86_64-unknown-linux-musl

# 4. Swap the dynamic agent for the static one so the prebuilt Dockerfile packages the correct artifact
cp target/x86_64-unknown-linux-musl/release/stormchaser-agent target/release/stormchaser-agent
```

### Step 2: Package and Upload the Binaries

```bash
mkdir -p dist
for BIN in stormchaser-api stormchaser-engine stormchaser-runner-k8s stormchaser-runner-docker stormchaser-agent stormchaser stormchaser-tui; do
  if [ -f "target/release/$BIN" ]; then
    tar -czvf dist/$BIN-x86_64-unknown-linux-gnu.tar.gz -C target/release $BIN
  fi
done

# If using GitHub CLI, create the release manually:
# gh release create vX.Y.Z dist/*.tar.gz --title "vX.Y.Z" --notes-file CHANGELOG.md
```

### Step 3: Build and Push the Docker Images

We use `Dockerfile.prebuilt` to quickly containerize the binaries compiled locally in Step 1, rather than compiling from scratch inside Docker again.

```bash
# Authenticate with the GitHub Container Registry
docker login ghcr.io -u <your-github-username>

# Define the components to build
COMPONENTS=("stormchaser-api" "stormchaser-engine" "stormchaser-runner-k8s" "stormchaser-runner-docker" "stormchaser-agent")

export VERSION="X.Y.Z" # Replace with actual version

# Build and push each component
for BIN in "${COMPONENTS[@]}"; do
  IMAGE_TAG="ghcr.io/paninfracon/stormchaser-${BIN}:${VERSION}"

  echo "Building ${IMAGE_TAG}..."
  docker build -f Dockerfile.prebuilt \
    --build-arg BINARY=$BIN \
    -t $IMAGE_TAG .

  echo "Pushing ${IMAGE_TAG}..."
  docker push $IMAGE_TAG
done
```

### Step 4: Package and Publish Helm Charts

```bash
helm dependency update deploy/charts/stormchaser
helm package deploy/charts/stormchaser
# Push the resulting .tgz to your Helm repository (if applicable)
```

### Step 5: Publish Crates to crates.io

Ensure your local `CARGO_REGISTRY_TOKEN` is configured or run `cargo login`.

Due to index propagation delays, publish the foundational crates first and pause before publishing dependent crates.

```bash
# Publish foundational crates first
cargo publish -p stormchaser-model
sleep 15
cargo publish -p stormchaser-opa
sleep 15
cargo publish -p stormchaser-tls
sleep 15
cargo publish -p stormchaser-dsl
sleep 15

# Publish dependent crates
cargo publish -p stormchaser-engine
sleep 15
cargo publish -p stormchaser-api
sleep 15
cargo publish -p stormchaser-runner-docker
sleep 15
cargo publish -p stormchaser-runner-k8s
sleep 15
cargo publish -p stormchaser-agent
sleep 15

# Publish top-level CLI
cargo publish -p stormchaser-cli
```
