# Contributing

Bug reports and focused pull requests are welcome. Include the BetterExplorer version, Windows build, and steps using a small sample folder. Use sample files when sharing screenshots or logs.

## Build and check

Use Windows 11 x64, Rust stable, the MSVC build tools, and the Windows SDK.

    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo build --release --locked

Use the [test plan](docs/test-plan.md) when changing Windows integration and the [benchmark runner](docs/benchmarks.md) when changing startup, navigation, or painting. Update the [feature matrix](docs/parity-matrix.csv) with relevant implementation and test evidence.
