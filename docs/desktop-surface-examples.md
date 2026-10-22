# BigHead Desktop Surface Examples

These examples use the current `render_status` protocol. The desktop app stays
a renderer and input client: it displays these objects and sends clicks back as
`status_event`; the agent still decides what the click means.

## Pattern: Re-entry Home

Use when the user opens the desktop app after being away.

```json
{
  "op": "set_state",
  "state": {
    "title": "暖茶 continuation — 客厅夜灯下",
    "subtitle": "re-entry moment, waiting for a direction",
    "summary": "Big Head is holding the active thread and a short A/B/C choice.",
    "items": [
      {
        "id": "ambient-reentry",
        "title": "Reconnect the current thread",
        "state": "running",
        "control": "progress",
        "group": "progress",
        "description": "Verify anchors, summarize current state, and surface choices.",
        "progress": 0.55,
        "priority": 40
      },
      {
        "id": "choice-a-continue",
        "title": "Continue current implementation",
        "state": "pending",
        "control": "item",
        "group": "choice",
        "description": "Keep moving on the current development slice.",
        "priority": 30
      },
      {
        "id": "choice-b-review",
        "title": "Review what changed",
        "state": "pending",
        "control": "item",
        "group": "choice",
        "description": "Summarize commits, risks, and next checks.",
        "priority": 20
      }
    ],
    "messages": [
      {
        "id": "desktop-reentry-msg",
        "role": "assistant",
        "text": "I found the active thread and prepared a compact choice surface.",
        "created_at": 1786298400
      }
    ]
  }
}
```

## Pattern: Work Queue

Use when the user asks what is moving or wants progress they can inspect.

```json
{
  "op": "set_state",
  "state": {
    "title": "BigHead desktop app",
    "subtitle": "UI polish and release-readiness",
    "items": [
      {
        "id": "desktop-chat",
        "title": "Conversation readability",
        "state": "done",
        "control": "checkbox",
        "group": "checklist",
        "checked": true,
        "description": "Markdown bubbles and local waiting feedback.",
        "priority": 50
      },
      {
        "id": "desktop-diagnostics",
        "title": "Local diagnostics",
        "state": "running",
        "control": "progress",
        "group": "progress",
        "description": "Expose send/wait/runtime events without polluting chat.",
        "progress": 0.7,
        "priority": 40
      },
      {
        "id": "desktop-attachments",
        "title": "Attachment tray",
        "state": "pending",
        "control": "checkbox",
        "group": "checklist",
        "checked": false,
        "description": "Drag, paste, and send image/file context.",
        "priority": 30
      }
    ]
  }
}
```

## Pattern: Durable Note

Use when content should be read again later. Do not bury it only in chat.

```json
{
  "op": "upsert_document",
  "document": {
    "id": "doc-desktop-release-readiness",
    "title": "Desktop release-readiness checklist",
    "subtitle": "Current product gaps and next implementation focus",
    "kind": "plan",
    "text": "## Current state\n\n- Runtime connection works.\n- Spark input works.\n- Status items, choices, checklist, progress, inspector, and diagnostics exist.\n\n## Next\n\n1. Attachment tray.\n2. Visual regression checks.\n3. Stronger UI publishing examples.\n4. Packaging and first-run validation.",
    "references": [
      {
        "kind": "document",
        "id": "docs/bighead-ambient-desktop-goal.md",
        "title": "Ambient desktop goal"
      }
    ],
    "updated_at": 1786298400
  }
}
```

## Pattern: Click Feedback

When a desktop `status_event` arrives, do not treat it as a raw log. Respond by
inspecting or updating the surface.

```json
{
  "op": "append_message",
  "message": {
    "id": "desktop-click-feedback-001",
    "role": "assistant",
    "text": "Opened `desktop-diagnostics`. I will inspect recent runtime events and update the work queue if there is a blocker.",
    "created_at": 1786298400
  }
}
```

Then update the clicked item if the state changed:

```json
{
  "op": "upsert_item",
  "item": {
    "id": "desktop-diagnostics",
    "title": "Local diagnostics",
    "state": "done",
    "control": "progress",
    "group": "progress",
    "description": "Activity strip and expandable local event log are available.",
    "progress": 1.0,
    "priority": 40
  }
}
```

## Pattern: Attachment Spark

Desktop command bar input may include local file/image attachments. The app
passes metadata only; the agent should read or inspect the file path when
needed.

```json
{
  "surface": "desktop",
  "type": "spark",
  "text": "帮我看看这张截图里 UI 哪里不对",
  "source_client": "macos_app",
  "input_surface": "desktop_command_bar",
  "attachments": [
    {
      "id": "att-1",
      "name": "clipboard-1786298400.png",
      "path": "/Users/me/Library/Application Support/BigHead/Attachments/clipboard-1786298400.png",
      "mime_type": "image/png",
      "byte_count": 12345
    }
  ]
}
```

Good response pattern:

```json
{
  "op": "append_message",
  "message": {
    "id": "desktop-attachment-received-001",
    "role": "assistant",
    "text": "收到 1 个附件。我会先检查 `clipboard-1786298400.png`，然后把发现更新到桌面状态里。",
    "created_at": 1786298400
  }
}
```

## Hard Rules

- `progress` is a JSON number from `0.0` to `1.0`, never a string like `"70%"`.
- `checked` is a JSON boolean, never `"true"` or `"false"`.
- Attachment payloads contain metadata and local paths, not inline file bytes.
- Keep titles short enough for a desktop list.
- Use `append_message` for short feedback.
- Use `upsert_document` for durable summaries, plans, and reusable notes.
- Use stable ids so later updates replace the same item/document.
