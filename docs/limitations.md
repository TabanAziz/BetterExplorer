# Known limitations

- The command bar, breadcrumbs, file list and built-in actions use native Windows controls. BetterExplorer has its own visual styling. The interface is currently in English.
- Recycle Bin, Network and Libraries open in Windows File Explorer. This preview does not embed those Shell namespaces or offer Explorer's undo/redo history. Home provides known and pinned folders. Recent items, file favorites and Gallery are pending.
- The virtual list has Details, List and icon views. Grouped files and the Explorer Tiles and Content layouts are pending. File list icons are generic. The Details pane previews supported local images and text after selection.
- Each file operation captures its target. A paste waiting in the queue may need to be repeated if another copy changes the clipboard before Windows starts it.
- The app does not host arbitrary Shell context-menu extensions, sharing handlers or OneDrive sync overlays. Windows file associations and properties remain available. A separate menu action shows the item in Windows File Explorer.
- Search matches file and folder names recursively, skips reparse points and may skip unreadable areas. It does not use Windows Search, search file contents or implement Advanced Query Syntax.
- Incoming Windows file drops copy to the active folder. Outbound and internal drag, device-specific namespace drops and drag modifier behavior are pending.
- ZIP supports unencrypted Stored and Deflate entries. The ZIP commands show completion or error, with no mid-operation progress or pause. Additional formats, passwords and restoration of archive metadata are pending. ZIP extraction always creates a fresh destination folder.
- Remote paths use a dedicated scanner when Windows identifies the path as UNC or mapped network. Windows may block a remote I/O call before cancellation takes effect. Only the active local folder has change notifications. Refresh network folders manually.
- A secondary window reads current preferences and has its own session. Only the primary window writes preferences to reduce conflicting changes. The optional startup restoration is per primary session.
- Shell integration tests cover local NTFS. Timing comparison against File Explorer, cold boots, SMB faults, long paths under all policies, cloud placeholder behavior, high DPI, multiple monitors and full Narrator and touch coverage are pending.
- The packages are unsigned. Installation is per user. BetterExplorer's optional Win+E helper runs in the current Windows desktop session. Apps that intercept the same key combination, Windows policies or an elevated foreground app can prevent the shortcut from being handled there. In those cases Windows can still handle the shortcut.
