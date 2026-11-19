import { describe, expect, it } from "vitest";

import type { NotificationShowNotification } from "../api/protocol.gen";
import { worthShowing } from "./notifications";

const finished: NotificationShowNotification = {
  subscription: 1,
  plugin: "notify",
  title: "Fix the login form",
  body: "Finished: all tests pass.",
  thread_id: "thr_1",
  quiet_when_shown: true,
};

describe("worthShowing", () => {
  it("stays quiet about the thread on screen in a focused window", () => {
    expect(worthShowing(finished, { focused: true, selected: "thr_1" })).toBe(false);
  });

  it("shows it when the window is not focused or another thread is open", () => {
    expect(worthShowing(finished, { focused: false, selected: "thr_1" })).toBe(true);
    expect(worthShowing(finished, { focused: true, selected: "thr_2" })).toBe(true);
    expect(worthShowing(finished, { focused: true, selected: null })).toBe(true);
  });

  it("shows it anyway when the plugin asks to", () => {
    const always = { ...finished, quiet_when_shown: false };
    expect(worthShowing(always, { focused: true, selected: "thr_1" })).toBe(true);
    const unsaid = { ...finished, quiet_when_shown: undefined };
    expect(worthShowing(unsaid, { focused: true, selected: "thr_1" })).toBe(true);
  });

  it("shows one about no thread in particular", () => {
    const general = { ...finished, thread_id: null };
    expect(worthShowing(general, { focused: true, selected: "thr_1" })).toBe(true);
  });
});
