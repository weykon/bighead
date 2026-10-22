#!/usr/bin/env bash
set -euo pipefail

# Small local client for the Big Head A2A bridge.
# Override A2A_URL when the bridge uses a non-default port.
A2A_URL="${A2A_URL:-http://127.0.0.1:3040}"

usage() {
  cat <<'EOF'
Usage:
  scripts/a2a.sh card
  scripts/a2a.sh health
  scripts/a2a.sh send "message text" [intent]
  scripts/a2a.sh task TASK_ID

Environment:
  A2A_URL   Bridge base URL (default: http://127.0.0.1:3040)
EOF
}

command="${1:-}"
case "$command" in
  card)
    exec curl --fail --silent --show-error "$A2A_URL/.well-known/agent-card.json"
    ;;
  health)
    exec curl --fail --silent --show-error "$A2A_URL/health"
    ;;
  task)
    task_id="${2:-}"
    if [[ -z "$task_id" ]]; then
      usage >&2
      exit 2
    fi
    exec curl --fail --silent --show-error "$A2A_URL/a2a/tasks/$task_id"
    ;;
  send)
    text="${2:-}"
    intent="${3:-research}"
    if [[ -z "$text" ]]; then
      usage >&2
      exit 2
    fi
    # Python is used only for JSON escaping, so quotes and non-ASCII prompts
    # are transmitted exactly as entered.
    payload="$(TEXT="$text" INTENT="$intent" python3 - <<'PY'
import json
import os

print(json.dumps({
    "from": "codex",
    "to": "*",
    "kind": "request",
    "intent": os.environ["INTENT"],
    "text": os.environ["TEXT"],
    "permissions": {
        "can_observe": True,
        "can_plan": True,
        "can_mutate": False,
        "can_execute": False,
        "can_forward": False,
    },
}))
PY
)"
    exec curl --fail --silent --show-error \
      -X POST "$A2A_URL/a2a/messages" \
      -H 'content-type: application/json' \
      --data-binary "$payload"
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
