# Public Publishing Audit Report

I have conducted an analysis of the project to identify issues that would counter-indicate publishing it publicly. Here is the report detailing the critical blockers and areas that need attention before a public release:

### 1. Placeholder & Incomplete Documentation (Critical Blockers)

There are several areas where documentation contains template placeholders or refers to the wrong project entirely:

* **`SECURITY.md`:** Contains a placeholder email address `security@example.com` along with the explicit instruction: `(replace with actual security email before public launch)`. This must be updated with a real contact method for vulnerability disclosures.
* **`CONTRIBUTING.md`:** Refers to the project as **"Paninfracon"** rather than "Stormchaser" and directs users to clone and open issues on a different repository (`https://github.com/cydergoth/paninfracon.git`). This appears to be copy-pasted from another project.
* **`README.md` & `Cargo.toml`:** Both files use the placeholder repository URL `https://github.com/your-org/stormchaser`. These need to point to the actual public repository.

### 2. Security Vulnerabilities in Dependencies

Running `cargo audit` revealed **8 active vulnerabilities** and several unmaintained dependencies that should be resolved before making the project public:

* **High/Medium Severity Vulnerabilities:**
  * **`rsa 0.9.10`:** Marvin Attack potential key recovery (RUSTSEC-2023-0071).
  * **`rustls-webpki 0.101.7` & `0.102.8`:** Multiple vulnerabilities including reachable panics (RUSTSEC-2026-0104) and faulty certificate constraint validation logic (RUSTSEC-2026-0098, RUSTSEC-2026-0099, RUSTSEC-2026-0049).
  * **`git2 0.19.0`:** Potential undefined behavior when dereferencing (RUSTSEC-2026-0008).
  * **`lru 0.12.5`:** `IterMut` violates Stacked Borrows (RUSTSEC-2026-0002).
* **Unmaintained Crates:** The dependency tree relies on deprecated libraries that no longer receive security patches, including `backoff 0.4.0`, `instant 0.1.13`, and `rustls-pemfile` (versions 1.0.4 and 2.2.0).

### 3. Hardcoded Default Credentials

While these are primarily found in development, testing, and deployment configurations, they pose a risk if users deploy the default configurations directly to production:

* **`deploy/charts/stormchaser/values.yaml`:** Contains default plaintext credentials such as `password: stormchaserpassword` for PostgreSQL and `clientSecret: "stormchaser-cli-secret"` for OIDC/Dex.
* **`docker-compose.yml`:** Hardcodes `POSTGRES_PASSWORD: stormchaser` and `MINIO_ROOT_PASSWORD: stormchaser`.
* **Engine Source Code:** `crates/stormchaser-engine/src/handler/integrations/approval.rs` (Line 174) contains a hardcoded fallback: `smtp_password: Some("pass".to_string())`.

### 4. Dependency Duplication & License Hygiene

* **`cargo deny check`** flagged `OpenSSL` as an "unmatched license allowance". While there is a note in `deny.toml` indicating compatibility was manually verified, relying on OpenSSL might complicate the strict `MIT OR Apache-2.0` licensing of your workspace.
* There are significant duplicate crate versions in your `Cargo.lock` (e.g., multiple versions of `aws-sdk-*` crates, `thiserror`, `tower`, and `wasm-encoder`). This drastically bloats compilation times and binary sizes for end-users.

### 5. Missing `publish = false` Flags

None of the internal workspace crates in the `crates/` directory (e.g., `stormchaser-agent`, `stormchaser-runner-docker`, `stormchaser-api`) have the `publish = false` flag set in their `Cargo.toml`.

* **Impact:** Running `cargo publish` at the workspace root will attempt to publish *every* crate to crates.io. If you intend for some of these crates to remain private or be distributed only via Docker images/binaries, you should explicitly set `publish = false` on them.
