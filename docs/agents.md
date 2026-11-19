# Agents and sessions

qcc has no agent loop of its own. Real agent CLIs do the work, each through a provider plugin
(a bridge):

| Provider      | Agent                             | Tested with         |
| ------------- | --------------------------------- | ------------------- |
| `claude-code` | `claude -p`, stream-json          | Claude Code 2.1.325 |
| `opencode`    | `opencode acp` (shared ACP bridge) | opencode 1.19.4    |
| `codex`       | `codex app-server`                | codex-cli 0.166.0   |
| `echo`        | test agent, example plugin        | —                   |

`echo` is not a real agent. It repeats the text it receives, and on commands (`/slow`, `/ask`,
`/approve`, `/fail`, …) it imitates what real agents do. It is the daemon's test provider and an
example of a plugin with UI (the "Echo" and "Echo canvas" tabs on its threads). Its manifest marks
it `example = true`, and the app hides such plugins until **Show example plugins** is turned on in
the settings. In the CLI it is always available.

## Models

qcc asks the agent itself for the list of models instead of keeping its own: Claude Code answers
`initialize` in stream-json (the same list as in `/model`), Codex answers `model/list` in
`codex app-server`, opencode answers `opencode models`. The request costs nothing: no message is
sent to the agent, and the process exits right after the answer (or after 20 s). The daemon
remembers the answer until `--refresh`.

```sh
qcc provider models claude-code [--refresh]   # ids for `thread spawn --model` and effort levels
```

In the app the model field is a searchable list. A model that is not in the list can be typed in
as is.

## Effort

Effort is how deeply the model thinks. Each model has its own levels, and the agent names them
itself: Claude Code reports `supportedEffortLevels` in the same `initialize` answer (passed with
the `--effort` flag); Codex reports `supportedReasoningEfforts` and `defaultReasoningEffort` in
`model/list` (passed as `model_reasoning_effort`). opencode has no choice. Without an effort the
agent works at its default level.

```sh
qcc thread spawn --provider codex --effort high --prompt "…"
qcc thread agent <thread-id> codex --effort low   # a different level between turns
qcc settings provider codex --effort medium       # the level for new threads of this agent
```

A child thread of the same agent inherits its parent's effort. In the app, effort is chosen next
to the model: in the new thread dialog, in the agent button in the thread header, and in the
agent's settings. The list of levels depends on the selected model. A level the new model does not
know is reset to "default".

## Switching agent and model

Between turns a thread can be moved to another model, another effort or another agent:

```sh
qcc thread agent <thread-id> claude-code --model sonnet   # another model of the same agent
qcc thread agent <thread-id> codex                        # another agent, with its default model
```

The current session is released. With the next message the same agent resumes it with the new
model and effort.
A different agent cannot continue someone else's session, so it starts its own in the same
directory. As its first message it receives the thread's conversation (the latest messages, if the
conversation is long) and the user's new message. A [permission mode](#permission-mode) the new
agent does not support is reset to "default". In the app the button with the agent and model in
the thread header does the same.

## Settings

The daemon keeps the settings (`<data>/settings.json`, readable only by its owner), so they apply
both in the app (the "Settings" screen) and in the CLI:

```sh
qcc settings show                                   # where things are and what is set
qcc settings program claude ~/opt/claude/bin/claude # which file to run instead of searching PATH
qcc settings program claude                         # search PATH again
qcc settings default-provider codex                 # default agent for new threads
qcc settings provider claude-code --model sonnet --effort high --permission-mode accept-edits
qcc settings provider codex --env OPENAI_API_KEY=… --unset-env OLD_VAR
qcc settings provider opencode --hidden true        # do not offer it in the app
```

- **Programs.** A plugin can run only the programs named in its `process` permission (`claude`,
  `codex`, `opencode`, `git`, …). The setting only says which file stands behind a name. Without
  it the name is looked up in the daemon's `PATH`. The settings screen shows which file will run,
  or why there is none.
