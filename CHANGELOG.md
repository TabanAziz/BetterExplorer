# Changelog

## 0.1.11

- Run the optional Win+E logon helper and its foreground launch at normal priority instead of inheriting background priority.
- Start the first folder scan ahead of other background services. Use bulk Windows enumeration for local folders and coalesce repeated list redraws. Defer large icon setup until an icon view uses it.

- Keep the Close button above the tabs and redraw it reliably during layout changes.
- Right-click a Quick access folder for actions including Copy as path, Open in new tab, and Properties.
- Win+E brings a responsive BetterExplorer window from the same installation forward when available. Ctrl+N still opens a new window.
- Show the first window without blocking on DWM composition. Reuse recent local folder listings while refreshing them in the background.

## 0.1.10

- Start the opted-in Win+E helper through a per-user sign-in task so it can run without waiting for delayed Startup Apps.
- Keep a single shortcut helper per Windows session.

## 0.1.9

- Keep navigation inside a manually opened Windows File Explorer window when BetterExplorer is the default folder manager.
- Show themed tooltips beside truncated file and folder names. Dismiss the tooltip when the pointer moves to another row or the view changes.

## 0.1.8

- Added a standalone Windows installer alongside the portable ZIP.
- Restore the Win+E listener on launch when this copy is already configured for the shortcut.
- Paint the first frame with the selected theme before revealing the window.
- Reduced work during file selection and dimmed cut items until the clipboard changes or the cut is completed.
- Added folder drop targets, standard drag modifiers, faster same-volume moves, and clearer errors for unavailable paste sources.
- Expanded the New menu with document and archive formats.
- Added direct ZIP, 7z, and TAR actions with automatic output names and optional compression settings.
- Added an optional default folder manager setting in BetterExplorer and Setup, separate from Win+E.

## 0.1.7

- Unified Win+E, date format, and nine theme previews in one scrollable Settings page.
- Reserved space for theme artwork in compact, multi-tab windows.
- Added Windows-indexed PC search, Quick access pinning, native file dragging, and per-window undo and redo since 0.1.1.
- Added Windows file type icons, adaptive date grouping, and saved window placement and size.
- Published the Windows x64 portable preview and repeatable performance measurements.

## 0.1.1

- Added a Settings tab for the optional Win+E shortcut.
- Added the same choice to interactive setup and per-user uninstall cleanup.

## 0.1.0

- Initial Windows 11 preview with native tabs, path navigation, virtual file views and name search.
- Added local drive cards, file metadata and supported image/text previews.
- Integrated Windows Shell file operations and safe ZIP creation/extraction.
- Added eased wheel scrolling and stable icon alignment at the top of the list.
- Preserved keyboard focus and previews during live folder refresh.
- Added repeatable Windows tests, benchmark scripts and portable and per-user packages.
