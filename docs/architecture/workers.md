# Native Workers

MeshMend runs heavy geometry through external worker processes. Rust writes a
versioned JSON request, launches the worker with `--request`, streams JSONL
progress from stdout, then reads the final response JSON.

Current binaries:

- `meshmend-cgal-worker`: links CGAL and validates binary STL input as the first
  polygon-processing smoke operation.
- `meshmend-openvdb-worker`: links OpenVDB and validates binary STL input as
  the first SDF/voxel-processing smoke operation.

Local dependencies are installed through Homebrew:

```bash
brew install cgal openvdb
```

Build and smoke-test:

```bash
just worker-build
cargo run -p meshmend -- worker-smoke cgal fixtures/stl/cube_binary.stl
cargo run -p meshmend -- worker-smoke openvdb fixtures/stl/cube_binary.stl
```

`MESHMEND_WORKER_DIR` can point the Rust runner at a custom worker directory.
Without it, the runner checks `target/workers/cpp`, then bundled app resources.

Licensing gate: CGAL polygon mesh processing and simplification are GPL-covered
in the open-source distribution. Do not distribute worker binaries until the
release/license posture is explicit.
