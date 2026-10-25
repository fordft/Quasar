import "$styles/global.css";
import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import WorkDecisions from "../components/WorkDecisions.svelte";
const native = vi.hoisted(() => ({ read: vi.fn(), set: vi.fn() }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    workDecisionPreference: native.read,
    workSetDecisionPreference: native.set,
  });
});
const profile = "00000000000000000000000001";
const preference = (choice: "standard" | "off") => ({
  version: 1,
  profile,
  choice,
  effective: choice,
  typesafe_key_present: false,
  error: null,
});
test("decision preference uses ChatGPT and has no separate credential flow", async () => {
  await page.viewport(1000, 700);
  native.read.mockResolvedValue(preference("standard"));
  native.set.mockResolvedValue(preference("off"));
  const screen = await render(WorkDecisions, { profile });
  await screen.getByRole("radio", { name: "Off", exact: true }).click();
  expect(native.set).toHaveBeenCalledWith(profile, "off");
  await expect
    .element(screen.getByText("Typed decisions use the connected ChatGPT account."))
    .toBeVisible();
  await expect.element(screen.getByRole("textbox")).not.toBeInTheDocument();
});
