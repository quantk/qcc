// Commands plugins contribute (`qcc <plugin> <command>` on the command line). Each
// one goes into the command palette; those that act on a thread are also offered
// in the thread's header. Running one asks for its parameters in a dialog.

import type { CommandContribution, CommandListResult, PluginInfo, ThreadId } from "../../api/protocol.gen";
import { store } from "../../state/store";
import type { UiModule, UiRegistry } from "../../ui/registry";
import { CommandDialog } from "./CommandDialog";
import { ThreadCommandMenu, commandKey } from "./ThreadCommandMenu";

const COMMANDS = "plugins.commands";

export function openCommand(ui: UiRegistry, command: CommandContribution, threadId: ThreadId | null) {
  ui.showDialog(({ close }) => <CommandDialog command={command} threadId={threadId} close={close} />);
}

/** The module made from the commands the daemon's plugins contribute right now. */
export function commandsModule({ commands }: CommandListResult): UiModule {
  return {
    id: COMMANDS,
    activate(ui) {
      for (const command of commands) {
        ui.command({
          id: commandKey(command),
          title: command.thread ? `${command.title} (this thread)` : command.title,
          enabled: () => !command.thread || store().getState().selected !== null,
          run: () => openCommand(ui, command, store().getState().selected),
        });
      }
      const forThreads = commands.filter((command) => command.thread);
      if (forThreads.length > 0) {
        ui.threadAction({
          id: "plugin-commands",
          order: 50,
          when: ({ thread }) => !thread.archived,
          render: ({ thread }) => (
            <ThreadCommandMenu commands={forThreads} run={(command) => openCommand(ui, command, thread.id)} />
          ),
        });
      }
    },
  };
}

export const pluginCommandsModule: UiModule = {
  id: "core.plugin-commands",
  activate(ui) {
    // Read again whenever the set of plugins is, like their UI.
    let seen: readonly PluginInfo[] | null = null;
    const sync = () => {
      const { plugins, daemon } = store().getState();
      if (plugins === seen || daemon.status !== "connected") return;
      seen = plugins;
      store()
        .api.call("command.list", {})
        .then((result) => {
          const shown = new Set(plugins.map((plugin) => plugin.id));
          const commands = result.commands.filter((command) => shown.has(command.plugin));
          if (seen === plugins) ui.activate(commandsModule({ commands }));
        })
        .catch(() => {
          seen = null;
        });
    };
    const stop = store().subscribe(sync);
    sync();
    return () => {
      stop();
      ui.deactivate(COMMANDS);
    };
  },
};
