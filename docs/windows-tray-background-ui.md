# Windows Tray and Background Control Center

## Behavior

- Production Control Center launches without a terminal window.
- A Microgifter HomeServer icon remains in the Windows system tray while the Control Center UI is running.
- Left-clicking the tray icon restores and focuses the current Control Center page.
- The tray menu opens Dashboard, Agent Chat, HomeServer Status, checks for signed updates, or explicitly exits the Control Center UI.
- Closing the main window hides it to the system tray instead of terminating the UI process.
- The LocalSystem HomeServer service continues independently when the Control Center is hidden or explicitly exited.
- Settings includes an optional **Start Control Center with Windows** toggle.
- Windows autostart launches the Control Center hidden so the tray icon is available without opening the dashboard.

No HomeServer service, pairing, provider, model, backup, MCP, or local-data behavior is changed.

## Validation

The focused Windows implementation passed frontend validation, the production frontend build, Rust formatting, native Control Center compilation, and strict Clippy. Full Production Quality and Cloud Connector validation are running on the clean product source.