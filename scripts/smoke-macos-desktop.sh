#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> Swift desktop build"
swift build --package-path external/big-head-menubar

echo "==> Desktop status fixture"
cargo test -p big-head-view desktop_status_smoke_fixture_decodes_and_covers_native_groups

echo "==> Desktop spark attachment routing"
cargo test -p big-head-view parse_status_spark_event_preserves_attachments

echo "==> Full view protocol smoke"
cargo test -p big-head-view

echo "==> Done"
echo "Next visual step: launch the app with sample status, capture a screenshot, and compare it against a blessed artifact."