- **Default agent, model, effort and permission mode** are filled into a new thread when they are
  not given (`thread spawn`, the new thread dialog).
- **Environment variables** are given to every session of this agent (an API key, a configuration
  directory). `LD_*`, `DYLD_*` and `QCC_*` cannot be set.

The same screen has the switch for showing example plugins, the interface scale, and the list of
plugins with their contributions and permissions, where each can be turned off or on
([Turning plugins on and off](plugins.md#turning-plugins-on-and-off)).

### Plugin settings

A plugin can declare its own settings: typed fields — text, a flag, a choice from a list, an
agent, a model of the agent from another field, an effort of that model, a permission mode. The
daemon stores the values (`plugins` in `settings.json`) and checks them against the type. The
plugin receives them, with defaults filled in, when the daemon starts and on every change. In the
app the fields are drawn in the plugin's card on the "Settings" screen (a model field uses the
same list as when creating a thread). In the CLI:

```sh
qcc plugin settings provider-echo          # fields, values and the defaults
qcc plugin set provider-echo greeting hi   # set a value
qcc plugin set provider-echo greeting      # restore the default
```

## MCP servers

Servers configured in qcc are given to every new session of any agent, **in addition to** the
ones named in the agent's own configuration (`~/.claude.json`, `~/.codex/config.toml`, the opencode
config). qcc manages only its own servers: it turns them on and off for all agents and for a
single thread (see [below](#turning-a-server-off-in-one-thread)). Servers from the agent's config
are shown with their status, but qcc does not toggle them; that is done in the agent itself.

```sh
qcc mcp add docs --env TOKEN=… -- npx -y docs-mcp       # a program the agent talks to over stdio
qcc mcp add search --url https://…/mcp --header Authorization="Bearer …"   # an HTTP server
qcc mcp list                                            # what is configured and what is off
qcc mcp disable docs                                    # stays in the settings, but sessions do not get it
qcc mcp enable docs
qcc mcp remove search
```

A server name consists of Latin letters, digits, `_` and `-`. Bridges pass the servers the way the
agent accepts them: Claude Code through `--mcp-config`, Codex through an override of `mcp_servers`
in the thread's config, opencode through `mcpServers` of the ACP protocol. If an ACP agent does not
support HTTP servers, it gets only the stdio servers, and a warning appears in the timeline.

### Changes reach running sessions

A change of the settings — a server added, replaced, turned on or off, removed — reaches every
thread that is running, not only the sessions started after it. Each thread gets the servers that
are on in the settings minus those it keeps off (see [below](#turning-a-server-off-in-one-thread)).
How the change gets into a running session depends on the agent:

- **Claude Code** takes it at once, even in the middle of a turn: the bridge replaces qcc's servers
  of the session with the `mcp_set_servers` request. A new server connects, a removed one is shut
  down and leaves the status list, one set up anew (`qcc mcp add` with the same name) connects
  again with its new configuration. The servers of Claude Code's own configuration stay as they
  are.
- **Codex and opencode** cannot change the servers of a running session. qcc releases their session
  as soon as the current turn ends, and the next message continues it (`resume`) with the new
  servers. A thread without a session gets them when its next session starts.

A provider tells qcc that it can do the former with the `mcp_live` capability (see
`qcc plugin list`).

### Status

The agent reports how each MCP server of its session is doing, including the servers from its own
config: connecting, ready, failed (with the reason) or waiting for sign-in. The status is stored in
the thread and shown in `qcc thread show` (the `mcp` lines), in the app (see
[MCP servers](desktop.md#mcp-servers) in the desktop docs) and in `thread.mcp_servers` of the
protocol. Claude Code is asked right after the session starts, before the first turn (once a second
while servers are connecting, and again after every turn). Codex reports server startup by itself.
opencode does not report status. A new session starts the list over.

### Turning a server off in one thread

A new thread gets every qcc server that is on in the settings. It can be turned off in a single
thread: there it stays off, while other threads and the children of this thread keep it.

```sh
qcc thread mcp <id>                  # the session's servers (the agent's own too, with status) and what is off
qcc thread mcp <id> --off docs
qcc thread mcp <id> --on docs
```

A server kept off is not given to the agent at all. The switch reaches a running session the same
way a change of the settings does (see [above](#changes-reach-running-sessions)): Claude Code drops
or connects the server at once, Codex and opencode get the change in the session that continues
after the current turn.

## Permission mode

A thread has a mode that qcc passes to the agent: `--permission-mode` on `thread spawn`, later
`qcc thread mode <id> <mode>`.

| Mode           | Meaning                                                              |
| -------------- | -------------------------------------------------------------------- |
| *(not set)*    | the agent's own configuration decides; qcc changes nothing           |
| `read-only`    | the agent only reads and looks; what would change something is refused, and nobody is asked |
| `ask`          | everything the agent wants approved is asked of the user             |
| `accept-edits` | file edits in the working directory go through; everything else is a question |
| `auto`         | the provider's own automatic reviewer decides; the user is not asked |
| `full`         | nothing is asked, no sandbox                                         |

How the mode reaches the agent: Claude Code gets the `--permission-mode` flag; Codex gets the
sandbox, the approval policy and the reviewer (`user` or `auto_review`) in `thread/start`; for ACP
agents the bridge itself answers permission requests (so `auto` is not available for opencode:
ACP has no reviewer of its own). A provider declares the modes it supports, see
`qcc plugin list`.

`read-only` is the strictest mode, so any thread may start a child in it. A provider declares it
only when it can enforce it, not just tell the agent:

- **Claude Code**: `--permission-mode dontAsk` with only the tools that read (`Read`, `Grep`,
  `Glob`) and `Bash` limited to commands that look (`git diff`, `git log`, `git show`,
  `git status`, `git blame`, `git ls-files`, `qcc thread diff`, `qcc thread changes`,
  `qcc thread show`, `qcc comment list`, `ls`), and `--restricted`, so that allow rules in the
  user's settings files cannot loosen it. Anything else is refused without a prompt; should a
  prompt come anyway, the bridge refuses it.
- **Codex**: the `read-only` sandbox with the approval policy `never`. A command that writes fails
  in the sandbox. The sandbox also keeps `qcc` from reaching the daemon's socket, so a Codex
  session in this mode reads changes with git.
- **opencode**: permission rules in `OPENCODE_PERMISSION` that deny everything but reading,
  searching and the same looking commands; opencode refuses the rest itself, and the bridge
  refuses any request that reaches it.

Known limits: a command that looks may still take an option that writes (`git diff
--output=<file>`); Claude Code's allow rules match by prefix and cannot tell. Requests addressed to the user arrive in the timeline and are closed with
`qcc interaction resolve`.

Claude Code's questions (`AskUserQuestion`) are not a permission request. The bridge asks them in
the timeline one at a time; the options become buttons, and a custom value can be typed as text.
Where several options can be chosen, they are written separated by commas. The answers to all
questions go to the agent; a question left unanswered cancels the whole call.

## Messages during a turn

By default `qcc thread send` to a busy thread puts the message **in the queue**: it goes out as a
separate turn when the current one ends. With `--steer` the message is **mixed into the running
turn** (Claude Code, Codex). If the provider cannot do that (opencode), the message is queued. The
queue is stored in the journal and survives a daemon restart. `qcc thread stop` clears it, and
`qcc thread unqueue <thread-id> <queue-id>` takes one message back (the id is printed by
`thread send --json`).

In the app, while the agent is working, the composer has a **Queue / Steer** switch. It decides
what Enter does (Alt+Enter does the other thing), and it is remembered. Queued messages are shown
above the composer; each one can be mixed into the running turn ("Steer now"), returned to the
composer for editing, or deleted. Agents without steer have no switch, and the status line says
"Queue only".

## New conversation and compaction

```sh
qcc thread reset <thread-id>              # start the conversation over in the same thread
qcc thread compact <thread-id> [--follow] # ask the agent to compact its context
```

**New conversation** (`thread reset`): the next message starts a new agent session instead of
continuing the old one. Nothing is deleted. Earlier conversations stay in the journal and in the
timeline (in the app they are collapsed above the "A new conversation" divider), while the thread
transcript — for `handoff`, `review`, `thread wait` and switching agents — starts with the latest
conversation. A reset is rejected while a turn is running.

**Context compaction** (`thread compact`): the agent summarizes the conversation itself and
continues from the summary. This is a provider capability (`compact` in `qcc plugin list`): Claude
Code receives `/compact`, Codex receives `thread/compact/start`; opencode does not have it.
Compaction runs as a separate turn without a user message and ends with a note in the timeline
saying how many tokens there were before and after (if the agent says). Compaction the agent does
by itself when the context is full is marked the same way. A thread without a live session first
resumes it. In the app these are the "Compact the context" and "Start a new conversation" items in
the thread menu.

## What the context is made of

```sh
qcc thread context <thread-id> [--json]   # how full the window is, and its parts
```

Every provider knows how full the context window is (`context_used`/`context_window` in the thread
projection). The breakdown into parts — system prompt, system tools, MCP tools, memory files,
skills, agents, messages, autocompact buffer, free space — comes from a provider with the
`context_usage` capability (visible in `qcc plugin list`). Currently that is Claude Code: after
every turn and when the session starts, the bridge sends it the `get_context_usage` control request
(the same thing `/context` shows; the model is not called). The breakdown arrives in the timeline
as a `context/usage` event, together with the contents of the large parts (which memory files, MCP
tools and skills take how much). The thread projection keeps only the parts (`context`). Parts
marked "deferred" are known to the agent but not loaded into the window. Codex and opencode do not
give a breakdown. For them the app shows an estimate from the usage reports (the smallest request
in a conversation is its beginning: the system prompt, tools and the first message; the rest is
the conversation), clearly marked "estimate".

## Subscription usage limits

```sh
qcc provider limits [<provider>] [--refresh] [--json]
```

```
claude-code (max)  as of 3m ago
  5-hour                 42%  resets in 2h 10m
  Weekly                 44%  resets in 6h 40m
  Weekly · Fable         26%  resets in 6h 40m
codex (prolite)  as of 3m ago
  Weekly                  4%  resets in 1d 12h
```

Claude and ChatGPT subscriptions limit how much an agent may do in a 5-hour window and in a week
(and, on some plans, per model). These limits belong to the account an agent signs in with, not to
a thread: every thread of the provider spends the same allowance. So qcc keeps them per provider —
the latest report and when it came — in its database, where they survive a restart of the daemon.
Each window has a stable id, a label (`5-hour`, `Weekly`), a scope when it covers one model only,
the share spent in percent, when it starts over (`resets_at`, milliseconds since the epoch), its
length and, when the agent says, a status (`allowed`, `warning`, `exceeded`).

Where the numbers come from (the `rate_limits` capability, visible in `qcc plugin list`):

- **Claude Code.** When a session starts, the bridge sends the `get_usage` control request (what
  `/usage` shows, read from the claude.ai usage endpoint; the model is not called): the plan, the
  5-hour and weekly windows and the per-model weekly ones. After that every `rate_limit_event`
  Claude Code sends when a response moved a limit updates the 5-hour and weekly windows. An account
  signed in with an API key or through a cloud provider has no such limits, and none are shown.
- **Codex.** After the handshake the bridge asks `account/rateLimits/read`, and Codex sends
  `account/rateLimits/updated` after every turn. Its windows are named by their length: on some
  plans there is only a weekly one. Buckets of single models (`rateLimitsByLimitId`) are listed
  with their name as the scope.
- **opencode** does not say, so nothing is shown for it.

Without `--refresh` the command shows what the daemon remembers and asks the agents only when it
knows nothing yet. `--refresh` (and the method `provider.limits` with `refresh`) asks each
provider now: it starts the agent for a moment without a prompt (`claude -p` answering `get_usage`,
`codex app-server` answering `account/rateLimits/read`), which costs nothing of the limits
themselves. A client that watches threads (`thread.watch`) also gets `limits.updated` with a
provider's limits whenever they change. A window whose reset time has passed is shown as reset:
what was spent in it no longer counts.

## Notes on a message

Along with a message, qcc can tell the agent a few facts the user did not write: what the message
is about (`--about <thread-id>`: "the user is looking at thread X, its title, agent and state"), or
what has changed since the session started. The bridge passes the notes the way its agent accepts
them; Claude Code, Codex and opencode receive them as a `<qcc-notes>` block before the text. Notes
are stored in the timeline with the message and are shown under it, both in the app and in
`qcc thread show`.

```sh
qcc thread send <thread-id> "why did it fail?" --about <other-thread-id>
```

## Attachments

Files and images can be attached to a message: `--attach <file>` (repeatable) on `thread send` and
`thread spawn`. In the app: the paperclip button in the composer, pasting from the clipboard
(`Ctrl+V`: an image, or files copied in a file manager), and dragging files onto the window. Before
sending, attachments are shown above the input field, and each one can be removed. qcc keeps a copy
of every file in the data directory (`attachments/<thread-id>/`), so the message in the timeline
looks the same later too. A file can be up to 50 MiB. Attachments are shown under the message in
the timeline (images as thumbnails) and in `qcc thread events`.

The bridge decides how an attachment reaches the agent. A provider that declared the `images`
capability (visible in `qcc plugin list`) gets the bytes of images (PNG, JPEG, GIF, WebP) and passes
them to the agent as images: Claude Code as stream-json image blocks, Codex as `localImage` with the
path of the copy, opencode as an ACP image block if the agent said during the handshake that it
accepts images. Other files (and images for an agent without this capability) reach the agent as a
note listing the paths of the copies, and the agent reads them itself.

```sh
qcc thread send <thread-id> "what is wrong in the screenshot?" --attach ~/shot.png --attach build.log
```

## Skills

A skill is a set of instructions a plugin offers to agents. A session learns the skill's name,
when to read it and the path to its file; the agent reads the file itself when a task fits. Most
skills are given to every session; an "on request" skill (`on-request` in the contract) is given
only to a thread that asked for it. A skill can be turned off for all threads or for one:

```sh
qcc skill list                                   # who offers what, "on request", "off"
qcc skill disable qcc-orchestration              # no session gets it
qcc skill enable qcc-orchestration
qcc thread spawn --skill echo-demo --without-skill qcc-orchestration --prompt "…"
qcc thread skills <thread-id> --on echo-demo --off qcc-orchestration
```

**The agents' own skills** (from `~/.claude/skills`, `~/.agents/skills`, their plugins) are only
shown by qcc: `qcc provider skills claude-code [--refresh]` and the "The agents' own" section on the
"Skills" screen. The list is requested from the agent itself, without a model request: Claude Code
answers `reload_skills` in stream-json, Codex answers `skills/list` in app-server. opencode does
not return the list reliably (in ACP skills are mixed with commands, and `opencode debug skill`
loses its output in a pipe), so for it the app says "does not say". Turning these skills on and off
is up to the agent's own configuration.

A running session learns about a skill turned on in its thread right away, as a note on the next
message. A skill turned off disappears with the thread's next session: what the agent has already
read cannot be taken back. A thread's choice is not passed to its children. In the app: the
"Skills" screen with a switch for all threads, and the "Skills" tab of a thread.

The `qcc-orchestration` skill and the commands of the `orchestration` plugin are described in
[Orchestration](orchestration.md#the-orchestration-plugin).
