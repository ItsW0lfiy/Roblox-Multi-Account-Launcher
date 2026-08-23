# Security Policy

## Boundaries

Roblox Multi-Account Launcher is an external Windows launcher and process/window manager.

It does not:

- read or store Roblox cookie contents, passwords, authentication tickets, or account tokens;
- inject DLLs, hook Roblox, scan or modify process memory, patch binaries, or manipulate anti-cheat systems;
- hide processes or handles, install drivers, spoof hardware, or intercept Roblox network traffic;
- automate gameplay, account farming, quests, or anti-termination behavior;
- transmit telemetry, logs, diagnostics, or process information.

The teleport-protection handle opens `RobloxCookies.dat` read-only with exclusive sharing and never reads the stream. Automated tests use fixture files only.

## Diagnostics

Launcher logs are local under `%LOCALAPPDATA%\RobloxMultiAccountLauncher\Logs`. Copied diagnostics redact common cookies, tickets, tokens, bearer values, and private-server parameters. Logs are never uploaded automatically.

## Reporting security issues

This repository is currently local and unpublished. Do not place secrets in an issue or public message. If the project is published later, add a private security-reporting address or GitHub private vulnerability reporting instructions here before accepting reports.

## Supported versions

Only the current local V2 build is intended for evaluation. No long-term support policy is established yet.
