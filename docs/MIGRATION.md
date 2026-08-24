# V3 migration notes

V3 replaces the former C#/.NET 10 WPF implementation with Rust, `egui`/`eframe`, and direct Win32 APIs. Git history retains the previous source commits, but no active C# project/source/build output remains in the V3 tree.

The known-good PowerShell helper is intentionally preserved under `legacy\powershell`. New PowerShell scripts are optional embedded diagnostics; the Rust application does not require them for core operation.

The Rust protection owner extends the proven `ROBLOX_singletonMutex` technique by also occupying `ROBLOX_singletonEvent` with a native mutex for current-client compatibility. Both handles are all-or-nothing and remain on one owner thread until multi-account mode is disabled or the GUI exits. Real multi-client behavior remains a required manual validation because Roblox's mechanism is undocumented and changes over time.
