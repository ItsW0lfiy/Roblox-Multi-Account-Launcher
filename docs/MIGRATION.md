# V3 migration notes

V3 replaces the former C#/.NET 10 WPF implementation with Rust, `egui`/`eframe`, and direct Win32 APIs. Git history retains the previous source commits, but no active C# project/source/build output remains in the V3 tree.

The known-good PowerShell helper is intentionally preserved under `legacy\powershell`. New PowerShell scripts are optional embedded diagnostics; the Rust application does not require them for core operation.

The Rust protection owner extends the proven `ROBLOX_singletonMutex` technique by also occupying `ROBLOX_singletonEvent` with a native mutex for current-client compatibility. Both handles are all-or-nothing and remain on one owner thread until multi-account mode is disabled or the GUI exits. Two-client coexistence, teleporting, and bidirectional logout behavior were subsequently validated manually three times on the validation machine.

Repeatedly launching the same exact client executable path still failed to create Client 2. Multi-account launches now allocate native NTFS junction aliases under the application runtime directory and start `RobloxPlayerBeta.exe` through a unique `Client-NNNN` path. This path-isolation addition is automated-test complete but remains manual-validation pending until the second **Launch Roblox** click creates Client 2.
