# Native Workers

MeshMend runs heavy geometry through external worker processes. Rust writes a
versioned JSON request, launches the worker with `--request`, streams JSONL
progress from stdout, then reads the final response JSON.

Current binaries:

- `meshmend-cgal-worker`: links CGAL, validates binary STL input, performs the
  current `hole_fill` operation for simple boundary-loop repair, and performs
  the current `cut` operation by clipping a triangle soup against a plane and
  capping the kept mesh. It also performs the current `remesh` operation with
  CGAL isotropic remeshing over all faces.
- `meshmend-openvdb-worker`: links OpenVDB, validates binary STL input, and
  performs the current `local_sdf_wrap` operation by converting STL triangles
  to an OpenVDB level set, extracting a replacement surface, and writing STL.

Local dependencies are installed through Homebrew:

```bash
brew install cgal openvdb
```

Build and smoke-test:

```bash
just worker-build
cargo run -p meshmend -- worker-smoke cgal fixtures/stl/cube_binary.stl
cargo run -p meshmend -- worker-smoke openvdb fixtures/stl/cube_binary.stl
cargo run -p meshmend -- hole-fill fixtures/stl/cube_missing_top.stl --output outputs/cube-filled.stl
cargo run -p meshmend -- local-wrap fixtures/stl/cube_binary.stl --output outputs/cube-wrapped.stl --voxel-size 0.08
cargo run -p meshmend -- cut fixtures/stl/cube_binary.stl --output outputs/cube-cut.stl --normal 1 0 0 --offset 0 --keep positive
cargo run -p meshmend -- remesh fixtures/stl/cube_binary.stl --output outputs/cube-remesh.stl --target-edge-length 0.75
```

`MESHMEND_WORKER_DIR` can point the Rust runner at a custom worker directory.
Without it, the runner checks `target/workers/cpp`, then bundled app resources.

Licensing gate: CGAL polygon mesh processing and simplification are GPL-covered
in the open-source distribution. Do not distribute worker binaries until the
release/license posture is explicit.
