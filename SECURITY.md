# Security Policy

## Supported Versions

Stormchaser is in active development. Only the latest release is supported for security updates.

| Version | Supported |
| ------- | --------- |
| 1.5.x   | Yes       |
| < 1.5   | No        |

## How Stormchaser is secured

[docs/security.md](docs/security.md) describes authentication, the authorization
policy, transport security, what operators must provide (such as encryption of the
database at rest), and the known limitations of the current release.
[docs/nats.md](docs/nats.md) covers securing the NATS broker, and
[docs/opa-rbac.md](docs/opa-rbac.md) writing your own policy.

## Reporting a Vulnerability

We take the security of Stormchaser seriously, especially given its role in orchestrating workloads and handling secrets.

If you discover a security vulnerability, please **DO NOT** open a public issue. Instead, send an email to the core team at `security@paninfracon.net`.

Please include the following information:

- A description of the vulnerability.
- Steps to reproduce the issue.
- Any potential impact or risk you have identified.
- Your contact information so we can follow up.

We will acknowledge receipt of your vulnerability report within 48 hours and strive to send you regular updates about our progress. If you report a vulnerability that materially affects our users, we will give you full credit for the discovery in our release notes (unless you prefer to remain anonymous).
