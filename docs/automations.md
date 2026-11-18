# Automations

An automation is a prompt the daemon sends an agent on a schedule: "every hour, look at the failing
CI jobs", "every weekday at nine, summarize what changed in the repository". On its schedule qcc
starts a thread with the prompt, exactly as `qcc thread spawn` would, and records the run.

```sh
qcc automation add "Morning summary" --schedule "0 9 * * mon-fri" \
    --prompt "Summarize yesterday's commits and open TODOs" \
    --provider claude-code --model haiku --cwd ~/src/app --permission-mode accept-edits
qcc automation list                    # when each runs next and how its last run went
qcc automation show "Morning summary"  # its settings and its latest runs, with their threads
qcc automation run "Morning summary"   # run it now; its schedule goes on as before
qcc automation edit "Morning summary" --schedule @daily --env git-worktree
qcc automation disable "Morning summary"   # and `enable`
qcc automation remove "Morning summary"    # its threads stay
```

An automation is named by its id (`aut_…`) or its exact name. Every command takes `--json`. In
the desktop app the same lives on the **Automations** screen ([desktop.md](desktop.md)).

## Schedules

| schedule | when |
| --- | --- |
| `every 30m`, `every 1h30m`, `every 2d`, `every 45s` | at that interval, counted from the previous scheduled run (units `s`, `m`, `h`, `d`) |
| `0 9 * * mon-fri` | a cron expression: `minute hour day-of-month month day-of-week` |
| `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly` | the usual cron shorthands (`@midnight`, `@annually` too) |

Cron fields take `*`, numbers, names (`jan`–`dec`, `sun`–`sat`; `0` and `7` are both Sunday),
ranges `1-5`, steps `*/15` or `10-50/20`, and lists `5,35`. When both the day of the month and the
day of the week are restricted, a day matching either one counts, as in classic cron. Cron times
are wall-clock times of the daemon's local time zone: `0 9 * * *` is nine o'clock in winter and in
summer; a time the clock skips when it goes forward runs right after the jump. The interval after
`every` counts from when the automation was made, switched on, given its schedule or last came
due; a run by hand does not move it.

A schedule is checked when the automation is made or changed, and so are the provider, the
directory, the environment and the permission mode: a mistake shows up then, not at night.

## What a run is

Each run sends the automation's prompt with its provider, model, effort and permission mode, in its
directory, optionally in an environment of its own (`--env git-worktree`: a fresh worktree and
branch for every run). Model and effort left unset mean the defaults set for the agent
(`qcc settings provider`), then the agent's own.

- **A new thread for every run** (the default, `--threads new`). Runs do not see each other, and
  each one can be read, archived or delivered on its own. This is the right choice for most jobs.
- **One thread for all runs** (`--threads continue`). The first run starts a thread; every later
  run is a message to it, so the agent remembers what it did before. When that thread is archived,
  the next run starts a new one.

A thread an automation started is named after it and carries its id: `qcc thread show` prints
"started by automation aut_…", the JSON has `"automation": "aut_…"`, and the desktop app links the
thread to the automation. The thread is the user's own like any other: it belongs to the project
of its directory, shows in the overview, and can be answered, continued, archived or delivered.

Nobody watches a scheduled run. In `ask` mode an agent that wants approval waits in the thread
until you answer; pick the mode the job needs (`accept-edits` for a job that edits files in a
worktree, for example).

## When runs collide or are missed

**The previous run is still at work** (its turn runs, or the agent waits for an answer). By
default the new run is skipped and recorded as skipped, with the thread it waited on. With
`--overlap allow` it goes ahead: a new thread runs alongside, or, in `continue` mode, the message
waits in the thread's queue.

**The daemon was not running** (or the computer slept) when a run was due. By default the missed
run is recorded as skipped and the automation waits for its next time. With `--catch-up` it is
made up for as soon as the daemon notices — once, however many runs were missed. A run counts as
missed when it starts more than a minute late (or more than half the interval, for intervals
shorter than two minutes).

Switching an automation on, or giving it a new schedule, counts from that moment: runs it would
have had before are not owed. `qcc automation run` works on an automation that is switched off,
which is a way to keep a prompt at hand without a schedule.

## History

The daemon keeps the latest 100 runs of each automation: when it was due, when it ran, why
(`schedule`, `missed`, `manual`), what came of it (`started`, `skipped`, `failed`) with the reason,
and the thread. How the agent's work went is the thread's to tell: open it, or
`qcc thread wait <id>` for its last answer. Removing an automation removes its history; its
threads stay.

## How it is built

Automations live in the daemon's core, next to projects and settings, rather than in a plugin.
An automation is the user's own standing request: what it does on each run is what a client does
(`thread.spawn`, `thread.send`) with the user's authority, and the daemon owns the journal where
its definition and history belong. Nothing about it is specific to an agent; the core does not
look at which provider an automation uses. The API is `automation.list`, `automation.create`,
`automation.update`, `automation.set_enabled`, `automation.delete`, `automation.run` and
`automation.runs`; a client that watches threads is also told `automations.updated` whenever an
automation changes or runs.
