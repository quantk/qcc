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
`divider`). The app draws the nodes it knows and skips the rest.

```rust
use qcc_plugin_sdk::{prelude::*, ui};   // feature "ui"

impl Ui for Notes {
    fn views(&self) -> Vec<UiViewInfo> {
        vec![UiViewInfo {
            id: "notes".into(),
            title: "Notes".into(),
            placement: UiPlacement::ThreadTab,   // or App: a separate screen
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

`Plugin::settings` declares the plugin's settings (`SettingField` with `SettingKind`), and
`Plugin::on_settings` receives their values: after the daemon starts and on every change. No
permission is needed for this: a plugin sees only its own settings. Example: the settings of
`plugins/provider-echo` (how users change them is in
[Plugin settings](agents.md#plugin-settings)).

## Installing and permissions

An installed plugin lives in `<data>/plugins/<id>/`. A third-party plugin is granted permissions by
the file `<data>/grants/<id>.toml`, in the format of the manifest's `[permissions]` section; without
it the plugin is loaded without permissions.
