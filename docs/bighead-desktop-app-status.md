# BigHead Desktop App Status

Last updated: 2026-08-11

Product direction and next-phase details live in
[`docs/bighead-ambient-desktop-goal.md`](bighead-ambient-desktop-goal.md).

## Current State

- Native macOS AppKit desktop app lives in `external/big-head-menubar`.
- The app opens a regular desktop window and a menu bar item.
- The app does not contain agent logic. It talks to the Rust runtime over the existing `big-head-view` HTTP/WebSocket protocol.
- Packaged builds include the Rust runtime binary at `BigHead.app/Contents/Resources/big-head-runtime`.
- The packaged app starts the bundled runtime with `--view-server --headless`.
- Runtime data lives in `~/Library/Application Support/BigHead`.
- The app has a Settings window for editing `MiniMax_API_KEY`.
- If the key is missing, the main window shows a visible warning and runtime auto-start is blocked.
- The desktop window now uses an Ambient multi-mode shell:
  - `Dwelling`
  - `Work`
  - `Chat`
  - `Notes`
- The top Ambient surface summarizes current title, summary, running/pending/done counts, and latest signal.

## Build

From the repo root:

```bash
scripts/package-macos-app.sh
```

Outputs:

- `dist/macos/BigHead.app`
- `dist/macos/BigHead-macOS.zip`

## Runtime Setup

Before first use, create:

```text
~/Library/Application Support/BigHead/.env
```

Example:

```text
MiniMax_API_KEY=your-key-here
```

## Known Distribution Gaps

- No Apple Developer signing yet.
- No notarization yet.
- No DMG installer yet.
- First-run onboarding is partial: missing API key opens Settings, but there is no provider picker yet.
- Friend/family distribution may require right-click Open because Gatekeeper will flag unsigned builds.

## Next Distribution Tasks

1. Add provider picker beyond MiniMax.
2. Add app icon and bundle metadata polish.
3. Add code signing.
4. Add notarization.
5. Generate a DMG installer.
6. Add update/version metadata.
