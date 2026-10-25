import "$styles/global.css";
import { beforeEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import type { WorkModelReasoning, WorkModelsReadyV1 } from "$shared/ipc/bindings";
import ModelPicker from "../components/bar/ModelPicker.svelte";
import ModelBarStage from "./ModelBarStage.svelte";
import { PROFILE, models } from "$shared/testing/work-models";
import { accountFixture } from "$shared/testing/account";

const native = vi.hoisted(() => ({
  read: vi.fn(),
  choose: vi.fn(),
  effort: vi.fn(),
  more: vi.fn(),
  account: vi.fn(),
  command: vi.fn(async () => ({ operation_id: "op", accepted: true })),
  settings: {} as Record<string, string>,
  ready: vi.fn(async (profile: string): Promise<WorkModelsReadyV1> => ({
    version: 1,
    profile,
    ready: true,
    missing_roles: [],
    fault: null,
  })),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    workModels: native.read,
    workChooseModel: native.choose,
    workSetModelEffort: native.effort,
    workMoreModels: native.more,
    workModelsReady: native.ready,
    accountGet: native.account,
    runCommand: native.command,
    settingGet: async (key: string) => native.settings[key] ?? null,
    settingSet: async (key: string, value: string) => {
      native.settings[key] = value;
      return { accepted: true, operation_id: null };
    },
  });
});

const view = (
  lead = "openai/gpt-6-luna",
  chosen: string | null = null,
  effort?: WorkModelReasoning,
) => {
  const value = models({ keys: { open_ai: "valid" }, lead, chosen, effort });
  return {
    ...value,
    entries: value.entries
      .filter((entry) => entry.model.provider === "open_ai")
      .map((entry) => ({ ...entry, price: null })),
  };
};
const ready = (value: boolean) => async (profile: string) => ({
  version: 1,
  profile,
  ready: value,
  missing_roles: value ? [] : (["lead"] as const).slice(),
  fault: null,
});
beforeEach(() => {
  native.ready.mockImplementation(ready(true));
  native.account.mockResolvedValue({
    account: accountFixture({ signed_in: true, sharing: true, email: "ada@example.com" }),
    fault: null,
  });
});

test("the picker lists only ChatGPT models and sends the native model choice", async () => {
  await page.viewport(1200, 800);
  native.read.mockResolvedValue(view());
  native.choose.mockResolvedValue(view("openai/gpt-6-sol", "openai/gpt-6-sol"));
  const screen = await render(ModelPicker, { profile: PROFILE });
  await screen.getByRole("button", { name: /Model: GPT-6 Luna/u }).click();
  await expect.element(page.getByRole("tab", { name: "ChatGPT", exact: true })).toBeVisible();
  await expect.element(page.getByRole("tab", { name: "Anthropic" })).not.toBeInTheDocument();
  await page.getByRole("option", { name: /GPT-6 Sol/u }).click();
  expect(native.choose).toHaveBeenCalledWith(PROFILE, "lead", "openai/gpt-6-sol");
  await expect.element(screen.getByRole("button", { name: /Model: GPT-6 Sol/u })).toBeVisible();
});

test("typing searches the account catalog and Enter selects its first match", async () => {
  native.read.mockResolvedValue(view());
  native.choose.mockResolvedValue(view("openai/gpt-6-astra", "openai/gpt-6-astra"));
  const screen = await render(ModelPicker, { profile: PROFILE });
  await screen.getByRole("button", { name: /Model: GPT-6 Luna/u }).click();
  await userEvent.keyboard("astra");
  await expect.element(page.getByRole("combobox", { name: "Search models" })).toHaveValue("astra");
  await expect.poll(() => page.getByRole("option").all().length).toBe(1);
  await userEvent.keyboard("{Enter}");
  expect(native.choose).toHaveBeenCalledWith(PROFILE, "lead", "openai/gpt-6-astra");
});

test("without a ChatGPT connection the control opens connection settings", async () => {
  native.ready.mockImplementation(ready(false));
  native.read.mockResolvedValue(models({}));
  native.account.mockResolvedValue({ account: accountFixture(), fault: null });
  const onsettings = vi.fn();
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings });
  await screen.getByRole("button", { name: "Continue with ChatGPT" }).click();
  expect(onsettings).toHaveBeenCalledOnce();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
});

