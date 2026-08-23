# Manual testing checklist

Automated validation deliberately does not launch or close Roblox and does not touch the real `RobloxCookies.dat`. None of these V2 checks should be marked complete until performed manually.

1. Launch the application normally.
2. Confirm no CMD/PowerShell window appears.
3. Verify the dark UI visually at normal Windows scaling.
4. Verify Fishstrap autodetection and quick actions.
5. Verify Bloxstrap autodetection if installed/testable.
6. Verify stock/default Roblox fallback.
7. Launch Roblox with multi-account mode off.
8. Enable multi-account mode with Roblox closed.
9. Enable it while Roblox is running and verify explicit close warning.
10. Launch two Roblox clients.
11. Perform another real multi-client teleport.
12. Verify 50/50 tiling.
13. Verify 70/30 tiling.
14. Verify vertical tiling.
15. Verify swapping clients.
16. Verify client focus actions and optional hotkeys.
17. Verify preferred-monitor movement on every available monitor.
18. Verify graceful Roblox close.
19. Verify approved force-close fallback.
20. Disable multi-account mode while clients are running.
21. Verify notification-area mode and menu actions.
22. Verify protection-aware exit warning.
23. Verify Fishstrap launch-failure diagnostics where practical.
24. Verify copied diagnostics contain no sensitive data.
25. Verify UI layout at 100%, 125%, 150%, and the user's normal DPI scaling.
