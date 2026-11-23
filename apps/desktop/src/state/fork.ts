// Where a thread can be forked from: the end of each ended turn, in the provider
// session the turn ran in. The same rules as the daemon's (`qcc_domain::fork`), so
// the timeline can tell before asking why a reply cannot be forked from.

import type { ForkSupport, ProviderCapabilities, ThreadEvent, TurnId } from "../api/protocol.gen";

/** One ended turn, as a fork needs to know it. */
export type TurnPoint = {
  turnId: TurnId;
  /** The agent's last message in it, which a fork from it is offered under. */
  replyItem?: string;
  /** Where its session stood after it, in the provider's terms. */
  point?: string;
  /** The provider session it ran in. */
  session?: string;
  provider: string;
  /** Which of `Turns.started` it is. */
  started: number;
};

export type Turns = {
  /** The ended turns, oldest first. */
  ended: readonly TurnPoint[];
  /** The session each turn started in, the running one included. */
  started: readonly (string | undefined)[];
  provider: string;
  session?: string;
  /** The agent's latest message in the running turn. */
  reply?: string;
};

export const emptyTurns: Turns = { ended: [], started: [], provider: "" };

/** Fold one event in. */
export function applyTurns(turns: Turns, event: ThreadEvent): Turns {
  switch (event.type) {
    case "thread/created":
      return { ...turns, provider: event.thread.provider, session: event.thread.provider_session ?? undefined };
    case "session/started":
      return event.provider_session ? { ...turns, session: event.provider_session } : turns;
    case "session/identified":
      return { ...turns, session: event.provider_session };
    case "thread/agent":
      return event.provider === turns.provider ? turns : { ...turns, provider: event.provider, session: undefined };
    case "thread/reset":
    case "environment/attached":
      return { ...turns, session: undefined };
    case "turn/started":
      return { ...turns, started: [...turns.started, turns.session], reply: undefined };
    case "item/completed":
      return event.item.type === "agentMessage" ? { ...turns, reply: event.item.id } : turns;
    case "turn/completed": {
      // A turn the journal saw end without starting counts as started now.
      const started =
        turns.started.length === turns.ended.length ? [...turns.started, turns.session] : turns.started;
      const ended: TurnPoint = {
        turnId: event.turn_id,
        replyItem: turns.reply,
        point: event.point ?? undefined,
        session: turns.session,
        provider: turns.provider,
        started: started.length - 1,
      };
      return { ...turns, started, ended: [...turns.ended, ended], reply: undefined };
    }
    default:
      return turns;
  }
}

/** No turn of the same session started after this one, the running one included. */
export function isLatest(turns: Turns, turn: TurnPoint): boolean {
  return !turns.started.slice(turn.started + 1).includes(turn.session);
}

/** What the timeline needs to know about an agent: its name and what it can do. */
export type Agent = { name: string; capabilities: ProviderCapabilities };

/** Why the thread cannot be forked from `turn`; null when it can. */
export function forkBlocker(turns: Turns, turn: TurnPoint, agent: Agent | undefined): string | null {
  if (!agent) return `No plugin provides ${turn.provider} now`;
  const fork: ForkSupport = agent.capabilities.fork ?? "none";
  if (fork === "none" || !agent.capabilities.resume) return `${agent.name} cannot fork a conversation`;
  if (!turn.point || !turn.session) {
    return "This reply has no point to fork from: it came before qcc could fork, or the agent did not say where its session stood";
  }
  if (fork === "latest" && !isLatest(turns, turn)) {
    return `${agent.name} forks a conversation only from its latest reply`;
  }
  return null;
}

/** The ended turn whose last reply is `itemId`, if it is one. */
export function turnOfReply(turns: Turns, itemId: string): TurnPoint | undefined {
  return turns.ended.find((turn) => turn.replyItem === itemId);
}
