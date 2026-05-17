# MeshMend Command, Scenario, and Control Workflow

MeshMend now has a scriptable command surface for visual and repair workflow verification. The desktop UI, scenario runner, render command, and optional live control socket all use the same `AppCommand` schema.

## Render a View

Create a deterministic viewport screenshot and optional state JSON:

```bash
cargo run -p meshmend -- render fixtures/stl/cube_binary.stl \
  --output outputs/render-cube.png \
  --state outputs/render-cube-state.json \
  --width 1280 \
  --height 800 \
  --view rendered
```

The render command fails if the image is effectively blank.

## Run Scenarios

Scenario files live under `tests/scenarios/`. They are JSON documents with:

- an input STL,
- a fixed viewport size,
- ordered `AppCommand` steps,
- semantic assertions.

Run all tracked smoke scenarios:

```bash
just scenario-smoke
```

This currently verifies:

- a successful view-line cut,
- multiple successful cuts,
- face and brush selection,
- orbit, pan, and zoom camera movement,
- screenshots and state reports,
- STL export and reload validation.

Each run writes a folder under `outputs/` containing screenshots, state JSON, exported STL files, and `run-report.json`.

## Live Local Control

The live control socket is disabled by default. Start the app with an explicit Unix socket:

```bash
cargo run -p meshmend -- --control-socket target/meshmend-control/dev.sock --replace-control-socket fixtures/stl/cube_binary.stl
```

Then send commands from another shell:

```bash
target/debug/meshmend control --socket target/meshmend-control/dev.sock state
target/debug/meshmend control --socket target/meshmend-control/dev.sock screenshot outputs/live.png
target/debug/meshmend control --socket target/meshmend-control/dev.sock preview-cut 500 120 780 680
target/debug/meshmend control --socket target/meshmend-control/dev.sock apply-cut
target/debug/meshmend control --socket target/meshmend-control/dev.sock select-object 0
target/debug/meshmend control --socket target/meshmend-control/dev.sock export-visible outputs/live-cut.stl
```

The protocol is JSON lines over a local Unix domain socket. It does not open a TCP port and it does not execute shell commands.

Run the socket smoke test:

```bash
just control-smoke
```

## Adding a New User-Visible Tool

For every new major UI action:

1. Add or extend an `AppCommand` variant in `apps/meshmend/src/commands.rs`.
2. Execute it in the scenario runner.
3. Wire the UI or live control path to the same command.
4. Add a scenario when the behavior is user-visible.
5. Include screenshots, state reports, and export/reload checks where relevant.
