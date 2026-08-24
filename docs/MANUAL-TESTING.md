# Manual testing checklist

Automated checks do not launch/close Roblox, touch the real `RobloxCookies.dat`, modify the registry, mutate Fishstrap/Bloxstrap, or perform account login/logout. The tests below are **not completed** until Wolfy performs them.

## A. Multi-instance (required)

1. Start `dist\RobloxMultiAccountLauncher.exe` before Roblox.
2. Enable Multi-Account Mode and wait until Diagnostics shows:
   - `singletonMutex: HELD`
   - `singletonEvent compatibility mutex: HELD`
3. Launch Client 1 with the single **Launch Roblox** action.
4. Launch Client 2 with the same action.
5. Verify both clients remain open and the launcher reports a genuinely new stable PID for each launch.
6. Optionally launch Client 3.
7. Verify no existing client closes. If one does, capture Diagnostics and note whether an installer/update process was reported.

## B. Teleport (required)

1. Confirm Teleport protection shows **Protected**.
2. Run two differently authenticated clients.
3. Enter the same experience/game.
4. Perform an actual cross-place or cross-server teleport.
5. Verify both clients survive and remain usable.

## C. Shared logout (required; not claimed fixed)

1. Run two differently authenticated clients.
2. Log out from Client 2.
3. Observe Client 1 while it remains inside an experience.
4. Leave Client 1's experience and return to the Roblox desktop app.
5. Verify whether Client 1 remains authenticated.

Current expected product status is **Login-state isolation: Unsupported / not enabled**. Do not mark this issue fixed unless this test passes after a specifically validated, narrow protection mechanism is implemented.

## D. Metadata-only logout investigation

1. Open Diagnostics and select **Start Local-State Trace**.
2. Confirm the displayed log path is under `%LOCALAPPDATA%\RobloxMultiAccountLauncher\traces`.
3. Perform test C.
4. Select **Stop Trace**.
5. Review only file names/actions/timestamps. Confirm no file contents, credentials, cookie values, authentication tickets, or passwords appear.
6. Correlate repeated create/write/delete/rename events with the logout moment before proposing any experimental file lock.

The tracer cannot attribute the writer process through `ReadDirectoryChangesW`; entries therefore record `process=unavailable` rather than guessing.

## E. GUI and integration

1. Confirm no console/CMD/PowerShell window appears.
2. Verify dark-theme readability at 100%, 125%, 150%, and normal DPI.
3. Verify the only primary launch action is **Launch Roblox**.
4. Verify stock Roblox launching in normal mode.
5. Verify Fishstrap launching.
6. Verify Bloxstrap launching if available.
7. Verify Auto fallback order: Fishstrap, Bloxstrap, stock Roblox.
8. Enable with Roblox already running; verify explicit close approval and bounded graceful/force-close behavior.
9. Verify setup-failure and Retry behavior, including disable/re-enable handle reacquisition.
10. Verify teleport Warning when the cookie lock is unavailable without exposing file contents.
11. Verify protection-loss launch blocking where safely reproducible.
12. Verify 50/50, 70/30, vertical, swap, focus, and preferred-monitor behavior.
13. Verify tray open, launch, tile, focus, close, disable, and exit actions.
14. Verify protection-aware disable/window-close/exit warnings.
15. Verify PowerShell Assist with PowerShell 7, Windows PowerShell compatibility where practical, and Rust-only mode.
16. Verify copied diagnostics contain no sensitive values.
