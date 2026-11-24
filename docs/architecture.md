# Architecture

BetterExplorer targets Windows 11 x64. The reference machine runs Windows 11 Pro 25H2 build 26200.9974 with the German Windows interface. The application uses its own Better series branding, a native command bar and a virtual Win32 list view.

- The window thread owns controls, navigation history, selection and visible data. It never enumerates directories or runs a transfer.
- Enumeration workers publish the first results in small batches, then produce the sorted snapshot. Navigation supersedes old requests. An in-flight kernel call may finish before cancellation takes effect.
- The file list uses `LVS_OWNERDATA`. The application supplies text and icons for the requested rows. Wheel motion uses a timer only while scrolling and clamps the icon view to its native origin. Folder notifications refresh the layout without discarding the visible rows first. Grouping and tile view are not built into this mode.
- A separate COM apartment runs Windows Shell operations. The operating system handles conflict prompts, progress and Recycle Bin integration.
- An explicitly opened shell destination can use File Explorer for Windows-managed virtual locations. The default Windows shell remains installed and available.
- Store per-user settings locally and keep diagnostic data opt-in. No network services or telemetry are bundled.

## Win+E

The Settings tab, setup and uninstall share the executable's shortcut controls. The opt-in uses a single per-user startup entry. A separate message-loop process handles Win+E when enabled. The file browser stays out of the input callback, and key presses are not stored. The listener stops when the option is turned off. The standard Windows handling applies whenever the helper is not running.

## Performance targets

| Metric | Target |
| --- | --- |
| Warm launch to responsive window | p95 ≤ 150 ms |
| Warm navigation to first available rows | p95 ≤ 100 ms |
| UI thread response during regular local navigation | ≤ 100 ms |
| Idle CPU over 30 minutes | < 0.5% of one logical core |
| Large directory | 100,000 entries with a responsive window |

Use a release binary and a generated local fixture. Measure warm launch separately from navigation. Cold starts, Explorer comparisons and transfer integrity require controlled Windows runs. See [benchmarks](benchmarks.md).

## Windows API references

- [Virtual list views](https://learn.microsoft.com/en-us/windows/win32/controls/list-view-controls-overview)
- [Windows 11 Details pane screenshots](https://www.anoopcnair.com/show-details-pane-in-file-explorer-windows-11/)
- [Windows low-level keyboard callback](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
- [IFileOperation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation)