test("the clear button empties the search and returns to the provider's list", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("openai/gpt-6-luna"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: GPT-6 Luna/u }).click();
  await userEvent.keyboard("zzz");
  await expect.element(page.getByText("No model matches")).toBeVisible();
  await page.getByRole("button", { name: "Clear search" }).click();
  await expect.element(page.getByRole("combobox", { name: "Search models" })).toHaveValue("");
  await expect.element(page.getByRole("option", { name: /GPT-6 Sol/u })).toBeVisible();
});

test("the effort shows in a word, and the card turns into its control to set it", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("openai/gpt-6-luna"));
  native.effort.mockImplementation(async (_profile: string, effort: WorkModelReasoning) =>
    view("openai/gpt-6-luna", null, effort),
  );
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: GPT-6 Luna, Medium effort/u }).click();

  // At rest the card is the model, with its effort in a word; the control is not built.
  const pill = page.getByRole("button", { name: "Effort: Medium" });
  await expect.element(pill).toHaveAttribute("aria-expanded", "false");
  expect(page.getByRole("slider").query()).toBeNull();
  expect(document.querySelector(".model-menu .core")).toBeNull();

  await pill.click();
  const effort = page.getByRole("slider", { name: "Effort" });
  await expect.element(effort).toHaveFocus();
  await expect.element(effort).toHaveAttribute("aria-valuemax", "4");
  await expect.element(effort).toHaveAttribute("aria-valuetext", "Medium");
  // The model is never shown beside it.
  expect(document.querySelector(".model-menu .face.model")?.hasAttribute("inert")).toBe(true);
  const pace = () =>
    document
      .querySelector(".model-menu .core")!
      .getAnimations({ subtree: true })
      .map((run) => run.playbackRate)[0];
  await expect.poll(pace).toBeGreaterThan(0);
  const calm = pace()!;

  await userEvent.keyboard("{End}");
  await expect.element(effort).toHaveAttribute("aria-valuetext", "Max");
  expect(native.effort).toHaveBeenLastCalledWith(PROFILE, "max");
  await expect.poll(pace).toBeGreaterThan(calm);
  await expect
    .element(screen.getByRole("button", { name: /Model: GPT-6 Luna, Max effort/u }))
    .toBeInTheDocument();

  // Used, it turns back to the model and hands the keys to the button that opened it.
  await expect.element(page.getByRole("button", { name: "Effort: Max" })).toHaveFocus();
  expect(document.querySelector(".model-menu .face.effort")?.hasAttribute("inert")).toBe(true);
});

test("Escape turns the open effort back first, and only then closes the menu", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("openai/gpt-6-luna"));
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  const trigger = screen.getByRole("button", { name: /Model: GPT-6 Luna/u });
  await trigger.click();
  await page.getByRole("button", { name: "Effort: Medium" }).click();
  await expect.element(page.getByRole("slider", { name: "Effort" })).toBeVisible();
  await userEvent.keyboard("{Escape}");
  await expect
    .element(page.getByRole("button", { name: "Effort: Medium" }))
    .toHaveAttribute("aria-expanded", "false");
  await expect.element(page.getByRole("region", { name: "GPT-6 Luna" })).toBeVisible();
  await userEvent.keyboard("{Escape}");
  await expect.element(page.getByRole("region", { name: "GPT-6 Luna" })).not.toBeInTheDocument();
});

