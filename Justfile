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

smoke:
    cargo run -p meshmend -- --smoke-window

verify-rose:
    test -f rose/raw.stl
    cargo run -p meshmend -- inspect rose/raw.stl --parallel
    cargo run -p meshmend -- --verify-render rose/raw.stl

perf path:
    mkdir -p outputs
    stem="$(basename "{{path}}" .stl)"; cargo run -p meshmend -- perf "{{path}}" --output "outputs/perf-${stem}.json"

clean:
    rm -rf outputs/*

worker-build:
    if [[ -f workers/cpp/CMakeLists.txt ]]; then cmake -S workers/cpp -B target/workers/cpp -DCMAKE_BUILD_TYPE=Release && cmake --build target/workers/cpp; else echo "workers/cpp will be added in Phase 7"; fi
