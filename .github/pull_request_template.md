## Summary

Describe the focused change and why it is needed.

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --locked`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo test --locked`
- [ ] Relevant safe manual test documented

## Safety and scope

- [ ] No credentials, cookies, authentication tickets, private links, secrets, or machine-specific dumps were added.
- [ ] No injection, memory access/modification, hooks, anti-cheat manipulation, process hiding, or Roblox file modification.
- [ ] No C#/.NET, Node, Electron, WebView, JVM, browser runtime, or other unapproved runtime dependency.
- [ ] Critical mutex/event/cookie ownership was not changed, or the reason and validation are explicit.
- [ ] No unrelated refactor or generated artifact is included.
