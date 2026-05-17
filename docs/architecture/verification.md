# Verification

Core checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

Parser checks:

```bash
cargo test -p meshmend-stl
cargo run -p meshmend -- inspect fixtures/stl/cube_binary.stl
cargo run -p meshmend -- inspect rose/raw.stl --parallel
```

Renderer checks:

```bash
cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
cargo run -p meshmend -- --screenshot fixtures/stl/cube_binary.stl outputs/cube.png
cargo run -p meshmend -- --verify-render rose/raw.stl
```

`--verify-render` captures pixels from the native WGPU surface and fails if the
image is effectively blank. Outputs under `outputs/` are ignored.
