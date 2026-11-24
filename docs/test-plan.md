# Test plan

Reference desktop is Windows 11 Pro 25H2 build 26200.9974, German UI, 1920 × 1080 at 100% scaling. App controls currently use English.

## Repeatable checks

    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo build --release --locked
    powershell -NoProfile -File tools/bench.ps1

- Core fixtures verify tab history, Windows filename validation, Unicode names, progressive reads and natural ordering.
- Shell integration tests use actual Windows COM operations in temporary folders. They compare bytes after copy and move, and reload a shortcut to check its destination.
- ZIP tests cover a round trip, Unicode, empty directories, existing destinations, duplicate or unsafe names, bad CRC, junctions and the Windows origin mark.
- The native icon-grid regression scrolls a real Windows list control, checks an intermediate position and verifies that the first row is fully visible at the upper edge.
- The benchmark exercises release startup and the same Win32 navigation and paint paths used in the app, with a generated 100,000-file NTFS folder.

## Manual desktop checks

Use a disposable fixture folder for write tests.

1. Open a folder from This PC and inspect disk capacity. Open a folder from a breadcrumb, switch back and forward, and reopen a tab.
2. Change between Details, List and icon sizes. Sort each column. Scroll to the end of the icon view and then back to the first row. Try with an empty folder and a long filename.
3. Search a nested fixture by name, then stop an active search. Toggle Hidden items. Check keyboard focus and the details pane.
4. Create a text file, rename it, copy and cut using the Windows clipboard, then paste into a second test folder. Test a name conflict, cancel and verify source and destination. Test the Recycle Bin and restore through the Shell.
5. Create a ZIP, extract it and compare its contents. Confirm that selecting an existing destination leaves it intact.
6. Repeat navigation and opening files in light/dark, contrast, 100/125/150/200% DPI and on two monitors. Use Narrator. Record separate outcomes per configuration.
7. Compare launch and first content on a clean Windows build. Compare hashes, timestamps and throughput for large copies. Test disconnected SMB, inaccessible folders, encrypted files and cloud placeholders in a test environment.

## Win+E

On a disposable Windows account, install with Windows File Explorer selected and verify that Win+E opens Windows. Enable BetterExplorer in Settings, close its window and press Win+E on both sides of the keyboard. BetterExplorer should open once per press and the Start menu should stay closed. Ordinary typing, Win by itself, extra-modifier chords and all other Windows shortcuts should behave normally. Sign out and back in to verify the startup option. Switch back in Settings, then verify the Windows shortcut again. Repeat an upgrade and uninstall with the helper enabled.

See the [parity matrix](parity-matrix.csv) for recorded results and open verification.
