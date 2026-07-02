# Pony IPC

This fork now treats `/tell` as the user-facing command for pony letters.
The live `/tmp` transport is an implementation detail that carries structured
letters; the receiving pony writes each letter into its mailbox before acting
on it.

## User-facing command

Supported forms:

- `/tell list`
- `/tell <pony-name> <message>`
- `/tell all <message>`

Examples:

- `/tell rd do a ls -la`
- `/tell twilight please verify the branch before I continue`
- `/tell all status check`
- `/tell aj databases should use RDS.`

The `/tell` command is a built-in slash command. On successful dispatch it
clears the composer like other inline slash commands.

## Letter envelope

Each sent letter carries four core fields:

- `DATE`
- `FROM`
- `SUBJECT`
- `BODY`

`SUBJECT` is derived from the body text up to the first `.`, `!`, `?`, newline,
or 25 characters, whichever comes first. `BODY` is the remainder.

The sender's cutie-mark symbol is included in the transport and mailbox render
so intrasystem messages are visually distinct from ordinary user text.

## Identity and scope

Pony letters are only active when the TUI is launched with pony identity env vars.

Identity comes from:

- `AGENIC_LAUNCH_PERSONALITY`, falling back to `PERSONALITY`
- `AGENIC_PROJECT_ROOT`
- `AGENIC_PROJECT_BRANCH`

If no pony identity is present, `/tell` remains unavailable at the App layer and
the TUI reports that pony letters are unavailable for the current session.

## Transport

The current implementation is intentionally local-only and file-backed.

- Live-session registry: `/tmp/codex-pony-registry.jsonl`
- Registry cleanup lock: `/tmp/codex-pony-registry.cleanup.lock`
- Letter log: `/tmp/codex-pony-chat.jsonl`
- Letter cleanup lock: `/tmp/codex-pony-chat.cleanup.lock`

Each pony session:

- writes a heartbeat entry on startup
- refreshes that heartbeat every 6 seconds
- polls the shared letter log every 6 seconds
- ignores its own outbound letters
- accepts direct pony targets or broadcast `*`

Entries older than 1 hour are treated as stale, and the next live session may
reset the corresponding log.

## Delivery behavior

Inbound pony letters are written to the receiver's project-local mailbox under
`pony/team.coordination/` and then delivered into the TUI as synthetic prompts
when the composer is free.

Current path:

1. sender issues `/tell ...`
2. Codex app appends a structured letter entry to the shared JSONL log
3. receiver polls and reads matching entries
4. receiver appends the letter to its mailbox markdown file
5. receiver injects the message into the chat widget as a synthetic submission
   when the composer is free

Rendered prompt text includes the sender's cutie-mark symbol so the message is
clearly an intrasystem letter rather than user input.

Delivery waits until the receiving composer is empty and no modal or popup is
active.
