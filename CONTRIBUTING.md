# Contributing to Stormchaser

First off, thank you for considering contributing to Stormchaser! It's people like you that make Stormchaser such a great tool.

Stormchaser is an open source project and we love to receive contributions from our community — you! There are many ways to contribute, from writing tutorials or blog posts, improving the documentation, submitting bug reports and feature requests or writing code which can be incorporated into Stormchaser itself.

Following these guidelines helps to communicate that you respect the time of the developers managing and developing this open source project. In return, they should reciprocate that respect in addressing your issue or assessing patches and features.

## Code of Conduct

We have a [Code of Conduct](CODE_OF_CONDUCT.md) that we expect all contributors to adhere to. Please read it before contributing.

## How to Report a Bug

If you find a bug, please open an issue on our GitHub repository. Please include as much information as possible, including:

* A clear and descriptive title.
* A description of the problem.
* Steps to reproduce the problem.
* The expected behavior.
* The actual behavior.
* Your system information (OS, Rust version, etc.).

## How to Suggest a Feature or Enhancement

If you have an idea for a new feature or an enhancement to an existing one, please open an issue on our GitHub repository. Please include:

* A clear and descriptive title.
* A detailed description of the proposed feature or enhancement.
* The motivation for the new feature.
* Any examples or mockups that might help to illustrate the new feature.

## Your First Code Contribution

Unsure where to begin contributing to Stormchaser? You can start by looking through these `good first issue` and `help wanted` issues:

* [Good first issues](https://github.com/paninfracon/stormchaser/labels/good%20first%20issue) - issues which should only require a few lines of code, and a test or two.
* [Help wanted issues](https://github.com/paninfracon/stormchaser/labels/help%20wanted) - issues which should be a bit more involved than `good first issue` issues.

### Getting the code

1. Fork the repository on GitHub.
2. Clone your fork locally: `git clone https://github.com/<your-username>/stormchaser.git`
3. Create a new branch from `trunk` for your changes: `git checkout -b my-new-feature`

### Prerequisites

* **Rust 1.96 or newer** (the workspace `rust-version`). Install or update it with `rustup update stable`. CI builds and lints with the latest stable toolchain, so run `cargo clippy` on stable before opening a pull request.
* **Docker** with Docker Compose v2, used by the test scripts to start Postgres and NATS, and by `scripts/setup.sh` for the local stack.
* **Python 3** (with `passlib[bcrypt]` for `scripts/setup.sh`) and **`openssl`**, used by the scripts to generate passwords, tokens and test certificates.
* **`sqlx-cli`** (`cargo install sqlx-cli`), used by the test scripts to apply database migrations.
* **`cargo-deny`** (`cargo install cargo-deny`), run by the pre-commit hooks and CI to check advisories and licenses.
* For the pre-commit hooks: [`prek`](https://github.com/j178/prek) or `pre-commit`, plus the `opa` CLI (Rego formatting and checks, and `scripts/build-opa-policy.sh`) and `terraform` (formats `.storm` and HCL files).
* For the Web UI and its end-to-end tests:
  * the WASM target: `rustup target add wasm32-unknown-unknown`
  * `cargo-leptos`: `cargo install --locked cargo-leptos`
  * a `wasm-bindgen` CLI whose version matches the `wasm-bindgen` crate resolved in `Cargo.lock` (check with `cargo tree -i wasm-bindgen --depth 0`, then `cargo install wasm-bindgen-cli --version <that version>`)
  * Node.js with the Playwright browsers: `cd crates/stormchaser-web/e2e && npm ci && npx playwright install --with-deps`

### Making changes

* Make your changes to the code.
* Format the code with `cargo fmt` (default `rustfmt` settings; there is no `rustfmt.toml`).
* This project uses pre-commit hooks (`.pre-commit-config.yaml`): formatting, clippy, cargo-deny, shellcheck, gitleaks, markdownlint, OPA checks and `stormchaser lint` on `.storm` files. Install them with `prek install` or `pre-commit install`.
* Add tests for your changes. You can run the tests via `./scripts/test-unit.sh` and `./scripts/test-integration.sh`; CI runs both with `--all-features`.
* Make sure the tests pass, and that `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo clippy -p stormchaser-engine --all-targets --no-default-features -- -D warnings` are clean.
* If you change `deploy/opa/policy.rego`, rebuild the built-in policy with `./scripts/build-opa-policy.sh` and commit `crates/stormchaser-opa/policy/stormchaser.wasm`.
* Add a line to the `[Unreleased]` section of `CHANGELOG.md` for user-visible changes.

## Pull Request Process

1. Push your changes to your fork: `git push origin my-new-feature`
2. Open a pull request on the Stormchaser repository.
3. The pull request will be reviewed by the maintainers.
4. Once the pull request is approved, it will be merged into the `trunk` branch.

Thank you for your contribution!
