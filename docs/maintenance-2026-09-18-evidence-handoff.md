# Evidence handoff and observer retries

## Confirmed failures

- A mixed shard/TickAction audit lost the latter part of its operator report because the presence of shard evidence reduced the receipt allowance to 2,000 characters.
- Tool-free reporting still passed through context projection. Large evidence could become a shard reference that the reporter had no tool to read.
- Failed observation batches were retried at the same size, including after iteration exhaustion. Writes from failed attempts survive, so rediscovery can repeat work.

## Changes

- Pass the complete operator report with termination and independent shard evidence. A serialized handoff larger than 96 KiB bypasses secondary narration and displays the operator report, explicitly labeled as such. API/budget errors also expose the original report instead of hiding it behind an error message.
- Tool-free API requests preserve inline evidence and check the existing token budget. They fail before sending if the evidence cannot fit; they never create an inaccessible projection.
- Failed observer leases halve the batch size down to one, preserving FIFO pending events and the existing 300-second backoff. The reduced limit survives checkpoints, defaults to unrestricted for legacy checkpoints, and resets when the backlog drains. Success still requires validated dispositions for every leased event.
- Steward guidance uses stable source_id for durable identity, treats entity IDs as current tool addresses, and checks surviving prior writes before writing again. Narration guidance distinguishes budget exhaustion from loops and restore remapping from repeated spawning.

## Scope and limits

No live business records, checkpoint data, cooldowns, or iteration caps were changed. Legacy health records without a verifiable source are not automatically migrated. The stable identity and retry-write instructions guide the model; they do not enforce uniqueness on arbitrary dynamic schemas.

A single persistently failing event remains pending with backoff; this change does not silently acknowledge or discard it. LLM correctness and reduced live retry counts require observation after restarting with the rebuilt application. Raw-report fallback preserves the operator's claims, not independent verification of those claims.

Regression coverage includes mixed reports beyond the old cutoff, oversized handoffs, inline tool-free evidence and budget rejection, shrinking retries, restored retry state, legacy metadata, and acknowledgement of all remaining events.
