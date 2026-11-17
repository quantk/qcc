// Skills: a screen with the skills plugins offer agents and a switch for all
// threads, and a tab that switches them in one thread.

import { SparklesIcon } from "lucide-react";

import { visibleSkills } from "../../state/skills";
import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { SKILLS_VIEW, SkillsView } from "./SkillsView";
import { ThreadSkills } from "./ThreadSkills";

export const skillsModule: UiModule = {
  id: "core.skills",
  activate(registry) {
    registry.appView({ id: SKILLS_VIEW, title: "Skills", order: 95, icon: SparklesIcon, place: "bottom", render: SkillsView });
    registry.threadTab({
      id: "skills",
      title: "Skills",
      order: 35,
      when: () => {
        const state = store().getState();
        return visibleSkills(state.skills, state.plugins).length > 0;
      },
      render: ({ thread }) => <ThreadSkills thread={thread} />,
    });
    registry.command({ id: "skills.open", title: "Skills", run: () => store().openView(SKILLS_VIEW) });
  },
};
