# Win+E startup diagnosis
Build only in an isolated Windows test account: `cargo build --locked --release --features win-e-trace`.
This diagnostic is not the release binary. Keep the original setup and portable files unmodified.
Use the normal installer/enrollment path in a disposable test session. Confirm the shortcut is enrolled before restarting the test machine.
Capture one clean trace for the first actual Win+E after sign-in, then separate captures with a normal and elevated foreground app, and with an existing BetterExplorer window. Observe which app opens. Do not substitute SendInput for the shortcut: the hook intentionally ignores injected input.
Compare files from the same boot in `%LOCALAPPDATA%\BetterExplorerWinETrace`. Use the filename PID and `spawn_pid` to join listener and child records; the QPC difference times 1000 divided by the header frequency gives milliseconds. Sort by QPC rather than file order. At most one QPC tick across threads can be ambiguous.
- No `listener_ready`: inspect scheduled task process start and startup eligibility. Readiness alone does not prove a key was intercepted.
- `hook_recognized` followed by `mask_sent` below two: the OS gets the shortcut. That counter does not identify UIPI; compare the controlled foreground cases. A `queue` value of zero or minus one is a rejected request, not an app startup.
- `worker_received` to `reuse` shows the focus/reuse path. A true reuse does not create a second process.
- `spawn_started`, `spawn_pid` or `spawn_error`, `ui_entered`, and `window_shown` divide process creation and window startup. A window marked shown is not proof of presentation by the compositor or a populated listing. Missing trace data can also reflect dropped diagnostics and is not itself proof of slower startup.
- On a disposable test machine, capture an independent boot trace with [Windows Performance Recorder](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/wpr-command-line-options#boottrace) if available. Use process creation and its PID as a cross-check, and observe the visible result independently.
The trace captures no arbitrary keystrokes, window titles, paths from the app, or filenames from the app. Remove diagnostic TSV files after analysis and restore the standard build.
