// The composer's `/` list: the skills the thread's agent can be pointed at, and
// the slash commands it runs, in a group of their own.

import { SparklesIcon, SquareSlashIcon } from "lucide-react";

import type { Thread } from "../../api/protocol.gen";
import type { AgentCatalog } from "../../state/agentCatalog";
import { fuzzyRank } from "../../state/fuzzy";
import { offeredCommands, offeredSkills, visibleSkills } from "../../state/skills";
import { store } from "../../state/store";
import type { ComposerCompletion } from "../../ui/registry";

/** At most this many entries of each kind; a narrower query finds the rest. */
const LIMIT = 50;

async function catalogOf(thread: Thread): Promise<AgentCatalog> {
  return store().agentCatalogs.get(thread.provider, thread.id);
}

function skillsOf(thread: Thread, own: AgentCatalog) {
  const state = store().getState();
  return offeredSkills(thread, visibleSkills(state.skills, state.plugins), state.settings?.settings, own.skills);
}

/** Both kinds look again when the list opens; both hear when that found more. */
const freshness: Pick<ComposerCompletion, "opened" | "subscribe"> = {
  opened: ({ thread }) => store().agentCatalogs.revalidate(thread.provider, thread.id),
  subscribe: (listener) => store().agentCatalogs.subscribe(listener),
};

export const skillCompletion: ComposerCompletion = {
  id: "skills.complete",
  trigger: "/",
  order: 10,
  title: "Skills",
  ...freshness,
  async complete(query, { thread }) {
    const skills = skillsOf(thread, await catalogOf(thread));
    return fuzzyRank(skills, query, (skill) => skill.name)
      .slice(0, LIMIT)
      .map(({ item: skill, match }) => ({
        key: skill.name,
        label: `/${skill.name}`,
        positions: match.positions.map((position) => position + 1),
        detail: skill.description,
        badge: skill.source ?? undefined,
        icon: SparklesIcon,
        insert: `/${skill.name} `,
      }));
  },
};

export const commandCompletion: ComposerCompletion = {
  id: "skills.commands",
  trigger: "/",
  order: 20,
  title: "Commands",
  ...freshness,
  async complete(query, { thread }) {
    const own = await catalogOf(thread);
    const commands = offeredCommands(own.commands, skillsOf(thread, own));
    return fuzzyRank(commands, query, (command) => command.name)
      .slice(0, LIMIT)
      .map(({ item: command, match }) => ({
        key: command.name,
        label: `/${command.name}`,
        positions: match.positions.map((position) => position + 1),
        detail: [command.argument_hint, command.description].filter(Boolean).join(" — "),
        badge: command.source ?? undefined,
        icon: SquareSlashIcon,
        insert: `/${command.name} `,
      }));
  },
};
