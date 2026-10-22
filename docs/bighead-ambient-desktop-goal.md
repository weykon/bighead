# BigHead Ambient Desktop Goal

Last updated: 2026-08-11

## Intent

BigHead Desktop should not become a conventional chat box with a task sidebar.
The product goal is an **ambient operating surface**: a native desktop place
where the user can sense what BigHead is holding, what is moving, what needs a
choice, and where conversation can happen without becoming the only interface.

The app remains a renderer and input client. It does not own reasoning, ECS
writes, task decisions, or agent policy. The runtime and agent publish view
state through the view server; the app renders that state and sends user actions
back as normal Spark/view events.

## Design North Star

The long-term shape is **Ambient Dwelling**:

- **Dwelling**: the first screen feels like a shared state space, not a log.
- **Sparse but meaningful**: surface only work, signals, choices, and summaries
  that help the user re-enter the world.
- **Agent-authored state**: BigHead decides what should be visible, not the app.
- **Native affordance**: the app provides high-quality macOS components and
  interaction primitives, but does not hard-code domain workflows.
- **Modeful but coherent**: different modes expose different focus, while using
  the same underlying runtime state.

## Reference Blend

The goal is not to clone one product. The product language combines:

- **Raycast**: fast command input, keyboard-first action launch, low ceremony.
- **Linear**: clear work queue, progress, blockers, decisions, strong hierarchy.
- **Things**: personal, calm, understandable to non-technical users.
- **Craft**: readable long-form output, Markdown/living notes, knowledge carry.
- **Ambient Dwelling**: the global feeling that BigHead is present and holding a
  world state, not just replying to prompts.

## Current Implemented Baseline

Native macOS AppKit app:

- Source: `external/big-head-menubar`
- Protocol source of truth: `crates/big-head-view`
- Packaged app: `dist/macos/BigHead.app`
- Runtime transport:
  - `GET /api/status`
  - `WS /status/ws`
  - desktop input sends `{ "type": "spark", "text": "..." }`

Current UI shell:

- Ambient top surface: title, summary, running/pending/done counts, latest signal.
- Four view modes: `Dwelling`, `Work`, `Chat`, `Notes`.
- Status board: renders `StatusItem`.
- Status inspector: clicking a status item opens a local detail view with
  protocol fields, progress, description, and related messages/documents.
- Conversation: renders `ChatMessage`.
- Activity strip: local send/wait/runtime events are visible without polluting
  the assistant transcript.
- Attachment tray: desktop command bar can attach files by picker/drag and paste
  clipboard images as local PNG files.
- Command bar: sends Spark input and shows waiting state.
- Smoke test entry: `scripts/smoke-macos-desktop.sh` builds the Swift app and
  verifies desktop status/attachment protocol fixtures.

Current protocol primitives:

- `StatusState`
  - `title`
  - `subtitle`
  - `summary`
  - `items: [StatusItem]`
  - `messages: [ChatMessage]`
- `StatusItem`
  - `id`
  - `title`
  - `state`
  - `control`
  - `description`
  - `progress`
  - `checked`
  - `group`
  - `priority`
  - `updated_at`
- `ChatMessage`
  - `id`
  - `role`
  - `text`
  - `created_at`

This baseline is useful, but it is still too narrow. It can show work and chat,
but cannot yet express a richer ambient scene, choices, documents, or source
relationships as first-class UI.

## Product Modes

### 1. Dwelling

Default mode. This is the home surface.

Purpose:

- Help the user re-enter the current BigHead world quickly.
- Show the current atmosphere, active thread, latest meaningful signal, and
  decisions waiting for the user.
- Keep details available but not visually dominant.

Expected panels:

- Ambient surface
- Active signals
- Running/pending/done state
- Current choice prompts
- Compact conversation tail

### 2. Work

Linear-like operational mode.

Purpose:

- Make ongoing work inspectable.
- Give the user confidence that BigHead is doing useful, structured work.
- Surface blockers, progress, next actions, and ownership.

Expected panels:

- Work queue grouped by `group`/status
- Progress and blocker rows
- Selected work details
- Activity timeline
- Action buttons such as open, continue, pause, mark done, clarify

### 3. Chat

Companion mode.

Purpose:

- Let ordinary users talk to BigHead without learning the system.
- Make assistant responses readable.
- Preserve waiting, errors, and runtime state clearly.

Expected panels:

- Full conversation
- Markdown rendering
- Local user bubbles
- Waiting state
- Error recovery text

### 4. Notes

