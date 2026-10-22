# BigHead Desktop

Native macOS desktop + menu bar client for Big Head.

It does not contain agent logic. Big Head owns the world, reasoning, and status
decisions; this app renders `/api/status`, subscribes to `/status/ws`, and sends
user input back as normal Spark events.

The main window is an Ambient multi-mode shell:

- an Ambient top surface for the current title, summary, counts, and latest
  signal
- `Dwelling`, `Work`, `Chat`, and `Notes` mode switches
- a status board for agent-published work items, progress rows, and checkboxes
- a local status inspector that opens when a status item is clicked
- a chat transcript for desktop Spark input, runtime feedback, and final agent
  replies
- an activity strip above conversation for local send/wait/runtime diagnostics

The product direction and protocol gaps are tracked in
`docs/bighead-ambient-desktop-goal.md`.

Reusable `render_status` patterns for agent-authored desktop UI are tracked in
`docs/desktop-surface-examples.md`.

## Run

### Development

From the repo root, run:

```bash
swift run --package-path external/big-head-menubar BigHeadMenuBar
```

The app opens a native window and a menu bar item. If no runtime is listening on
`127.0.0.1:3030`, it starts:

```bash
cargo run -- --view-server --headless
```

You can also start the runtime manually, then launch the app:

```bash
cargo run -- --view-server --headless
swift run --package-path external/big-head-menubar BigHeadMenuBar
```

### Package

From the repo root:

```bash
scripts/package-macos-app.sh
```

The script creates:

- `dist/macos/BigHead.app`
- `dist/macos/BigHead-macOS.zip`

The packaged app contains `target/release/big-head` as
`Contents/Resources/big-head-runtime`, so the recipient does not need Rust or
Cargo installed.

### Smoke Test

From the repo root:

```bash
scripts/smoke-macos-desktop.sh
```

This builds the Swift desktop app and runs the view protocol smoke checks for:

- desktop status fixture decoding
- native status groups: `choice`, `checklist`, `progress`
- desktop Spark attachment forwarding
- full `big-head-view` test suite

It is not yet a screenshot-based visual regression test. That should be added
as the next release-readiness gate.

Runtime data is stored outside the app:

```text
~/Library/Application Support/BigHead
```

The app has a Settings window for editing the MiniMax key. You can also create
the file manually:

```text
~/Library/Application Support/BigHead/.env
```

with:

```text
MiniMax_API_KEY=your-key-here
```

Ask the agent to publish status items, or call the `render_status` tool from a
task. The app hydrates/subscribes from:

- `GET http://127.0.0.1:3030/api/status`
- `WS  ws://127.0.0.1:3030/status/ws`

The desktop input box sends:

```json
{
  "type": "spark",
  "text": "帮我整理今天要做的事",
  "source_client": "macos_app",
  "input_surface": "desktop_command_bar"
}
```

The Rust server routes it into the normal Spark queue, so the agent still owns
all reasoning and ECS writes.

## Keyboard

- `Command+1`: Dwelling
- `Command+2`: Work
- `Command+3`: Chat
- `Command+4`: Notes
- `Command+L`: focus the command bar
- `Command+R`: refresh status
- `Return`: send the command bar text
- `Shift+Return`: insert a newline in the command bar

## Attachments

The desktop command bar supports local attachments:

- click `Attach` to select files
- drag files onto the command bar
- paste an image from the clipboard; the app stores it under
  `~/Library/Application Support/BigHead/Attachments`

Spark events may include:

```json
{
  "type": "spark",
  "text": "看一下这张图",
  "source_client": "macos_app",
  "input_surface": "desktop_command_bar",
  "attachments": [
    {
      "id": "att-1",
      "name": "clipboard.png",
      "path": "/Users/me/Library/Application Support/BigHead/Attachments/clipboard.png",
      "mime_type": "image/png",
      "byte_count": 12345
    }
  ]
}
```

The runtime forwards attachment metadata to the agent as part of the desktop
view event. The app does not inline file contents into chat.

## Runtime Feedback

The conversation panel separates durable chat from local diagnostics:

