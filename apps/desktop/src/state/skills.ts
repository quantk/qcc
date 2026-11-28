// Skills: the instructions plugins offer agents, which of them the user turned
// off for every thread (a part of the daemon's settings), and which one thread
// takes or leaves out.

import type { AgentSkill, PluginInfo, Settings, SkillStatus, Thread } from "../api/protocol.gen";

/** Off for every thread. */
export function skillOff(settings: Settings | undefined, name: string): boolean {
  return settings?.skills?.[name]?.disabled ?? false;
}

/** Turn a skill off for every thread, or back on; on is the default and is not kept. */
export function withSkillOff(settings: Settings, name: string, off: boolean): Settings {
  const skills = { ...settings.skills };
  if (off) skills[name] = { disabled: true };
  else delete skills[name];
  return { ...settings, skills };
}

/** The skills of the plugins the window offers, by name. */
export function visibleSkills(skills: readonly SkillStatus[], plugins: readonly PluginInfo[]): SkillStatus[] {
  const shown = new Set(plugins.map((plugin) => plugin.id));
  return skills.filter((skill) => shown.has(skill.plugin)).sort((a, b) => a.name.localeCompare(b.name));
}

export type ThreadSkill = {
  skill: SkillStatus;
  /** The thread's sessions get it. */
  on: boolean;
  /** Off for every thread: the thread's choice does not count. */
  offForAll: boolean;
};

/** Every skill, and whether the thread's sessions get it. */
export function threadSkills(thread: Thread, skills: readonly SkillStatus[], settings: Settings | undefined): ThreadSkill[] {
  return skills.map((skill) => {
    const offForAll = skillOff(settings, skill.name);
    const chosen = skill.on_request
      ? (thread.skills_enabled ?? []).includes(skill.name)
      : !(thread.skills_disabled ?? []).includes(skill.name);
    return { skill, on: !offForAll && chosen, offForAll };
  });
}

/** A skill the composer offers after `/`. */
export type OfferedSkill = {
  name: string;
  description: string;
  /** Where it is from: `qcc` for a plugin's, else as the agent says (`user`, `project`, …). */
  source: string | null;
};

/**
 * The skills a thread's agent can be pointed at with `/name`: qcc's that its
 * sessions get, then the agent's own, each name once (qcc's first), by name.
 */
export function offeredSkills(
  thread: Thread,
  skills: readonly SkillStatus[],
  settings: Settings | undefined,
  own: readonly AgentSkill[],
): OfferedSkill[] {
  const offered = new Map<string, OfferedSkill>();
  for (const { skill, on } of threadSkills(thread, skills, settings)) {
    if (on) offered.set(skill.name, { name: skill.name, description: skill.description, source: "qcc" });
  }
  for (const skill of own) {
    if (!offered.has(skill.name)) {
      offered.set(skill.name, { name: skill.name, description: skill.description, source: skill.source ?? null });
    }
  }
  return [...offered.values()].sort((a, b) => a.name.localeCompare(b.name));
}