Craft-like living document mode.

Purpose:

- Turn agent output into durable reading surfaces.
- Support summaries, crystallized insights, and long-form reasoning.
- Help users revisit what BigHead has learned.

Expected panels:

- Index/anchors on the left
- Main living note/document area
- Markdown headings, lists, tables, code blocks
- Source/entity references

## Protocol Gap

The current `render_status` protocol is intentionally lightweight. It is good
for a menu bar and early desktop app, but not enough for the Ambient Desktop
goal.

Missing primitives:

- **Ambient signal**: a visible user-facing signal with intensity, category,
  lifetime, and source.
- **Choice prompt**: a set of choices the user can click, with stable IDs.
- **Surface section**: a named UI section that can be independently updated.
- **Document block**: Markdown or structured blocks for Notes mode.
- **Entity/source reference**: links from UI content back to ECS entities,
  shards, sources, or topics.
- **Selected item state**: which item is currently focused in a panel.
- **Action result**: feedback after the user clicks a status/action item.

## Implementation Notes

### 2026-08-11: Attachment Tray

The desktop command bar now accepts local attachments without changing BigHead's
reasoning ownership. Users can select files, drag files onto the command bar, or
paste an image from the clipboard. Clipboard images are saved under
`~/Library/Application Support/BigHead/Attachments`, and Spark events carry an
`attachments` array with `id/name/path/mime_type/byte_count`.

The view server preserves attachment metadata on `desktop:input` events,
including attachment-only sparks. The app does not inline file bytes or image
pixels into chat; the agent must inspect the provided paths with appropriate
tools.

### 2026-08-11: Desktop Smoke Test Base

A first smoke-test entry now exists at `scripts/smoke-macos-desktop.sh`. It
builds the Swift desktop app, decodes a desktop status fixture covering
`choice`/`checklist`/`progress` groups, verifies attachment-bearing desktop
Spark events, and runs the full `big-head-view` suite.

This does not yet prove visual quality. The next test layer should launch the
app with a controlled status snapshot, capture screenshots, and compare them
against blessed artifacts across at least one desktop size.

### 2026-08-11: Visual System Pass 1

The desktop app now has a more consistent native visual base:

- neutral app background with quieter raised panels
- shared panel/card/inset chrome tokens
- tighter window spacing and wider default layout
- consistent toolbar button styling
- calmer status groups, chat bubbles, diagnostics, composer, inspector, and
  document reference rows

This is a foundational visual pass, not the final product art direction. The
next visual layer should be screenshot-driven and should refine selected states,
mode-specific layouts, empty states, and first-run screens with actual runtime
fixtures.

### 2026-08-11: Status Inspector

The desktop app now treats a `StatusItem` click as two things:

- local selection, so the user immediately sees which object was clicked
- outbound `status_event`, so the agent still decides the semantic response

This keeps the app as a renderer/input client while adding a concrete
interaction layer. The inspector is deliberately protocol-driven: it renders
only fields already present in `StatusItem` and opportunistically links related
chat messages or desktop documents by matching item id/title.

### 2026-08-11: Conversation Activity Strip

The conversation panel now separates durable transcript messages from local
runtime diagnostics. User and assistant/error content stays in chat bubbles;
connection logs, send confirmation, and waiting/timeout feedback appear in a
small activity strip above the transcript. This gives the user immediate
feedback after sending a Spark while keeping the conversation readable.

The strip also opens a local diagnostics panel with recent client/runtime
events and a clear action. This is intentionally app-local evidence for the
user; it does not change BigHead reasoning, ECS writes, or the published
desktop protocol.

## Proposed Next Protocol Layer

Keep `render_status` for lightweight compatibility, but add a richer desktop
surface command family. The first version can live in the same status snapshot
instead of requiring a fully new server.

Candidate shape:

```json
{
  "op": "set_surface",
  "surface": {
    "title": "暖茶 continuation — 客厅夜灯下",
    "subtitle": "re-entry moment, crystallize 仍然稳阵",
    "mode": "dwelling",
    "ambient": {
      "headline": "客厅夜灯下",
      "signal": "tea warmth, waiting for A/B/C choice",
      "intensity": 0.35,
      "source": "agent"
    },
    "sections": [
      {
        "id": "choices",
        "kind": "choices",
        "title": "三条路你拣",
        "items": [
          {"id": "a", "title": "Spawn BizModelingDoc", "action": "choose"},
          {"id": "b", "title": "回应 Q1-Q5", "action": "choose"},
          {"id": "c", "title": "继续 archeology 落地", "action": "choose"}
        ]
      }
    ]
  }
}
```

