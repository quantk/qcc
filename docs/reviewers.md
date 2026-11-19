# Reviewers

The `reviewers` plugin gives a thread reviewers of its own: other agents that look at what the
thread changed once its agent is done, and report what they find as comments on the lines of the
changes. You choose who reviews (profiles), when (the trigger), and whether the findings only wait
for you or also go to the agent.

It is an ordinary plugin with no privileges. Turned off (`qcc plugin disable reviewers`), it starts
no reviewer, hears of no turn and shows no panel; the comments and reviewer threads it already made
stay, like any others. Turned on again, each thread's set-up is where it was.

## Setting up a thread

In the app: the **Reviewers** tab of the panel beside the thread (`Ctrl+B`). From the CLI:

```sh
qcc reviewers configure --thread <id> --enabled on --profiles general,security
qcc reviewers configure --thread <id>               # how it is set up now
qcc reviewers profiles --thread <id>                # the profiles it can use
```

From inside an agent `--thread` can be left out: it is the agent's own thread.

In the panel the agent is picked from those the daemon runs, then its model from the models that
agent lists (the same list as the new-thread dialog), and the effort from the levels that model
takes; each also has "As the profile says". Until the agent's models are in, the panel says it is
loading them and is drawn again when they come. Another agent clears the model and effort; a model
that does not take the chosen effort clears the effort. An agent that lists no models, or cannot be
asked, gets text fields for the model and effort instead (and a button to ask again).

| Setting | `configure` | Default | |
| --- | --- | --- | --- |
| Review on its own | `--enabled on/off` | off | Reviews start by the trigger. A review asked for by hand runs either way. |
| Profiles | `--profiles a,b` | `general` | Each chosen profile reviews in a thread of its own. |
| Agent, model, effort | `--provider`, `--model`, `--effort` | the profile's | Override every profile's choice; an empty value goes back to it. |
| Delivery | `--delivery collect/auto` | `collect` | See [Findings](#findings). A profile may override it. |
| Trigger | `--trigger finished/every-turn/every-turns/manual` | `finished` | See [When a review starts](#when-a-review-starts). |
| Delay | `--delay 30s` | 30s | For `finished`: how long the agent stays idle first. |
| Every | `--every 3` | 3 | For `every-turns`. |
| Rounds at most | `--max-rounds 3` | 3 | Per thread; `0` is no limit. `qcc reviewers reset --thread <id>` counts again. |
| Review answers | `--review-responses on/off` | off | Whether the turn in which the agent answers the findings is reviewed too. |

The defaults of a new thread are the plugin's own settings (app settings, or `qcc plugin set
reviewers …`): `profiles`, `provider`, `model`, `effort`, `delivery`, `trigger`, `delay`,
`max_rounds`.

Who runs a reviewer is decided in this order: the thread's choice, the profile, the plugin's
settings, and last the reviewed thread's own agent. A model or effort is taken only along with the
agent it was given for (or one given without an agent).

## Profiles

A profile is a markdown file: a frontmatter saying who reviews and how, and a body saying what to
look for.

```markdown
---
name: security                 # lowercase letters, digits, - and _; default: the file name
description: Security problems in what changed
provider: codex                # optional
model: gpt-5.1-codex           # optional
effort: high                   # optional
paths: [src/**/*.rs, "*.toml"] # optional: only these files; a glob without / matches the file name anywhere
severity: medium               # findings below it are dropped: info, low, medium, high, critical
delivery: auto                 # optional: collect or auto, over the thread's choice
---
Look only for security problems: injection, secrets, missing permission checks…
```

They come from three places; one of the same name in a later place replaces the earlier:

1. built in: `general` (bugs, risks, missing tests) and `security`;
2. global: `<data>/plugin-data/reviewers/profiles/*.md` (`~/.local/share/qcc/…` for `./dev app`);
3. the project: `.qcc/reviewers/*.md` in the directory the thread works in, so a worktree thread
   sees the profiles of its branch.

A profile that cannot be read is named in the panel and in `qcc reviewers profiles`.

## When a review starts

- **When the agent is done** (`finished`, the default): a turn completed, and no new turn started
  and no message came for the delay. A turn that starts meanwhile puts it off until that one ends.
- **After every turn** (`every-turn`) and **every few turns** (`every-turns`), counting completed
  turns.
- **Only when asked** (`manual`): from the panel's **Review now**, or `qcc reviewers request`.
- The agent can ask too: a thread with reviews on gets the `qcc-reviewers` skill, which tells it to
  run `qcc reviewers request` when it is done. That starts a round at once, without the delay.

(`commit` and `deliver` triggers are reserved in the settings but not wired up yet.)

Whatever the trigger, a round starts only if:

- there are changes, and they are not the ones the last round reviewed (the diff's fingerprint is
  compared);
- no round is running;
- the thread has not had its rounds at most;
- the turn was not the agent's answer to the findings sent to it, unless reviewing answers is on.

`qcc reviewers request` says why when it starts nothing.

## Reviewers

Each profile of a round is a child thread of the reviewed one, in its directory, in the
**`read-only`** permission mode (see [Permission mode](agents.md#permission-mode)): it reads and
looks around, nothing that changes a file runs, and it never stops to ask you. Claude Code, Codex
and opencode can run that way. An agent that cannot runs its reviewer in `ask` mode instead, and
then it may ask you for approval; the panel says so before a round, and `qcc reviewers request`
says so when it starts one. You can open it like any other thread. Its prompt has the diff (the files its
profile covers), the task the agent was given and how to answer: a JSON object at the end of the
reply,

```json
{"findings": [{"file": "src/lib.rs", "line": 12, "severity": "high", "text": "…"}]}
```

From the second round on, a reviewer gets only the files changed since the last round, and what it
found before with what became of it (resolved, with the agent's note, or still open), so it checks
the fixes instead of starting over.

## Findings

Once every reviewer of the round answered:

- each finding with a file and a line of the changes becomes a **line comment**, marked with the
  reviewer's name and severity (Changes tab and panel). The mark opens the reviewer's thread.
  If the changes moved on while the reviewers worked, the comments are marked **Outdated**: the
  line may be off. A finding about the change as a whole, or about a file the changes do not touch,
  stays a remark;
- a reviewer that did not answer in the format has its reply kept as one remark;
- with **collect** delivery that is all: **Send to agent** on the Changes tab hands the comments over
  when you choose;
- with **auto** delivery the agent also gets **one message** for the whole round (queued if it is
  working) listing the findings and remarks, each with its comment id. The agent resolves the
  comments with `qcc comment resolve <id> --note …` as it deals with them.

The panel shows the rounds of the thread, the round running, and why the last attempt started
nothing.

## How it is built

The plugin uses only the public plugin API (see [Writing a plugin](plugins.md)), with the
permissions `kv`, `timers`, `threads`, `turn-events`, `comments`, `files` and `skills`:

- `turn-events` tells it of every turn that ends; it keeps a timer per thread for the delay and
  checks with `threads.get` that the agent is still idle;
- `threads.diff` reads the diff (answered later, as an event), `threads.spawn` and
  `threads.watch` run the reviewers, `threads.send` delivers the message;
- `comments.add` leaves the comments; the daemon records the plugin as their author;
- `files` reads the profiles; the panel is a `thread-panel` view built from UI nodes.

Rounds in flight are kept in its key-value store and picked up again after a restart of the
daemon; the timer of a delay is not.
