# Writing a plugin

A plugin is a `wasm32-wasip2` component plus a `plugin.toml`. See `plugins/provider-echo`: it is a
complete example on the SDK, and `crates/qcc-plugin-testkit/tests/echo.rs` shows how to run a bridge
through the real host and compare its timeline with the golden one (`QCC_UPDATE_GOLDEN=1` rewrites
the golden files).

Each plugin instance is a single-threaded actor with an event queue, and the host limits it: the
memory of all instances of one plugin is up to 256 MiB, one `handle` call takes up to 10 s, and
pending calls (`deferred`) and timers are limited to 1024. If a plugin cannot keep up with a
process's output, the process waits while more than 8 MiB is queued. The executable is looked up
in the daemon's `PATH`, not in a `PATH` passed by the plugin. The `kv` store allows values up to
1 MiB, up to 64 MiB and 10,000 keys in total.

The contract is `wit/plugin.wit`; how the pieces fit together is in
[Development](development.md#architecture).

## Plugin UI

A plugin can add its own screen and thread tab to the app, and draw timeline items of its own
kind. It does not send markup or code; it answers with a tree of nodes from a fixed set (`stack`,
`row`, `heading`, `text`, `markdown`, `code`, `badge`, `keyValues`, `table`, `button`, `input`,
`select`, `switch`, `divider`). The app draws the nodes it knows and skips the rest. A `select`
sends its action with the value picked, a `switch` with `true` or `false`, and an `input` given a
value starts with it and keeps what was submitted: enough for a small settings form.

```rust
use qcc_plugin_sdk::{prelude::*, ui};   // feature "ui"

impl Ui for Notes {
    fn views(&self) -> Vec<UiViewInfo> {
        vec![UiViewInfo {
            id: "notes".into(),
            title: "Notes".into(),
            placement: UiPlacement::ThreadTab,   // App: a separate screen; ThreadPanel: the panel beside the thread
            scope: UiScope::AllThreads,          // or OwnThreads: only threads of its own providers
        }]
    }

    fn render(&mut self, request: UiRender) -> Result<String, String> {
        Ok(ui::stack([ui::heading("Notes"), ui::button("Add", "add").primary()]).to_json())
    }

    // A button press or an input field submission; the answer replaces the view.
    fn action(&mut self, action: UiAction) -> Result<String, String> { /* … */ }
}
```

When nodes are not enough (a canvas, a chart, a complex form), a view can be a page of a web
bundle: `web = "web"` in the manifest, `web: Some("index.html".into())` in the contribution. The
page opens in a sandboxed frame under the plugin's own origin (`qcc-plugin://<id>`), with no access
to the app and its IPC, to other plugins, or to the network. The only way out is a `postMessage`
bridge that delivers an action to the same plugin's `Ui::action` and returns its answer (example:
`plugins/provider-echo/web`).

A timeline item of its own kind (`ItemShape::Extension`, the kind is always `<plugin-id>/<name>`)
is drawn through `item_kinds` and `render_item`; a plugin cannot take over other plugins' kinds. A
working example is the "Echo" tab and the `/card` command in `plugins/provider-echo`. In the app,
plugin contributions are registered in the same registry as the built-in modules
(`apps/desktop/src/modules/plugin-ui`).

## Commands, skills and threads

`Commands` adds commands for the CLI (`qcc <plugin-id> <command>`) and the app's palette. A command
has a title, a description, an "acts on a thread" flag and text parameters. The daemon itself checks
that the thread exists and that required parameters are given. `Plugin::skills` offers instructions
to agents (the `skills` permission: their text can reach every session); a skill with `on_request`
is given only to threads that asked for it (`SpawnOptions::skills_enabled`). The `threads` import
(the `threads` permission) creates, reads and directs threads as a client of the daemon, and
`threads::watch` sends the `on_thread_settled` event when a thread finishes. An example of all of
this is `plugins/orchestration` (see [Orchestration](orchestration.md)).

A few more imports serve plugins that work alongside the user's threads (an example of all of them
is `plugins/reviewers`, see [Reviewers](reviewers.md)):

