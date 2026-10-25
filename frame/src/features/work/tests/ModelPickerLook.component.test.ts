import "$styles/global.css";
import { afterEach, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import type {
  WorkModelProvider,
  WorkModelReasoning,
  WorkModelRole,
  WorkModelsV1,
} from "$shared/ipc/bindings";
import { preferences } from "$domain/preferences";
import { PROFILE, models } from "$shared/testing/work-models";
import ModelPickerStage from "./ModelPickerStage.svelte";

const native = vi.hoisted(() => ({
  view: null as unknown as WorkModelsV1,
  ready: true,
  hosted: false,
  settings: {} as Record<string, string>,
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  const { accountFixture } = await import("$shared/testing/account");
  return mockBindings({
    workModels: vi.fn(async () => native.view),
    workModelsReady: vi.fn(async (profile: string) => ({
      version: 1,
      profile,
      ready: native.ready,
      missing_roles: (native.ready ? [] : ["lead"]) as WorkModelRole[],
      fault: null,
    })),
    workSetModelEffort: vi.fn(async (_profile: string, effort: WorkModelReasoning | null) => {
      native.view = { ...native.view, effort };
      return native.view;
    }),
    workMoreModels: vi.fn(async (_profile: string, provider: WorkModelProvider) => ({
      provider,
      entries: [],
      fault: null,
    })),
    settingGet: async (key: string) => native.settings[key] ?? null,
    settingSet: async (key: string, value: string) => {
      native.settings[key] = value;
      return { accepted: true, operation_id: null };
    },
    accountGet: vi.fn(async () => ({
      account: native.hosted
        ? accountFixture({ available: true, signed_in: false })
        : accountFixture({ signed_in: true, sharing: true, email: "ada@example.com" }),
      fault: null,
      retry_after: null,
    })),
  });
});

const shots = "../../../../../target/look/model-picker";
const settle = (ms = 450) => new Promise((done) => setTimeout(done, ms));
const keyed = (lead: string) => {
  const view = models({ keys: { open_ai: "valid" }, lead });
  return {
    ...view,
    entries: view.entries
      .filter((entry) => entry.model.provider === "open_ai")
      .map((entry) => ({ ...entry, price: null })),
  };
};

/** Glass is the menus' own material; solid is the opaque policy a Windows sidebar asks for. */
async function shoot(name: string, materials: readonly ("glass" | "solid")[] = ["glass"]) {
  for (const theme of ["dark", "light"]) {
    for (const material of materials) {
      document.documentElement.dataset.theme = theme;
      const menu = document.querySelector(".model-menu");
      if (material === "solid") menu?.setAttribute("data-menu-material", "opaque");
      else menu?.removeAttribute("data-menu-material");
      await settle();
      const suffix = materials.length > 1 ? `-${material}` : "";
      await page.screenshot({ path: `${shots}/${name}-${theme}${suffix}.png` });
    }
  }
  document.documentElement.dataset.theme = "dark";
  document.querySelector(".model-menu")?.removeAttribute("data-menu-material");
}

afterEach(() => {
  native.ready = true;
  native.hosted = false;
  native.settings = {};
  preferences.dispose();
});

test("ChatGPT model list, favourites and search in both themes", { timeout: 60000 }, async () => {
  await page.viewport(1100, 700);
  native.view = keyed("openai/gpt-6-luna");
  native.settings["work.favourite-models"] = "openai/gpt-6-sol";
  await preferences.init();
  const screen = await render(ModelPickerStage, { profile: PROFILE });
  await shoot("chatgpt-rest");
  await screen.getByRole("button", { name: /Model: GPT-6 Luna/u }).click();
  await shoot("chatgpt-card", ["glass", "solid"]);
  await page.getByRole("button", { name: "Favourites", exact: true }).click();
  await shoot("chatgpt-favourites");
  await page.getByRole("button", { name: "Favourites", exact: true }).click();
  await userEvent.keyboard("astra");
  await shoot("chatgpt-search");
});

test("the effort at every level, from the slider's keys", async () => {
  await page.viewport(1100, 700);
  native.view = { ...keyed("openai/gpt-6-luna"), effort: "low" };
  const screen = await render(ModelPickerStage, { profile: PROFILE });
  await screen.getByRole("button", { name: /Model: GPT-6 Luna/u }).click();
  await page.getByRole("button", { name: /^Effort: Low/u }).click();
  const levels = ["low", "medium", "high", "xhigh", "max"];
  for (const [index, level] of levels.entries()) {
    if (index) await userEvent.keyboard("{ArrowRight}");
    await shoot(`effort-${level}`, level === "max" ? ["glass", "solid"] : ["glass"]);
  }
  await settle(1200);
  await shoot("rest-max");
});

test("ChatGPT connection required", async () => {
  await page.viewport(1100, 700);
  native.view = models({});
  native.ready = false;
  const keyless = await render(ModelPickerStage, { profile: PROFILE });
  await shoot("keyless");
  await keyless.unmount();
});
