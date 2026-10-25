import "$styles/global.css";
import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import { PROFILE, models } from "$shared/testing/work-models";
import { accountFixture } from "$shared/testing/account";
import AiPage from "../components/sections/AiPage.svelte";
vi.mock("$domain/tabs", () => ({
  tabs: { profile: () => ({ id: "00000000000000000000000001", kind: "regular" }) },
}));
const native = vi.hoisted(() => ({
  read: vi.fn(),
  choose: vi.fn(),
  account: vi.fn(),
  more: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    workMoreModels: native.more,
    workModels: native.read,
    workChooseModel: native.choose,
    accountGet: native.account,
    voiceStatus: async () => ({
      version: 1,
      engines: ["on_device"],
      chosen: null,
      effective: "on_device",
      language: null,
      languages: ["en-US"],
      on_device: "en_US",
      microphone: "granted",
    }),
    workDecisionPreference: async () => ({
      version: 1,
      profile: PROFILE,
      choice: "standard",
      effective: "standard",
      typesafe_key_present: false,
      error: null,
    }),
  });
});
test("AI settings offers only ChatGPT and account-authorized models", async () => {
  await page.viewport(1100, 1200);
  native.account.mockResolvedValue({
    account: accountFixture({ signed_in: true, sharing: true, email: "ada@example.com" }),
    fault: null,
  });
  const view = models({ keys: { open_ai: "valid", anthropic: "valid" }, lead: "openai/gpt-6-sol" });
  native.read.mockResolvedValue(view);
  native.choose.mockResolvedValue(view);
  const screen = await render(AiPage);
  await expect.element(screen.getByText("Using ChatGPT plan", { exact: true })).toBeVisible();
  await expect.element(screen.getByRole("textbox")).not.toBeInTheDocument();
  await expect.element(screen.getByText("API keys", { exact: true })).not.toBeInTheDocument();
  await screen.getByLabelText("Work", { exact: true }).click();
  await expect.element(page.getByRole("option", { name: "GPT-6 Sol", exact: true })).toBeVisible();
  await expect.element(page.getByRole("option", { name: /Claude/u })).not.toBeInTheDocument();
  await page.getByRole("option", { name: "GPT-6 Sol", exact: true }).click();
  expect(native.choose).toHaveBeenCalledWith(PROFILE, "lead", "openai/gpt-6-sol");
});

test("model discovery can be retried without signing in again or entering a key", async () => {
  native.account.mockResolvedValue({
    account: accountFixture({ signed_in: true, sharing: true }),
    fault: null,
  });
  const connected = models({ keys: { open_ai: "valid" } });
  const failed = {
    ...connected,
    entries: [],
    providers: connected.providers.map((p) =>
      p.provider === "open_ai"
        ? { ...p, key: "missing" as const, fault: "unreachable" as const }
        : p,
    ),
  };
  native.read.mockResolvedValueOnce(failed).mockResolvedValue(connected);
  native.more.mockResolvedValue({ provider: "open_ai", entries: connected.entries, fault: null });
  const screen = await render(AiPage);
  await expect
    .element(screen.getByRole("alert"))
    .toHaveTextContent("Could not load ChatGPT models");
  await screen.getByRole("button", { name: "Retry", exact: true }).click();
  expect(native.more).toHaveBeenCalledWith(PROFILE, "open_ai");
  await expect.element(screen.getByLabelText("Work", { exact: true })).not.toBeDisabled();
  await expect.element(screen.getByRole("alert")).not.toBeInTheDocument();
});
