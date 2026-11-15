import { ChevronDownIcon } from "lucide-react";

import type { CommandContribution } from "../../api/protocol.gen";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

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
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant="outline" size="sm" title="Commands plugins offer for this thread" />}
      >
        More <ChevronDownIcon />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-auto min-w-48">
        {commands.map((command) => (
          <DropdownMenuItem key={commandKey(command)} title={command.description} onClick={() => run(command)}>
            {command.title}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
