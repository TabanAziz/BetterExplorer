# Benchmarks

Build from a Windows x64 shell with the MSVC toolchain.

    cargo build --release --locked
    powershell -NoProfile -File tools/bench.ps1 -IdleSeconds 30

To measure the exact GitHub release, extract the portable ZIP and pass the executable path.

    powershell -NoProfile -File tools/bench.ps1 -BinaryPath C:\path\to\BetterExplorer.exe -IdleSeconds 30

The runner creates a small directory and a directory with 100,000 zero-byte text files under a temporary fixture. It runs one unmeasured launch before 30 measured launches and one unmeasured navigation before 30 measured navigations. The output goes to `bench/out/`.

The app measures startup from the Windows process creation timestamp to the first recorded app paint flushed with DWM. Navigation starts at the UI navigation action and records the first populated frame and the complete sorted frame. Scan-result dispatch measures the longest sampled scan-result handler per navigation. These timings cover the application paths and exclude physical input latency.

The optional idle sample records CPU time of the open file manager after a five-second settle period. It does not include the optional Win+E helper. Percentiles use nearest rank. Cold starts, controlled Windows File Explorer comparisons, and transfer throughput are pending.

## Results for 0.1.7

Measured on November 24, 2026, on Windows 11 25H2 build 26200.9974, Intel Core i7-11800H, 16 logical processors, and a local NTFS volume. The benchmark requested no foreground activation. This run used the optimized x64 portable release binary, with build paths remapped to `/build` and static CRT linking.

| Measurement | Runs | p50 | p95 | p99 |
| --- | ---: | ---: | ---: | ---: |
| Warm launch to first recorded DWM-flushed paint | 30 | 104.4 ms | 124.1 ms | 125.8 ms |
| First entries in the 100,000-file folder | 30 | 55.6 ms | 69.3 ms | 69.5 ms |
| Complete sorted 100,000-file folder | 30 | 208.1 ms | 256.9 ms | 263.5 ms |
| Longest scan-result dispatch per navigation | 30 | 5.3 ms | 6.9 ms | 8.5 ms |

This run meets the 150 ms p95 first-paint target and the 100 ms p95 first-entries target. The idle sample recorded 0 ms of process CPU time over 30.015 seconds, at the timer's resolution. Results can vary with background activity, hardware, and Windows caching.

[Raw samples and executable hash](bench-data/v0.1.7)

## Results for 0.1.0

Windows 11 Pro 25H2 build 26200.9974, Intel Core i7-11800H, 16 GB RAM, local NTFS, 1920 × 1080 at 100% scaling and 144 Hz. Rust 1.99.1 with the MSVC x64 release profile. The benchmark windows requested no foreground activation. The executable is 1.16 MiB.

| Measurement | Runs | p50 | p95 | p99 |
| --- | ---: | ---: | ---: | ---: |
| Warm launch to first paint from the message loop | 30 | 83.7 ms | 118.2 ms | 212.1 ms |
| First entries in the 100,000-file folder | 30 | 48.6 ms | 62.6 ms | 84.0 ms |
| Complete sorted 100,000-file folder | 30 | 173.5 ms | 291.1 ms | 291.4 ms |
| Longest scan-result dispatch per navigation | 30 | 2.9 ms | 4.4 ms | 4.8 ms |

Warm launch was within the 150 ms p95 target. First entries were within the 100 ms p95 navigation target.

The idle sample measured 0.0 ms of CPU time over 30.0 s, or 0.000% of one logical core.

The settings and hardware were not varied during this run. Raw samples and the binary hash are in [bench-data](bench-data).
