# Ambient Dwelling Guide

Ambient primitives let the world provide sparse environmental context without turning it into surveillance or noisy polling.

## Components

- `SessionContext`: singleton ambient cue for the current session.
  Fields: `location`, `weather`, `time_of_day`, `light`, `updated_at`, `updated_at_epoch`, `source`.
- `AmbientSignal`: transient explicit event.
  Fields include `signal_id`, `category`, `wind`, `birds`, `light`, `temperature`, `intensity`, `source`, `timestamp`, `ttl_secs`, `created_at_epoch`, `expires_at_epoch`.
- `Glossary`: design vocabulary seeded by the runtime.
- `InsightStatus`: enum-like status component. Valid values are `open`, `crystallizing`, `done`, `deprecated`.

## Tools

Use `set_session_context` only when the user explicitly sets the ambient place or a preset asks for it.

Use `push_ambient_signal` for meaningful event-driven shifts:
- location changed
- time-of-day shifted meaningfully
- external source reports a privacy-sanitized ambient event

Do not push an ambient signal for every tool call, every ordinary task completion, or every tick.

## Privacy Boundary

For `category="human"`, only describe atmosphere:
- ok: quiet/noisy, slow/fast motion, calm/tense mood
- never: keystroke content, speech transcript, screen text, private personal details

## Naming

Use paradigm ids like:
- `bighead-gameworld-ambient-mvp1`
- `bighead-gameworld-ecs-coexistence`

Work items should include a partition-like `paradigm_id` when the component schema supports it:
- `bighead_main`
- `gameworld_vision`
- `external_favor`
- `vue_learning`
- `refactor`
