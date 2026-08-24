# Legacy prototype

`legacy/powershell` preserves the previously validated PowerShell/Windows Forms prototype for audit history only. It is not referenced, embedded, launched, or required by the Rust application or build/release workflows. The active product has no C#/.NET application code and no Windows Forms dependency.

The prototype remains useful evidence for the known-good `ROBLOX_singletonMutex` and exclusive read-only `RobloxCookies.dat` behavior. Do not restore it as the product implementation.