- **Turn events** (permission `turn-events`): `Plugin::on_turn_ended` hears of every turn that ends,
  in every thread, with the thread as it is then and how the turn ended. Unlike `threads::watch`,
  nothing needs to be subscribed again, and a thread that is already idle does not fire.
- **Diffs** (`threads`): `threads::diff` asks for one patch of what a thread changed, as `qcc thread
  diff` shows it; the environment that knows may take a while, so the answer comes later to
  `Plugin::on_thread_diff` with the id `diff` returned.
- **Skills of a thread** (`threads`): `threads::set_skill` turns one of the plugin's own skills on or
  off in a thread, the way the user can. **Agents** (`threads`): `threads::providers` lists them.
- **Comments** (permission `comments`): `comments::add` leaves a comment on a line of a thread's
  changes, and `comments::list` reads them. The daemon records the plugin as the comment's author
  (with the name, severity, reviewer thread and stale mark the plugin gives); the app shows it, and
  the note that hands comments to an agent says whose each one is.
- **Files** (permission `files`): `files::read` and `files::list` read text files in a thread's
  working directory (`Root::Thread`) or in the plugin's own folder `<data>/plugin-data/<id>`
  (`Root::Own`); paths are relative and cannot leave the root, through `..` or a link. Files are
  read up to 1 MiB.

`Plugin::settings` declares the plugin's settings (`SettingField` with `SettingKind`), and
`Plugin::on_settings` receives their values: after the daemon starts and on every change. No
permission is needed for this: a plugin sees only its own settings. Example: the settings of
`plugins/provider-echo` (how users change them is in
[Plugin settings](agents.md#plugin-settings)).

## Installing and permissions

An installed plugin lives in `<data>/plugins/<id>/`. A third-party plugin is granted permissions by
the file `<data>/grants/<id>.toml`, in the format of the manifest's `[permissions]` section; without
it the plugin is loaded without permissions. The manifest may carry a one-line `description`, shown
in `qcc plugin list` and in the app's settings.

## Turning plugins on and off

Any plugin can be turned off, bundled ones included (they have no privileges, so nothing depends on
them being there):

```sh
qcc plugin list                          # every installed plugin; "disabled" marks those turned off
qcc plugin disable provider-codex        # stop it now and on every later start
qcc plugin disable provider-codex --force   # even while its agents are working
qcc plugin enable provider-codex         # start it again, no restart needed
```

The choice is kept in `settings.json` (`disabled_plugins`) and is changed only by these commands
(`plugin.enable` / `plugin.disable` in the protocol): `settings.set` leaves it as it is. Both apply
at once. A plugin turned off is unloaded: its processes are killed, and its agents, environments,
commands, skills, settings fields and views disappear from the daemon and the app. Watching
clients get a `plugins.updated` notification. `plugin.list` still lists it, with `enabled: false`
and what its manifest says (version, description, permissions), but with no contributions. A
plugin that is on but failed to start is listed with its `error`.

Threads made with a plugin that is off:

- **Working agents block turning it off.** `plugin disable` is refused (`THREAD_BUSY`, naming the
  threads) while one of its agents is in a turn or starting a session, or while it is preparing or
  removing a worktree; `--force` cuts those turns short instead (they settle as interrupted). Idle
  sessions of its agents are closed either way, the way a daemon shutdown closes them: the
  timeline gets `session/ended` with the reason "plugin … was turned off".
- **The timeline stays readable**: it lives in the journal, not in the plugin.
- **New messages are refused** with `PLUGIN_DISABLED` (-32009): `agent "codex" belongs to
  plugin provider-codex, which is turned off`. So is a new thread with its agent or environment.
  Messages already queued wait and are delivered once it is back on, and the session is resumed
  as after a restart. A thread already in a worktree keeps working when only the worktree plugin
  is off, but its changes cannot be listed, and archiving it needs `--force`, which leaves the worktree on
  disk.

To tell which plugin an agent or environment belonged to once it is off, the daemon remembers what
each plugin contributed when it last ran (`<cache>/plugins/<id>.json`); without that record the
error is the plain "no plugin provides …".