- user and assistant/error messages render as chat bubbles
- runtime/client events appear in the activity strip at the top of the panel
- clicking `Log` opens a local diagnostics panel with recent client/runtime
  events and a clear action
- sending from the desktop command bar immediately shows a waiting spinner
- timeout feedback stays in the activity strip instead of becoming fake chat

This is still a local diagnostic layer, not a full runtime tracing system. A
release-ready app should later add persisted event history and deeper request
inspection.

## Protocol

Example command:

```json
{
  "op": "upsert_item",
  "item": {
    "id": "gameworld-design",
    "title": "bighead gameworld design",
    "state": "running",
    "control": "progress",
    "description": "Designing ambient status and interaction protocol",
    "progress": 0.45,
    "priority": 10
  }
}
```

`progress` is a number from `0.0` to `1.0`.

Native controls:

- `control: "item"` — clickable row
- `control: "progress"` — progress row with a native progress bar
- `control: "checkbox"` — checkbox row; include `checked: true/false`
- `group: "choice"` — desktop clients render these rows together as a choice
  group; clicking one sends `action: "choose"`
- `group: "checklist"` — compact step list; usually combine with
  `control: "checkbox"` and `checked`
- `group: "progress"` — compact progress list; usually combine with
  `control: "progress"` and `progress`

Clicking a status item sends this event back to Big Head over `/status/ws`:

```json
{
  "type": "status_event",
  "item_id": "gameworld-design",
  "action": "open",
  "title": "bighead gameworld design",
  "source_client": "macos_app"
}
```

Checkbox rows use `"action": "toggle"`.

The same click also selects the item locally and opens the status inspector in
the right panel. The inspector shows the item title, state, progress, protocol
fields, description, and related messages/documents when available. It does not
own Big Head state; its buttons send the same `status_event` shape back to the
runtime.

The server turns it into a normal spark-like view event:

```text
view event: panel=status:gameworld-design data={...}
```

The agent decides how to respond: inspect ECS, update status, open a richer
panel, or continue the conversation.

The runtime also appends chat transcript messages:

```json
{
  "op": "append_message",
  "message": {
    "id": "msg-1",
    "role": "assistant",
    "text": "## Result\n- Done",
    "created_at": 1786298400
  }
}
```

`role` is one of `user`, `assistant`, `system`, or `error`.

The macOS client renders a lightweight Markdown subset for chat and Notes
readability:

- headings: `#`, `##`, `###`
- unordered, ordered, and checkbox lists
- block quotes
- fenced code blocks
- simple table-like rows
- inline bold and inline code

For durable reading surfaces, publish a desktop document instead of only
appending chat messages:

```json
{
  "op": "upsert_document",
  "document": {
    "id": "doc-gameworld-reentry-001",
    "title": "bighead gameworld re-entry",
    "subtitle": "Ambient Dwelling next step",
    "kind": "insight",
    "text": "## Summary\n\n- Choice groups are interactive.\n- Notes can now render documents.",
    "references": [
      {
        "kind": "entity",
        "id": "E688",
        "title": "TopicInsight: click being in place",
        "description": "Current insight anchor"
      }
    ],
    "updated_at": 1786298400
  }
}
```

`Notes` mode shows `documents` as native document cards. If no documents exist,
it falls back to the conversation transcript.

Document `references` are optional anchors for ECS entities, sources, shards,
topics, or related documents. They are rendered as clickable reference rows in
Notes mode. Clicking one sends a `status_event` over `/status/ws`:

```json
{
  "type": "status_event",
  "item_id": "doc-gameworld-reentry-001:entity:E688",
  "action": "open_reference",
  "title": "TopicInsight: click + being in place",
  "document_id": "doc-gameworld-reentry-001",
  "document_title": "bighead gameworld re-entry",
  "reference_kind": "entity",
  "reference_id": "E688",
  "reference_title": "TopicInsight: click + being in place"
}
```

The macOS app only forwards the click. The agent decides whether to inspect ECS,
load a source/shard, append a reply, or update the desktop surfaces.
