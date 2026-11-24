# Benchmarks

Build from a Windows x64 shell with the MSVC toolchain:

    cargo build --release --locked
    powershell -NoProfile -File tools/bench.ps1 -IdleSeconds 30

The runner creates a small directory and a directory with 100,000 zero-byte text files under a temporary fixture. It runs one unmeasured launch before 30 measured launches and one unmeasured navigation before 30 measured navigations. The output goes to `bench/out/`.

The app measures startup from the Windows process creation timestamp to showing and painting the window. In navigation it measures wall time from the UI navigation action to the first populated frame and to the complete sorted frame. Paint is flushed with DWM. The recorded dispatch metric is the longest sampled processing time of a scan-result event for each navigation. It does not measure physical input latency.

The optional idle sample records process CPU time after a five-second settle period. Percentiles use nearest rank. Cold boots, controlled Windows File Explorer comparisons and transfer-throughput benchmarks are pending.

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
