# Pony IPC

This fork includes a local pony-to-pony messaging path inside the TUI.

## User-facing command

Supported forms:

- `/pony list`
- `/pony <pony-name> <message>`
- `/pony all <message>`

Examples:

- `/pony rd do a ls -la`
- `/pony twilight please verify the branch before I continue`
- `/pony all status check`

The `/pony` command is a built-in slash command. On successful dispatch it clears
the composer like other inline slash commands.

## Identity and scope

Pony IPC is only active when the TUI is launched with pony identity env vars.

Identity comes from:

- `AGENIC_LAUNCH_PERSONALITY`, falling back to `PERSONALITY`
- `AGENIC_PROJECT_ROOT`
- `AGENIC_PROJECT_BRANCH`

If no pony identity is present, `/pony` remains unavailable at the App layer and
the TUI reports that pony IPC is unavailable for the current session.

## Transport

The current implementation is intentionally local-only and file-backed.

- Live-session registry: `/tmp/codex-pony-registry.jsonl`
- Registry cleanup lock: `/tmp/codex-pony-registry.cleanup.lock`
- Chat log: `/tmp/codex-pony-chat.jsonl`
- Chat cleanup lock: `/tmp/codex-pony-chat.cleanup.lock`

Each pony session:

- writes a heartbeat entry on startup
- refreshes that heartbeat every 6 seconds
- polls the shared chat log every 6 seconds
- ignores its own outbound messages
- accepts direct pony targets or broadcast `*`

Entries older than 1 hour are treated as stale, and the next live session may
reset the corresponding log.

## Delivery behavior

Inbound pony messages are turned into synthetic user prompts inside the TUI.

Current path:

1. sender issues `/pony ...`
2. Codex app appends a chat entry to the shared JSONL log
3. receiver polls and reads matching entries
4. receiver mirrors the message into the project-local `pony/runtime` queue
5. receiver injects the message into the chat widget as a synthetic submission when the composer is free

Rendered prompt text currently looks like:

```text
[Applejack] says do a ls -la
```

Delivery waits until the receiving composer is empty and no modal or popup is
active.

## Current limitation

This IPC lane now mirrors inbound messages into the project-local queue runtime
before direct TUI delivery, so the parked shell host can retain durable state
when immediate injection is blocked.

Current limitation:

- the direct TUI lane and the queue-backed lane are still coupled by best-effort
  shelling to `pony/scripts/queue-runtime.sh`
- immediate live delivery removes the queued item again, so the queue acts as a
  persistence bridge rather than the sole execution engine
- this remains fork-specific behavior in the Codex checkout, not upstream Codex
  behavior
