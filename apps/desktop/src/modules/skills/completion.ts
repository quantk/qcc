// The composer's `/` list: the skills the thread's agent can be pointed at.

import { SparklesIcon } from "lucide-react";

import { fuzzyRank } from "../../state/fuzzy";
import { offeredSkills, visibleSkills } from "../../state/skills";
import { store } from "../../state/store";
import type { ComposerCompletion } from "../../ui/registry";

/** At most this many entries; a narrower query finds the rest. */
const LIMIT = 50;

export const skillCompletion: ComposerCompletion = {
  id: "skills.complete",
  trigger: "/",
  order: 10,
  title: "Skills",
  async complete(query, { thread }) {
    const own = await store().agentSkillsOf(thread);
    const state = store().getState();
    const skills = offeredSkills(thread, visibleSkills(state.skills, state.plugins), state.settings?.settings, own);
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
