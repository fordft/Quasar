import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { VoiceFaultV1 } from "$shared/ipc/bindings";
import { dailyResetTime } from "../lib/daily-reset";
import { voiceProblem } from "../lib/voice-problem";

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-10-20T10:30:00Z"));
});
afterEach(() => vi.useRealTimers());

test("the daily limit resets at the next 00:00 UTC on the person's clock", () => {
  const expected = new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(Date.UTC(2026, 9, 21));
  expect(dailyResetTime()).toBe(expected);
  expect(dailyResetTime(new Date("2026-10-20T23:59:59Z"))).toBe(expected);
});

test("every hosted refusal has its own message and one clear action", () => {
  const cases: [VoiceFaultV1, string, string | null][] = [
    ["plan_required", "Dictation needs a different Zephium plan.", "plans"],
    ["billing_review", "Your Zephium account is under billing review.", "account"],
    ["free_usage_paused", "Free Zephium AI usage is paused right now.", "plans"],
    ["hosted_paused", "Zephium AI is paused for now.", "settings"],
    ["invalid_audio", "Couldn’t read that recording. Try again.", "retry"],
    ["unsupported_audio", "That recording’s format isn’t supported.", "retry"],
    ["audio_too_long", "Recordings can be up to 4 minutes.", "retry"],
  ];
  for (const [fault, message, remedy] of cases) {
    const problem = voiceProblem(fault, "zephium");
    expect(problem.message, fault).toContain(message);
    expect(problem.remedy, fault).toBe(remedy);
  }
  const daily = voiceProblem("daily_limit", "zephium");
  expect(daily.message).toContain(`resets at ${dailyResetTime()}`);
  expect(daily.remedy).toBe("limit");
});

test("running out of Quasar credit points at plans, and a key's credit at settings", () => {
  expect(voiceProblem("out_of_credit", "zephium").remedy).toBe("plans");
  expect(voiceProblem("out_of_credit", "open_ai").remedy).toBe("settings");
});