This should be treated as a product direction, not as final schema. The schema
must stay strict enough for native rendering and simple enough for the agent to
use correctly.

## Development Phases

### Phase 1: Make the current shell feel intentional

Goal:

- The current app should look and behave like an intentional native product.

Concrete work:

- Tighten visual hierarchy for the four modes.
- Improve status row layout and empty states.
- Improve chat Markdown rendering and selectable text.
- Add a clear runtime/connection area.
- Add keyboard shortcuts for mode switching and command focus.

Done when:

- A non-technical user can open the app, send a message, see waiting feedback,
  and understand where response/status appears.
- Existing `StatusItem + ChatMessage` data looks polished in all modes.

### Phase 2: Agent-authored surface protocol

Goal:

- BigHead can publish richer desktop UI without the app knowing domain logic.

Concrete work:

- Extend the Rust status protocol or add a sibling desktop protocol.
- Add strict JSON schema and tool guide for the agent.
- Add native renderers for:
  - ambient signal
  - choice group
  - work item detail
  - document block
  - entity/source reference
- Route user actions back through `/status/ws` as view events.

Done when:

- The agent can create an A/B/C choice panel, the user can click an option, and
  the runtime receives a normal view event with enough context to continue.

### Phase 3: Re-entry and continuity

Goal:

- The app helps the user return to ongoing worlds and topics.

Concrete work:

- Persist desktop surface snapshot.
- Show last active topics and choice prompts on launch.
- Distinguish runtime logs from agent-authored content.
- Add selected item/details state.
- Add reconnect/restart guidance when an old external runtime lacks transcript
  support.

Done when:

- Restarting the app preserves useful state and does not feel blank or broken.

### Phase 4: Personal distribution

Goal:

- Friends/family can run the app without developer knowledge.

Concrete work:

- First-run onboarding.
- Provider settings.
- App icon and bundle metadata.
- Signing/notarization/DMG.
- Clear runtime error states.

Done when:

- A new user can install, enter an API key, start runtime, send a Spark, and see
  a visible response without opening a terminal.

### Phase 5: Ambient co-presence

Goal:

- BigHead can receive sparse ambient cues/signals and reflect them in the
  desktop surface without becoming noisy or invasive.

Concrete work:

- Connect `SessionContext` and `AmbientSignal` to desktop surface.
- Add user-controlled privacy settings.
- Add user-visible source labels for ambient signals.
- Keep signals sparse and meaningful.

Done when:

- A meaningful ambient event can update both the agent context and desktop
  surface, while the user can see what changed and why.

## Non-Goals

- The desktop app should not embed agent reasoning.
- The desktop app should not write ECS directly.
- The desktop app should not guess tasks from raw logs.
- The desktop app should not become a web app inside a native shell for this
  phase.
- Ambient sensing must not collect private content such as keyboard text,
  screen content, or speech transcripts without explicit future design.

## Immediate Next Engineering Slice

The next code slice should be **Phase 1 completion plus the smallest Phase 2
protocol foothold**:

1. Add keyboard shortcuts:
   - `Command+1..4` switch mode
   - `Command+L` focus command bar
   - `Command+R` refresh
2. Add a first-class `choices` surface using the existing `StatusItem` model as
   a bridge:
   - items with `group = "choice"` render as a compact choice group
   - clicks still emit `status_event`
3. Improve Markdown rendering for Notes mode:
   - headings
   - bullet spacing
   - code block background
4. Update `render_status` guide to teach the agent:
   - when to publish status
   - how to publish choices
   - how to append assistant messages for desktop users

This slice keeps the app useful now while opening the path to a richer
agent-authored surface protocol later.

### Implementation Note 2026-08-11

Completed from this slice:

- `Command+1..4` switches `Dwelling / Work / Chat / Notes`.
- `Command+L` focuses the command bar.
- `Command+R` refreshes the desktop client.
- `StatusItem.group == "choice"` renders as a native desktop choice group.
- Clicking a choice sends the existing `status_event` path with action
  `choose`.
- `render_status` tool description and guides now teach the agent how to publish
  choice groups.

Remaining:

- Move Notes mode from chat-bubble Markdown toward first-class document blocks.
- Decide whether `group == "choice"` stays a compatibility bridge or becomes a
  richer first-class desktop surface schema.

### Implementation Note 2026-08-11B

Completed from this slice:

