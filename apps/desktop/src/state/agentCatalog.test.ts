import { describe, expect, it } from "vitest";

import { type AgentCatalog, AgentCatalogs } from "./agentCatalog";

const catalog = (...skills: string[]): AgentCatalog => ({
  skills: skills.map((name) => ({ name, description: "" })),
  commands: [],
});

/** A fetch that answers from `answers`, and remembers how it was asked. */
function fake(answers: AgentCatalog[]) {
  const asked: boolean[] = [];
  const fetch = (_provider: string, _thread: string, refresh: boolean) => {
    asked.push(refresh);
    return Promise.resolve(answers[Math.min(asked.length - 1, answers.length - 1)]!);
  };
  return { asked, fetch };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("agent catalogs", () => {
  it("answers from what it knows and looks again in the background", async () => {
    const { asked, fetch } = fake([catalog("tdd"), catalog("tdd", "new")]);
    let now = 0;
    const catalogs = new AgentCatalogs(fetch, 3000, () => now);
    let heard = 0;
    catalogs.subscribe(() => heard++);

    expect((await catalogs.get("claude", "t")).skills.map((s) => s.name)).toEqual(["tdd"]);
    expect(asked).toEqual([false]);

    // A look right after the first is not worth running the agent again.
    catalogs.revalidate("claude", "t");
    expect(asked).toEqual([false]);

    now = 5000;
    catalogs.revalidate("claude", "t");
    catalogs.revalidate("claude", "t");
    expect(asked).toEqual([false, true]);
    // Still the old answer until the new one is in.
    await settle();
    expect(heard).toBe(1);
    expect((await catalogs.get("claude", "t")).skills.map((s) => s.name)).toEqual(["tdd", "new"]);
  });

  it("stays quiet when nothing changed and keeps threads apart", async () => {
    const { asked, fetch } = fake([catalog("tdd")]);
    let now = 0;
    const catalogs = new AgentCatalogs(fetch, 3000, () => now);
    let heard = 0;
    catalogs.subscribe(() => heard++);
    await catalogs.get("claude", "a");
    now = 10_000;
    catalogs.revalidate("claude", "a");
    await settle();
    expect(heard).toBe(0);
    // The first look at a thread shows what the daemon kept, then asks the agent.
    catalogs.revalidate("claude", "b");
    await settle();
    expect(asked).toEqual([false, true, false, true]);
  });

  it("asks again after a failure", async () => {
    let fail = true;
    const catalogs = new AgentCatalogs((_p, _t, _r) =>
      fail ? Promise.reject(new Error("gone")) : Promise.resolve(catalog("x")),
    );
    expect(await catalogs.get("claude", "t")).toEqual({ skills: [], commands: [] });
    fail = false;
    expect((await catalogs.get("claude", "t")).skills).toHaveLength(1);
  });
});
