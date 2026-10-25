import { describe, expect, test } from "vitest";
import type { WorkModelEntry } from "$shared/ipc/bindings";
import {
  contextSize,
  dollars,
  effortLevels,
  pickerGroups,
  priceTier,
  searchEntries,
  shortName,
  usable,
} from "../catalog";
import { entry, models } from "$shared/testing/work-models";

describe("the picker's list", () => {
  test("only ChatGPT account models can become selectable, including with stale provider state", () => {
    const view = models({
      keys: { anthropic: "valid", open_ai: "valid", google: "valid" },
      cloud: true,
      base: "http://localhost:11434/v1",
    });
    const served = [entry("compatible", "qwen4:32b", "Local model", false)];
    const { ready, needsKey } = pickerGroups(view, "lead", served);
    expect(ready.map((group) => group.provider)).toEqual(["open_ai"]);
    expect(ready[0]!.entries.map((item) => item.id)).toEqual([
      "openai/gpt-6-luna",
      "openai/gpt-6-sol",
      "openai/gpt-6-astra",
    ]);
    expect(needsKey).toEqual([]);
    expect(usable(view, "cloud")).toBe(false);
    expect(usable(view, "compatible")).toBe(false);
    expect(usable(view, "anthropic")).toBe(false);
  });
  test("an unconfirmed legacy OpenAI key cannot authorize models", () => {
    const view = models({ keys: { open_ai: "set" } });
    expect(pickerGroups(view, "lead").ready).toEqual([]);
    expect(usable(null, "open_ai")).toBe(false);
  });

  test("the trigger drops a family name the mark already says", () => {
    const opus: WorkModelEntry = entry("anthropic", "claude-opus-5-5", "Claude Opus 5.5", true);
    expect(shortName(opus)).toBe("Opus 5.5");
    expect(shortName(entry("open_ai", "gpt-6-sol", "GPT-6 Sol", true))).toBe("GPT-6 Sol");
  });
});

describe("the picker's details", () => {
  test("context windows and prices read the way a model card says them", () => {
    expect(contextSize(1_000_000)).toBe("1M");
    expect(contextSize(1_048_576)).toBe("1M");
    expect(contextSize(1_050_000)).toBe("1M");
    expect(contextSize(200_000)).toBe("200K");
    expect(contextSize(131_072)).toBe("128K");
    expect(dollars(2_000_000)).toBe("$2");
    expect(dollars(1_320_000)).toBe("$1.32");
    expect(dollars(100_000)).toBe("$0.1");
    expect(dollars(10_000)).toBe("$0.01");
    expect(priceTier({ input: 100_000, cached_input: 10_000, output: 500_000 })).toBe(1);
    expect(priceTier({ input: 2_000_000, cached_input: 200_000, output: 10_000_000 })).toBe(2);
    expect(priceTier({ input: 4_000_000, cached_input: 200_000, output: 20_000_000 })).toBe(3);
    expect(priceTier({ input: 10_000_000, cached_input: 1_000_000, output: 50_000_000 })).toBe(4);
    expect(priceTier(null)).toBeNull();
  });

  test("search finds models across providers by name, maker or id", () => {
    const view = models({ keys: { anthropic: "valid", open_ai: "valid" } });
    const { ready } = pickerGroups(view, "lead");
    expect(searchEntries(ready, "sonnet")).toEqual([]);
    expect(searchEntries(ready, "openai astra").map((item) => item.id)).toEqual([
      "openai/gpt-6-astra",
    ]);
    expect(searchEntries(ready, "  ")).toEqual([]);
  });

  test("each entry offers the efforts Rust says it takes", () => {
    const view = models({ keys: { deep_seek: "valid" }, lead: "deepseek/deepseek-v4-pro" });
    expect(effortLevels(view, "deepseek/deepseek-v4-pro")).toEqual(["low", "high"]);
    expect(effortLevels(view, "nobody/none")).toEqual([]);
    expect(view.effort).toBe("high");
  });
});