- Chat/Notes Markdown rendering now handles headings, unordered lists, ordered
  lists, checkbox lists, block quotes, dividers, fenced code blocks, simple
  table-like rows, inline bold, and inline code.
- README documents the supported Markdown subset so agent/user expectations are
  explicit.

Remaining:

- Native layout for rich document blocks is still not first-class.
- Tables are readable monospaced rows, not real native table components.
- Code blocks have text background styling, not full padded code panels.

### Implementation Note 2026-08-11C

Completed from this slice:

- `StatusState` now supports `documents`.
- `render_status` now supports `upsert_document` and `remove_document`.
- macOS Notes mode renders documents as native document cards when documents are
  present.
- Notes mode falls back to the conversation transcript when no documents exist.
- Agent guides now explain when to use `append_message` versus
  `upsert_document`.

Remaining:

- Document schema is still intentionally simple: title, subtitle, kind, text,
  references, updated_at.
- Richer document blocks and clickable reference deep-links still need a future
  first-class schema.

### Implementation Note 2026-08-11D

Completed from this slice:

- `StatusDocument` now supports `references`.
- Document references can point at entities, sources, shards, topics, documents,
  or other stable IDs.
- macOS Notes mode renders references below the document body as visible anchor
  rows.
- Agent guides now show how to attach references to `upsert_document`.

Remaining:

- References are visible anchors only; clicking a reference does not yet route a
  focused view event.
- References are still a flat array, not a typed graph or bidirectional ECS
  relationship model.

### Implementation Note 2026-08-11E

Completed from this slice:

- macOS Notes references are now clickable rows with an explicit `OPEN`
  affordance.
- Clicking a reference sends the existing `status_event` path with
  `action=open_reference`.
- The desktop payload includes `document_id`, `document_title`,
  `reference_kind`, `reference_id`, and `reference_title`.
- `big-head-view` now preserves extra `status_event` fields instead of reducing
  the event to item/action/title only.
- Agent guides now describe how to respond to reference clicks: inspect ECS,
  source, shard, or update desktop surfaces as needed.

Remaining:

- References are still a flat array, not a typed graph or bidirectional ECS
  relationship model.
- The desktop app still delegates all semantic interpretation to the agent loop;
  it does not fetch ECS entities directly.

### Implementation Note 2026-08-11F

Completed from this slice:

- macOS app visual structure was tightened toward the Ambient Dwelling model:
  top ambient status, left actionable status/index, right conversation or living
  notes, bottom command bar.
- Scroll view document areas now fill their viewport, preventing sparse content
  from drifting to the bottom of panels.
- Chat transcript no longer force-scrolls to the bottom on every remote state
  refresh unless the user is already near the bottom.
- macOS app spark payloads now include `source_client=macos_app` and
  `input_surface=desktop_command_bar`.
- `big-head-view` preserves extra desktop spark metadata when routing view
  events into the agent loop.

Remaining:

- The app still uses AppKit views only; no dedicated design system package or
  visual regression harness exists yet.
- Desktop composer attachments are still not modeled.

### Implementation Note 2026-08-11G

Completed from this slice:

- The macOS command bar is now a dedicated `CommandComposerView` module instead
  of inline `NSTextField` wiring inside `DesktopWindowController`.
- It uses a native `NSTextView` composer, so multiline paste no longer
  immediately sends partial lines.
- `Return` submits the command; `Shift+Return` inserts a newline.
- The composer auto-grows from compact single-line height up to a bounded
  multiline height, then scrolls internally.
- Waiting state is controlled through the composer API, keeping controller code
  focused on runtime flow.

Remaining:

- The app still uses AppKit views only; no dedicated design system package or
  visual regression harness exists yet.
- There is no attachment tray in the desktop composer yet.

### Implementation Note 2026-08-11H

Completed from this slice:

- Desktop status now understands three conventional `StatusItem.group` values:
  `choice`, `checklist`, and `progress`.
- `choice` keeps the existing mutually exclusive choice group behavior.
- `checklist` renders compact toggleable rows and summarizes completed/total
  count.
- `progress` renders compact progress rows and summarizes average completion.
- No new core component was added; this uses the existing `StatusItem` shape:
  `state`, `control`, `checked`, `progress`, `group`, and `priority`.
- `render_status` schema descriptions, agent guides, and the macOS README now
  describe these group semantics.

Remaining:

- Checklist/progress clicks are still routed as generic `status_event`; semantic
  state updates remain agent-owned.
- Group labels are currently conventional English headings, not agent-provided
  custom section titles.
