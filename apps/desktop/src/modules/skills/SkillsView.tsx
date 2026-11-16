import { ChevronRightIcon, RefreshCwIcon } from "lucide-react";
import { useEffect, useMemo } from "react";

import type { ProviderInfo } from "../../api/protocol.gen";
import { pickableProviders } from "../../state/selectors";
import { skillOff, visibleSkills, withSkillOff } from "../../state/skills";
import { store, useApp } from "../../state/store";
import { plural } from "../../util/format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Switch } from "@/components/ui/switch";

export const SKILLS_VIEW = "skills";

/** The skills plugins offer, with a switch that gives one to no thread. */
export function SkillsView() {
  const all = useApp((state) => state.skills);
  const plugins = useApp((state) => state.plugins);
  const settings = useApp((state) => state.settings?.settings);
  const skills = useMemo(() => visibleSkills(all, plugins), [all, plugins]);
  const providers = useMemo(() => pickableProviders(plugins, settings), [plugins, settings]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b px-6 pt-3.5 pb-3">
        <h1 className="text-lg">Skills</h1>
      </header>
      <div className="flex w-full max-w-3xl flex-col gap-3 overflow-auto px-6 pt-6 pb-10">
        <h2 className="text-base font-semibold">Offered by qcc's plugins</h2>
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
        {providers.length > 0 && (
          <section className="mt-4 flex flex-col gap-2">
            <h2 className="text-base font-semibold">The agents' own</h2>
            <p className="text-[13px] text-muted-foreground">
              Skills each agent finds by itself: in its own folders (<code>~/.claude/skills</code>,{" "}
              <code>~/.agents/skills</code>, …) and its plugins, as it reports them. Shown here, not switched: the
              agent's own configuration decides.
            </p>
            {providers.map((provider) => (
              <AgentSkills key={provider.id} provider={provider} />
            ))}
          </section>
        )}
      </div>
    </div>
  );
}

/** One agent's own skills, asked of it when the list is first opened. */
function AgentSkills({ provider }: { provider: ProviderInfo }) {
  const list = useApp((state) => state.agentSkills[provider.id]);
  const can = provider.capabilities.list_skills;
  useEffect(() => {
    if (can) void store().loadAgentSkills(provider.id);
  }, [can, provider.id]);
  const skills = list?.items ?? [];
  const summary = !can
    ? "does not say which skills it finds"
    : list?.loading
      ? "asking the agent…"
      : list?.error
        ? `could not list them: ${list.error}`
        : plural(skills.length, "skill");

  return (
    <Collapsible className="rounded-lg border">
      <div className="flex items-center gap-2 px-4 py-2.5">
        <CollapsibleTrigger
          className="group flex flex-1 items-center gap-2 text-left disabled:cursor-default"
          disabled={!can || skills.length === 0}
        >
          <ChevronRightIcon className="size-4 transition-transform group-data-[panel-open]:rotate-90" />
          <span className="font-semibold">{provider.name}</span>
          <span className="truncate text-[13px] text-muted-foreground">{summary}</span>
        </CollapsibleTrigger>
        {can && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Ask ${provider.name} for its skills again`}
            title="Ask the agent again"
            disabled={list?.loading}
            onClick={() => void store().loadAgentSkills(provider.id, true)}
          >
            <RefreshCwIcon className={list?.loading ? "animate-spin" : undefined} />
          </Button>
        )}
      </div>
      <CollapsibleContent>
        <ul className="flex flex-col divide-y border-t">
          {skills.map((skill) => (
            <li key={`${skill.name}:${skill.path ?? ""}`} className="flex flex-col gap-0.5 px-4 py-2">
              <div className="flex items-center gap-2">
                <span className="font-mono text-sm font-semibold">{skill.name}</span>
                {skill.source && <Badge variant="outline">{skill.source}</Badge>}
              </div>
              <p className="line-clamp-2 text-xs text-muted-foreground" title={skill.description}>
                {skill.description}
              </p>
              {skill.path && (
                <code className="truncate text-xs text-muted-foreground" title={skill.path}>
                  {skill.path}
                </code>
              )}
            </li>
          ))}
        </ul>
      </CollapsibleContent>
    </Collapsible>
  );
}
