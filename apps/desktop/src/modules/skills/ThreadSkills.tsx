import { useMemo } from "react";

import type { Thread } from "../../api/protocol.gen";
import { threadSkills, visibleSkills } from "../../state/skills";
import { store, useApp } from "../../state/store";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { SKILLS_VIEW } from "./SkillsView";

/** Which skills the thread's sessions get, with switches for this thread alone. */
export function ThreadSkills({ thread }: { thread: Thread }) {
  const all = useApp((state) => state.skills);
  const plugins = useApp((state) => state.plugins);
  const settings = useApp((state) => state.settings?.settings);
  const rows = useMemo(() => threadSkills(thread, visibleSkills(all, plugins), settings), [thread, all, plugins, settings]);

  return (
    <div className="flex w-full max-w-3xl flex-col gap-3 overflow-auto px-6 py-5">
      <p className="text-[13px] text-muted-foreground">
        The skills this thread's sessions are told about, for this thread alone, not the threads it starts. One turned
        on reaches the running session with the next message. One turned off is left out from the thread's next session:
        the agent may have read it already.
      </p>
      <ul className="flex flex-col divide-y rounded-lg border">
        {rows.map(({ skill, on, offForAll }) => (
          <li key={skill.name} className="flex flex-col gap-1 px-4 py-2.5">
            <div className="flex items-center gap-2">
              <span className={`font-mono text-sm font-semibold${on ? "" : " text-muted-foreground"}`}>{skill.name}</span>
              {skill.on_request && <Badge variant="secondary">on request</Badge>}
              <span className="flex-1" />
              <Switch
                aria-label={`Give this thread ${skill.name}`}
                title={offForAll ? "Off for every thread" : on ? "On in this thread" : "Off in this thread"}
                checked={on}
                disabled={offForAll || thread.archived}
                onCheckedChange={(next) => void store().setThreadSkill(thread.id, skill.name, next)}
              />
            </div>
            <p className="text-xs text-muted-foreground">{skill.description}</p>
            {offForAll && <p className="text-xs text-muted-foreground">Off for every thread on the Skills screen.</p>}
          </li>
        ))}
      </ul>
      <Button variant="ghost" size="sm" className="self-start" onClick={() => store().openView(SKILLS_VIEW)}>
        Manage skills
      </Button>
    </div>
  );
}
