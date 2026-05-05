# Security Policy

## Supported Versions

Only the `main` branch and the latest container images published from it
(`ghcr.io/frogshead/ratakierros-fi/api:latest`,
`ghcr.io/frogshead/ratakierros-fi/frontend:latest`) receive security updates.

## Reporting a Vulnerability

Please do **not** open a public GitHub issue for security problems.

Use one of the following private channels:

- **GitHub private vulnerability reporting** — preferred. Open the
  [Security tab](https://github.com/frogshead/ratakierros-fi/security/advisories/new)
  and submit a private advisory.
- **Email** — send details to the repository maintainer
  (see the GitHub profile of [@frogshead](https://github.com/frogshead)).

Please include:
- A description of the issue and the impact you observed.
- Steps to reproduce, or a proof-of-concept.
- The commit hash or container image tag the report applies to.

## Response Expectations

- Triage within **7 days** of receipt.
- Status updates at least every **14 days** until resolution.
- Coordinated disclosure once a fix is published.

## Standing Vulnerability Management Process

To meet CRA-style requirements, the project runs the following continuously:

- **Dependabot** (`.github/dependabot.yml`) — weekly upstream version checks for
  Rust crates, Docker base images, and GitHub Actions.
- **`cargo audit`** — runs in CI on every push to `main`, fails the build on any
  open RUSTSEC advisory affecting `api/Cargo.lock`.
- **Image vulnerability scan (Grype)** — runs in CI against the freshly built
  API and frontend images, fails on `high` or higher severity findings.
- **SBOM generation (Syft, CycloneDX)** — runs in CI for both images and the
  source tree; SBOMs are uploaded as workflow artifacts and retained for audit.

The CI workflow lives at `.github/workflows/build.yml` and gates the
production deploy on all of the above succeeding.
