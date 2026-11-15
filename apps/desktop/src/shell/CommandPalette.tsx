import type { Command } from "../ui/registry";
import {
  CommandDialog,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
  Command as CommandRoot,
  CommandShortcut,
} from "@/components/ui/command";

/** Every contributed command that is available now, filtered as you type. */
export function CommandPalette({ commands, close }: { commands: readonly Command[]; close: () => void }) {
  const available = commands.filter((command) => command.enabled?.() ?? true);

  const run = (command: Command) => {
    close();
    command.run();
  };

  return (
    <CommandDialog open onOpenChange={(open) => !open && close()} title="Commands" description="Run a command">
      <CommandRoot>
        <CommandInput placeholder="Run a command…" />
        <CommandList>
          <CommandEmpty>No matching command.</CommandEmpty>
          {available.map((command) => (
            // The id keeps two commands with the same title apart.
            <CommandItem key={command.id} value={`${command.title} ${command.id}`} onSelect={() => run(command)}>
              <span>{command.title}</span>
              {command.shortcut && <CommandShortcut>{command.shortcut}</CommandShortcut>}
            </CommandItem>
          ))}
        </CommandList>
      </CommandRoot>
    </CommandDialog>
  );
}
