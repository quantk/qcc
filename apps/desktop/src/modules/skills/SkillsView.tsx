import { useMemo } from "react";

import { skillOff, visibleSkills, withSkillOff } from "../../state/skills";
import { store, useApp } from "../../state/store";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";

export const SKILLS_VIEW = "skills";

/** The skills plugins offer, with a switch that gives one to no thread. */
export function SkillsView() {
  const all = useApp((state) => state.skills);
  const plugins = useApp((state) => state.plugins);
  const settings = useApp((state) => state.settings?.settings);
  const skills = useMemo(() => visibleSkills(all, plugins), [all, plugins]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b px-6 pt-3.5 pb-3">
        <h1 className="text-lg">Skills</h1>
      </header>
      <div className="flex w-full max-w-3xl flex-col gap-3 overflow-auto px-6 pt-6 pb-10">
        <p className="text-[13px] text-muted-foreground">
          Instructions plugins offer agents: a session is told each one's name and where to read it, and the agent reads
          it when a task needs it. Most reach every thread; one offered on request reaches only the threads that ask for
          it. A switch here reaches the sessions that start after it; one thread's choice is on its Skills tab.
        </p>
        {skills.length === 0 && (
          <p className="rounded-lg border border-dashed p-4 text-[13px] text-muted-foreground">No plugin offers skills.</p>
        )}
        {skills.map((skill) => {
          const enabled = !skillOff(settings, skill.name);
          return (
            <article key={skill.name} className={`flex flex-col gap-1.5 rounded-lg border p-4${enabled ? "" : " opacity-70"}`}>
              <header className="flex items-center gap-2">
                <h3 className="font-mono text-sm font-semibold">{skill.name}</h3>
                <Badge variant="outline">{skill.plugin}</Badge>
                {skill.on_request && <Badge variant="secondary">on request</Badge>}
                {!enabled && <Badge variant="outline">off</Badge>}
                <span className="flex-1" />
                <Switch
                  aria-label={`Offer ${skill.name} to agents`}
                  title={enabled ? "Sessions get it" : "No session gets it"}
                  checked={enabled}
                  onCheckedChange={(on) => void store().changeSettings((next) => withSkillOff(next, skill.name, !on))}
                />
              </header>
              <p className="text-[13px]">{skill.description}</p>
              <code className="truncate text-xs text-muted-foreground" title={skill.path}>
                {skill.path}
              </code>
            </article>
          );
        })}
      </div>
    </div>
  );
}
