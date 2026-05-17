# MeshMend

MeshMend is a Tauri desktop STL viewer for inspecting and later repairing
AI-generated 3D model meshes.

The current milestone is intentionally narrow:

- load any `.stl` file from disk
- render it with Three.js
- orbit, pan, and zoom
- fit/reset the camera
- show basic mesh stats

Run from this directory:

```bash
npm install
npm run tauri dev
```

The project does not bundle model assets. Use the local ignored test file at
`../../rose/raw.stl` when developing in this workspace.

Verification:

```bash
npm run build
npm run verify:viewer
npm run tauri build -- --bundles app
```

Regenerate app icons:

```bash
python3 scripts/generate_icons.py
```
