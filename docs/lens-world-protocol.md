# Lens World Protocol

Big Head treats one checkpoint as one sovereign ECS world. Multiple agent worlds
do not share Bevy entity ids and do not mutate each other directly. They exchange
Lens objects through buckets.

## Runtime Worlds

Start a runtime with an explicit world id:

```bash
cargo run -- --world local:default --view-server
cargo run -- --world peer:alice --headless
```

Each world gets isolated runtime files:

```text
big_head_worlds/
  local-default/
    world.json
    checkpoint.json
    view_snapshot.json
    lens/
      inbox/pending.jsonl
      outbox/pending.jsonl
```

The legacy `big_head_checkpoint.json` is copied into `local:default` on first
run when the new default checkpoint does not exist.

## Lens

A Lens is an agent-readable exchange object. It carries perspective, intent,
references, payload, and permissions.

```json
{
  "lens_id": "lens_...",
  "from_world": "world:weykon",
  "to_world": "world:alice",
  "kind": "proposal",
  "intent": "share_context",
  "summary": "Share an ambient dwelling insight",
  "refs": [
    {
      "world_id": "world:weykon",
      "object_id": "topic-ambient-dwelling-001",
      "object_type": "Topic",
      "version": "sha256:..."
    }
  ],
  "payload": {},
  "permissions": {
    "can_store": true,
    "can_derive": true,
    "can_forward": false
  }
}
```

## Bucket

The first transport is local JSONL files:

```text
pending.jsonl
accepted.jsonl
rejected.jsonl
derived.jsonl
```

Later transports can be HTTP inboxes, object storage buckets, or peer-to-peer
channels without changing the Lens shape.

## Agent Tools

The first runtime integration exposes Lens as agent tools when a runtime has a
`WorldManifest` attached:

- `create_lens` writes a Lens to this world's `lens/outbox/pending.jsonl`.
- `list_lens_inbox` lists pending inbound Lens objects.
- `read_lens` reads one pending inbound Lens by `lens_id`.
- `accept_lens` appends a pending inbound Lens to `accepted.jsonl`.
- `reject_lens` appends a pending inbound Lens to `rejected.jsonl`.

These tools only operate on the exchange bucket. Accepting a Lens does not
write ECS facts automatically. The local agent should explicitly inspect,
derive, and write ECS components using the normal ECS tools when appropriate.

## Boundary

Cross-world references must use stable `WorldObjectRef`, not Bevy `Entity`.
The receiving agent decides whether a Lens becomes local ECS facts, a reply
Lens, a rejection, or a request for more context.