test("the effort control is set by a click and a drag, and turns back after a rest, a click elsewhere or Escape", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("openai/gpt-6-luna"));
  native.effort.mockImplementation(async (_profile: string, effort: WorkModelReasoning) => {
    await new Promise((done) => setTimeout(done, 30));
    return view("openai/gpt-6-luna", null, effort);
  });
  const screen = await render(ModelPicker, { profile: PROFILE, onsettings: vi.fn() });
  await screen.getByRole("button", { name: /Model: GPT-6 Luna/u }).click();
  const pill = () => page.getByRole("button", { name: /^Effort:/u });
  const slider = page.getByRole("slider", { name: "Effort" });
  const asked = native.effort.mock.calls.length;
  const open = async () => {
    await pill().click();
    await expect.element(slider).toBeVisible();
  };

  await open();
  await slider.click({ position: { x: 96, y: 18 } });
  await expect.element(slider).toHaveAttribute("aria-valuetext", "Max");
  await slider.click({ position: { x: 6, y: 18 } });
  await expect.element(slider).toHaveAttribute("aria-valuetext", "Low");
  expect(native.effort.mock.calls.slice(asked).map((call) => call[1])).toEqual(["max", "low"]);

  // Dragged from the thumb, it follows the pointer and lands on the stop it is let go at.
  const thumb = page.elementLocator(document.querySelector(".model-menu .thumb")!);
  await userEvent.dragAndDrop(thumb, slider, { targetPosition: { x: 52, y: 18 } });
  await expect.element(slider).toHaveAttribute("aria-valuetext", "High");
  expect(document.querySelector(".model-menu .thumb")).not.toBeNull();

  // Used, it turns back a moment after the last change.
  await expect
    .poll(() => pill().element().getAttribute("aria-expanded"), { timeout: 4000 })
    .toBe("false");

  // Left alone, it turns back after a rest.
  const opened = performance.now();
  await open();
  await new Promise((done) => setTimeout(done, 1500));
  await expect.element(pill()).toHaveAttribute("aria-expanded", "true");
  await expect
    .poll(() => pill().element().getAttribute("aria-expanded"), { timeout: 4000 })
    .toBe("false");
  expect(performance.now() - opened).toBeGreaterThan(2500);

  // A click elsewhere in the menu puts it away at once.
  await open();
  await page.getByRole("tab", { name: "ChatGPT" }).click();
  await expect.element(pill()).toHaveAttribute("aria-expanded", "false");
});

test("choosing a model leaves the composer open and the cursor in the words", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("openai/gpt-6-sol"));
  native.choose.mockResolvedValue(view("openai/gpt-6-sol", "openai/gpt-6-sol"));
  const screen = await render(ModelBarStage, { profile: PROFILE });
  const bar = () => screen.container.querySelector<HTMLElement>(".work-bar")!;
  const words = screen.getByRole("textbox");
  await words.click();
  await words.fill("Plan the trip");
  await screen.getByRole("button", { name: /Model: GPT-6 Sol/u }).click();
  await page.getByRole("tab", { name: "ChatGPT" }).click();
  await page.getByRole("option", { name: /GPT-6 Sol/u }).click();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
  await expect.element(words).toHaveFocus();
  expect(bar().dataset.engaged).toBe("true");
  expect(bar().dataset.mode).toBe("focus");
  await expect.element(words).toHaveValue("Plan the trip");
});

test("closing the menu from the chip leaves an empty composer engaged and focused", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("openai/gpt-6-sol"));
  const screen = await render(ModelBarStage, { profile: PROFILE });
  const bar = () => screen.container.querySelector<HTMLElement>(".work-bar")!;
  const words = screen.getByRole("textbox");
  await words.click();
  const chip = screen.getByRole("button", { name: /Model: GPT-6 Sol/u });
  await chip.click();
  await expect.element(page.getByRole("listbox")).toBeVisible();
  await chip.click();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
  await expect.element(words).toHaveFocus();
  await new Promise((done) => setTimeout(done, 100));
  expect(bar().dataset.mode).toBe("focus");
});

test("a press on the words while the menu is open closes it and puts the cursor there", async () => {
  await page.viewport(1200, 800);
  native.ready.mockImplementation(ready(true));
  native.read.mockResolvedValue(view("openai/gpt-6-sol"));
  const screen = await render(ModelBarStage, { profile: PROFILE });
  const words = screen.getByRole("textbox");
  await words.click();
  await screen.getByRole("button", { name: /Model: GPT-6 Sol/u }).click();
  await expect.element(page.getByRole("listbox")).toBeVisible();
  await words.click();
  await expect.element(page.getByRole("listbox")).not.toBeInTheDocument();
  await expect.element(words).toHaveFocus();
});
