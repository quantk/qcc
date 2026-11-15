import type { CommandContribution } from "../../api/protocol.gen";

export const commandKey = (command: CommandContribution) => `plugin:${command.plugin}/${command.id}`;

/** A compact menu of the plugin commands that act on a thread. */
export function ThreadCommandMenu({
  commands,
  run,
}: {
  commands: readonly CommandContribution[];
  run: (command: CommandContribution) => void;
}) {
  return (
    <select
      className="small commands"
      value=""
      title="Commands plugins offer for this thread"
      onChange={(event) => {
        const command = commands.find((candidate) => commandKey(candidate) === event.target.value);
        if (command) run(command);
      }}
    >
      <option value="">More…</option>
      {commands.map((command) => (
        <option key={commandKey(command)} value={commandKey(command)} title={command.description}>
          {command.title}
        </option>
      ))}
    </select>
  );
}
