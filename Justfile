set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    just --list

run:
    if [[ -f rose/raw.stl ]]; then cargo run -p meshmend -- rose/raw.stl; else cargo run -p meshmend; fi

run-file path:
    cargo run -p meshmend -- "{{path}}"

build:
    cargo build --workspace

release:
    cargo build --workspace --release

package:
    just release
    just worker-build
    bash scripts/package-macos.sh

package-smoke:
    just package
    target/package/MeshMend.app/Contents/MacOS/meshmend --smoke-window
    target/package/MeshMend.app/Contents/MacOS/meshmend worker-smoke cgal fixtures/stl/cube_binary.stl
    target/package/MeshMend.app/Contents/MacOS/meshmend worker-smoke openvdb fixtures/stl/cube_binary.stl

test:
    cargo test --workspace

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

verify:
    cargo run -p meshmend -- --verify-render fixtures/stl/cube_binary.stl
    cargo run -p meshmend -- --verify-cross-section fixtures/stl/cube_binary.stl
    cargo run -p meshmend -- --verify-view-modes fixtures/stl/cube_binary.stl
    cargo run -p meshmend -- --verify-hit-stack fixtures/stl/cube_binary.stl
    just scenario-smoke

scenario-smoke:
    cargo run -p meshmend -- scenario tests/scenarios/cube-view-line-cut.json --output-dir outputs/scenario-cube-view-line-cut
    cargo run -p meshmend -- scenario tests/scenarios/cube-two-cuts.json --output-dir outputs/scenario-cube-two-cuts
    cargo run -p meshmend -- scenario tests/scenarios/cube-selection-camera.json --output-dir outputs/scenario-cube-selection-camera

scenario-rose:
    test -f rose/raw.stl
    cargo run -p meshmend -- render rose/raw.stl --output outputs/scenario-rose-load-render/rose-loaded.png --state outputs/scenario-rose-load-render/state.json --width 1600 --height 1000 --view rendered

control-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p meshmend
    mkdir -p outputs/control-smoke target/meshmend-control
    sock="target/meshmend-control/smoke.sock"
    rm -f "$sock"
    log="outputs/control-smoke/app.log"
    target/debug/meshmend --control-socket "$sock" --replace-control-socket fixtures/stl/cube_binary.stl >"$log" 2>&1 &
    pid=$!
    cleanup() {
        kill "$pid" >/dev/null 2>&1 || true
        wait "$pid" >/dev/null 2>&1 || true
        rm -f "$sock"
    }
    trap cleanup EXIT
    for _ in {1..80}; do
        [[ -S "$sock" ]] && break
        sleep 0.1
    done
    [[ -S "$sock" ]] || { echo "socket was not created"; cat "$log" || true; exit 1; }
    target/debug/meshmend control --socket "$sock" state > outputs/control-smoke/01-state.json
    target/debug/meshmend control --socket "$sock" pan 25 10 > outputs/control-smoke/02-pan.json
    target/debug/meshmend control --socket "$sock" zoom 0.5 > outputs/control-smoke/03-zoom.json
    target/debug/meshmend control --socket "$sock" screenshot outputs/control-smoke/04-live.png > outputs/control-smoke/04-screenshot.json
    target/debug/meshmend control --socket "$sock" preview-cut 500 120 780 680 > outputs/control-smoke/05-preview.json
    target/debug/meshmend control --socket "$sock" apply-cut > outputs/control-smoke/06-apply.json
    target/debug/meshmend control --socket "$sock" select-object 0 > outputs/control-smoke/07-select-object.json
    target/debug/meshmend control --socket "$sock" export-visible outputs/control-smoke/live-cut.stl > outputs/control-smoke/08-export.json
    jq -e '.ok == true and .state.triangles > 0' outputs/control-smoke/01-state.json >/dev/null
    jq -e '.ok == true' outputs/control-smoke/08-export.json >/dev/null
    test -s outputs/control-smoke/04-live.png
    test -s outputs/control-smoke/live-cut.stl
    cargo run -p meshmend -- analyze outputs/control-smoke/live-cut.stl --output outputs/control-smoke/live-cut-analysis.json >/dev/null

smoke:
    cargo run -p meshmend -- --smoke-window

verify-rose:
    test -f rose/raw.stl
    cargo run -p meshmend -- inspect rose/raw.stl --parallel
    cargo run -p meshmend -- --verify-render rose/raw.stl

repair-smoke:
    just worker-build
    cargo run -p meshmend -- hole-fill fixtures/stl/cube_missing_top.stl --output outputs/cube-missing-top-filled.stl
    cargo run -p meshmend -- analyze outputs/cube-missing-top-filled.stl --output outputs/cube-missing-top-filled-analysis.json
    cargo run -p meshmend -- local-wrap fixtures/stl/cube_binary.stl --output outputs/cube-local-wrap.stl --voxel-size 0.08
    cargo run -p meshmend -- analyze outputs/cube-local-wrap.stl --output outputs/cube-local-wrap-analysis.json
    cargo run -p meshmend -- cut fixtures/stl/cube_binary.stl --output outputs/cube-cut-x-positive.stl --normal 1 0 0 --offset 0 --keep positive
    cargo run -p meshmend -- analyze outputs/cube-cut-x-positive.stl --output outputs/cube-cut-x-positive-analysis.json
    cargo run -p meshmend -- remesh fixtures/stl/cube_binary.stl --output outputs/cube-remesh.stl --target-edge-length 0.75
    cargo run -p meshmend -- analyze outputs/cube-remesh.stl --output outputs/cube-remesh-analysis.json
    cargo run -p meshmend -- export outputs/cube-remesh.stl --output outputs/cube-export.stl --report-json outputs/cube-export-report.json --report-md outputs/cube-export-report.md

perf path:
    mkdir -p outputs
    stem="$(basename "{{path}}" .stl)"; cargo run -p meshmend -- perf "{{path}}" --output "outputs/perf-${stem}.json"

clean:
    rm -rf outputs/*

worker-build:
    if [[ -f workers/cpp/CMakeLists.txt ]]; then cmake -S workers/cpp -B target/workers/cpp -DCMAKE_BUILD_TYPE=Release && cmake --build target/workers/cpp; else echo "workers/cpp will be added in Phase 7"; fi
