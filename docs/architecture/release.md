# Release Workflow

CI builds Linux and Windows artifacts from committed fixtures and source only.
The ignored local `rose/raw.stl` file is never required in CI.

Early release status:

```text
Unsigned developer preview
```

Signing is intentionally late-stage:

- Windows requires a code-signing certificate and GitHub secret handling.
- macOS requires a Developer ID certificate, notarization credentials, stapling,
  and Gatekeeper verification.
- Linux can start with SHA256 checksums and GitHub release provenance.

The macOS workflow is manual-only so normal PRs and pushes do not consume macOS
runner credits.
