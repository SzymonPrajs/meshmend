# Release Workflow

CI builds Linux and Windows artifacts from committed fixtures and source only.
The ignored local `rose/raw.stl` file is never required in CI.

Early release status:

```text
Unsigned developer preview
```

Local macOS packaging is available through:

```bash
just package
just package-smoke
```

The package recipe builds:

```text
target/package/MeshMend.app/
  Contents/MacOS/meshmend
  Contents/Resources/workers/meshmend-cgal-worker
  Contents/Resources/workers/meshmend-openvdb-worker
```

The app binary discovers workers from `Contents/Resources/workers`, matching the
runtime worker discovery path used by release builds.

Signing is intentionally late-stage:

- Windows requires a code-signing certificate and GitHub secret handling.
- macOS requires a Developer ID certificate, notarization credentials, stapling,
  and Gatekeeper verification.
- Linux can start with SHA256 checksums and GitHub release provenance.

The macOS workflow is manual-only so normal PRs and pushes do not consume macOS
runner credits.
