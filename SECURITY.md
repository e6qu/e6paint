# Security Policy

## Supported versions

| version | support |
|---------|---------|
| 1.0.x   | security fixes |

## Reporting a vulnerability

Please use [GitHub Security Advisories](https://github.com/e6qu/e6paint/security/advisories/new)
to report vulnerabilities privately. Do not open public issues for security reports.

You will get an acknowledgment within a few days, and a fix or a mitigation plan
before any public disclosure.

## Scope notes

- The **relay** (`e6-relay`) is the trusted server component: it holds room
  snapshots in SQLite and gates writes with capability tokens (write token) and
  presence-only participation (read token). Tokens are opaque UUIDs minted at
  room creation and carried in share links — treat share links as secrets.
- The **web client** is static WASM served from GitHub Pages; it stores the
  board in the browser's OPFS (local-only) and talks to the relay configured
  via `?relay=` (default: `http://127.0.0.1:8480` for local development).
- Board content is **not encrypted** in transit beyond TLS termination at your
  proxy, and **not encrypted at rest** in the relay's SQLite file — deploy the
  relay on infrastructure you trust.

## Known accepted risk

- RUSTSEC unmaintained advisories on the transitive `loro -> im/sized-chunks`
  chain are documented as accepted upstream (no fix exists); `cargo audit`
  runs in CI and must stay free of vulnerabilities.
