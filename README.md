<p align="center"><img src="assets/bex.png" width="116" alt="BetterExplorer icon"></p>

<h1 align="center">BetterExplorer</h1>

<p align="center">A fast native file manager for Windows 11, with familiar navigation and original Better series artwork.</p>

BetterExplorer is part of the Better series with [Better Task Manager](https://github.com/TabanAziz/BetterTaskManager) and [BetterClipboard](https://github.com/TabanAziz/BetterClipboard).

## Use

Run BetterExplorer.exe from the portable package or pass a folder path on the command line. To install for your Windows account, extract the install ZIP and open Setup.exe. Choose which app should open with Win+E, then install.

The default for a new installation is Windows File Explorer. Open the **Settings** tab from the command bar or with Ctrl+, to change the shortcut later. Choosing BetterExplorer starts a small background helper and sets it to run at sign-in. Choosing Windows File Explorer returns the shortcut immediately. Restart and administrator rights are unnecessary.

The same setup program accepts unattended options.

    .\Setup.exe -Quiet -StartMenuShortcut -WinE BetterExplorer

To remove the installed program, run Setup.exe from the extracted bundle with the uninstall option. Uninstall also removes BetterExplorer's own shortcut setting.

    .\Setup.exe --uninstall

- Open folders from Home or This PC, use the breadcrumb bar, or press Ctrl+L to enter a path. Home shows Quick access, Recent and Favorites.
- Network, Recycle Bin and Libraries open inside BetterExplorer. Use the Recycle Bin context menu to restore or permanently delete items.
- In Settings under **Date modified format**, choose DD.MM.YYYY or MM/DD/YYYY. Both include the local time.
- Ctrl+T and Ctrl+W manage tabs. Use More to reorder tabs and enable tab restoration.
- Wheel scrolling eases toward the new position, including the first row at the upper edge.
- View offers Details, List and four icon sizes. This PC shows available space on local fixed drives.
- File lists use the icons registered for file types in Windows. The icons load in the background and are reused for matching extensions.
- Folders open newest first, with files and subfolders together. Details groups entries by modification date. Use Sort by and Group by to change the folder preference.
- Right-click an item to find actions such as Rename and Properties. Right-click empty space for View, Sort by, Group by, New and Open in Terminal.
- Right-click a folder and select **Pin to Quick access** to show it on Home and in the sidebar.
- From Home or This PC, **Search this PC** finds file or folder name prefixes in the Windows index. Choose **Search all drives** to match text anywhere in a name across fixed and removable local drives.
- In a folder, Ctrl+F searches file and folder names below that folder. Escape or **Stop search** cancels an active search.
- The command bar and context menu handle copy, cut, paste, rename, Recycle Bin deletion, properties, shortcuts and ZIP files. Copy and move use Windows Shell dialogs.
- Drag files or folders from the list to the desktop, File Explorer or another app that accepts Windows file drops. An outgoing drop offers Copy.
- The Details button shows metadata and previews supported local images and text files. Ctrl+Shift+N creates a folder.

## Build

Windows 11 x64, Rust stable and the x64 MSVC build tools with the Windows SDK.

    cargo build --release --locked
    cargo test --locked
    cargo clippy --locked --all-targets -- -D warnings

Package the release with PowerShell.

    powershell -NoProfile -File tools\release.ps1

The application is built as `target\release\BetterExplorer.exe`. See the [test plan](docs/test-plan.md) and [benchmarks](docs/benchmarks.md) for repeatable checks.

## Preview limits

Gallery, Shell extensions, cloud overlays and advanced Search options are still outside this preview. The [parity matrix](docs/parity-matrix.csv) records the current state of individual features. See [known limitations](docs/limitations.md).

## License

MIT. The app and file-list artwork are original. Dependency notices are in [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES).
