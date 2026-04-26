# Vendored Codex Snapshot

This directory contains vendored upstream Codex source. Treat it as read-only for
ChatCodex product work.

ChatCodex-owned code must live outside this directory. If Codex behavior needs
customization, add a first-party adapter under `crates/` or copy the minimal code
with attribution and maintain it as ChatCodex-owned code.

Do not patch this tree for ChatCodex features. Updating Codex should mean
replacing this snapshot with a new upstream snapshot and rebuilding ChatCodex.
