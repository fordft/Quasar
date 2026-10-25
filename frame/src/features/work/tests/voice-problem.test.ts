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

test("retired cloud dictation never points to billing or provider key setup", () => {
  const cases: VoiceFaultV1[] = [
    "plan_required",
    "billing_review",
    "free_usage_paused",
    "hosted_paused",
    "daily_limit",
    "out_of_credit",
    "key_refused",
    "signed_out",
  ];
  for (const engine of ["zephium", "open_ai", "groq"] as const) {
    for (const fault of cases) {
      const problem = voiceProblem(fault, engine);
      expect(problem.message).toContain("Cloud dictation has been retired");
      expect(problem.remedy).toBe("settings");
    }
  }
});
test("on-device recording problems still offer a retry", () => {
  for (const fault of ["invalid_audio", "unsupported_audio", "audio_too_long"] as const) {
    expect(voiceProblem(fault, "on_device").remedy).toBe("retry");
  }
});
