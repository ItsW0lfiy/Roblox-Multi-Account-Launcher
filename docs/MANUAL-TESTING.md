# Manual testing checklist

Automated checks do not launch/close Roblox, touch the real `RobloxCookies.dat`, modify the registry, mutate Fishstrap/Bloxstrap, or perform account login/logout. Results below record Wolfy's repeated manual validation separately from the remaining per-instance launch test.

## A. Multi-instance

**Validated externally:** two clients coexist while the Rust protections are active. Repeatedly opening the same exact Roblox executable path does not create Client 2.

**Still required for per-instance path isolation:**

1. Start `dist\RobloxMultiAccountLauncher.exe` before Roblox.
2. Enable Multi-Account Mode and wait until Diagnostics shows:
   - `Shared singleton mutex: HELD`
   - `Shared singleton event: HELD`
   - `Per-instance path isolation: READY`
3. Launch Client 1 with the single **Launch Roblox** action.
4. Launch Client 2 with the same action.
5. Verify both clients remain open and the launcher reports a genuinely new stable PID for each launch.
6. Optionally launch Client 3.
7. Verify no existing client closes. If one does, capture Diagnostics and note whether an installer/update process was reported.

## B. Teleport

**Passed:** two differently authenticated clients survived a real multi-client teleport. Repeat after the first successful in-launcher Client 2 launch to validate the combined path-isolation flow.

1. Confirm Teleport protection shows **Protected**.
2. Run two differently authenticated clients.
3. Enter the same experience/game.
4. Perform an actual cross-place or cross-server teleport.
5. Verify both clients survive and remain usable.

## C. Shared logout

**Passed three times, bidirectionally:** logging out Client 1 did not log out Client 2, and logging out Client 2 did not log out Client 1, while the external protections were active.

1. Run two differently authenticated clients.
2. Log out from Client 2.
3. Observe Client 1 while it remains inside an experience.
4. Leave Client 1's experience and return to the Roblox desktop app.
5. Verify Client 1 remains authenticated.
6. Repeat in the other direction.

Current product status is **Experimental / manually validated**. No separate auth-state file protection or credential handling is claimed.

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
15. Verify PowerShell Assist with PowerShell 7 and Rust-only mode when PS7 is absent. Confirm Windows PowerShell 5.1 is not selected.
16. Verify copied diagnostics contain no sensitive values.
17. Verify per-client audio, resource presets/restoration, role swapping, auto-layout, tray mute/resource actions, and optional hotkeys.
18. Verify the soft client limit and optional one-click target count stop on success, limit, or first failure.
19. Verify `portable.flag` uses a sibling `data` directory, then test settings export/import/reset and a sanitized support bundle.
20. Verify the recovery view identifies windowless/related processes and never removes aliases while Roblox is running.
21. Validate update check/download/restart/rollback only after a real matching GitHub prerelease exists; never interrupt protected Roblox sessions.
