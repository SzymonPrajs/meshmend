# Troubleshooting

## GPU Backend

MeshMend prefers the native backend for the platform:

- macOS: Metal
- Windows: D3D12
- Linux: primary `wgpu` backends

The selected adapter and backend are exposed in the UI model panel and included
in performance JSON output.

## Blank Screenshots

Run:

```bash
cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
```

If coverage is near zero, check that the platform can create a WGPU surface and
that the selected backend is available.

## Local Data

`rose/raw.stl` is intentionally ignored. CI and release workflows must use
committed fixtures only.
