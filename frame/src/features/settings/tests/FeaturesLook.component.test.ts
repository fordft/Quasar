import "$styles/global.css";
import { afterEach, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import { emitNativeEvent } from "$shared/testing/native-events";
import { accountFixture } from "$shared/testing/account";
import { preferences } from "$domain/preferences";
import * as settings from "../lib/settings-state.svelte";
import type { SettingsSection } from "../lib/settings-model";
import SettingsLookStage from "./SettingsLookStage.svelte";

const native = vi.hoisted(() => ({ settings: {} as Record<string, string> }));
// This bench exercises the complete feature set, independent of the test host.
vi.mock("$shared/platform", () => ({ IS_MAC: true, IS_WINDOWS: false, IS_LINUX: false }));
vi.mock("$domain/tabs", () => ({
  tabs: { profile: () => ({ id: "00000000000000000000000001", kind: "regular" }) },
}));
// A look bench: what the shots depend on answers for real, the rest nothing.
vi.mock("$shared/ipc/bindings", async () => {
  const { entry, models } = await import("$shared/testing/work-models");
  const known: Record<string, (...args: never[]) => Promise<unknown>> = {
    settingGet: async (key: string) => native.settings[key] ?? null,
    accountGet: async () => ({
      account: accountFixture({ signed_in: true, email: "ada@example.com", plan: "pro" }),
      fault: null,
      retry_after: null,
    }),
    keymapEntries: async () => [],
    workLocalServers: async () => [],
    workMoreModels: async () => ({
      entries: [entry("compatible", "qwen3:32b", "qwen3:32b", false)],
      fault: null,
    }),
    voiceStatus: async () => ({
      version: 1,
      engines: ["on_device", "open_ai"],
      chosen: null,
      effective: "on_device",
      language: null,
      languages: ["en-US"],
      on_device: "en_US",
      microphone: "granted",
    }),
    workDecisionPreference: async () => ({
      version: 1,
      profile: "00000000000000000000000001",
      choice: "recommended",
      effective: "standard",
      typesafe_key_present: false,
      error: null,
    }),
    workModels: async () =>
      models({
        keys: {
          anthropic: "valid",
          open_ai: "valid",
          google: "set",
          deep_seek: "valid",
          open_router: "set",
          xai: "set",
          mistral: "invalid",
          groq: "set",
          cerebras: "set",
        },
        base: "http://localhost:11434/v1",
        lead: "anthropic/claude-opus-5-5",
      }),
  };
  return {
    commands: new Proxy(known, {
      get: (target, key) =>
        typeof key === "string" ? (target[key] ?? (async () => null)) : undefined,
    }),
  };
});

afterEach(() => {
  preferences.dispose();
  native.settings = {};
});

const shots = "../../../../../target/look/settings";
const settle = (ms = 450) => new Promise((done) => setTimeout(done, ms));

async function shoot(name: string, section: SettingsSection) {
  settings.select(section);
  await settle();
  for (const theme of ["dark", "light"]) {
    document.documentElement.dataset.theme = theme;
    await settle(250);
    await page.screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
  document.documentElement.dataset.theme = "dark";
}

test("Features, and the Work group as AI and Work go off, both themes", async () => {
  await page.viewport(1280, 1100);
  await preferences.init();
  await render(SettingsLookStage);
  await settle();
  await shoot("features", "features");
  await shoot("work-on", "work");
  await shoot("models", "ai");

  settings.select("features");
  await page.getByRole("switch", { name: "AI" }).click();
  await settle();
  for (const theme of ["dark", "light"]) {
    document.documentElement.dataset.theme = theme;
    await settle(250);
    await page.screenshot({ path: `${shots}/ai-confirm-${theme}.png` });
  }
  await page.getByRole("button", { name: "Cancel" }).click();

  emitNativeEvent("uiCommand", "preference.ai.enabled=false");
  emitNativeEvent("uiCommand", "preference.notes.enabled=false");
  await shoot("features-off", "features");
  await shoot("work-ai-off", "work");
});
