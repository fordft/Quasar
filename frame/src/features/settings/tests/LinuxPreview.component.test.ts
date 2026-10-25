import "$styles/global.css";
import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { preferences } from "$domain/preferences";
import FeatureSwitch from "../components/FeatureSwitch.svelte";

vi.mock("$shared/platform", () => ({ IS_MAC: false, IS_WINDOWS: false, IS_LINUX: true }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    settingGet: async (key) => (key === "ai.enabled" || key === "work.enabled" ? "false" : null),
  });
});

afterEach(() => preferences.dispose());

for (const feature of ["ai", "work"] as const) {
  test(`Linux preview explains why ${feature} is unavailable`, async () => {
    await preferences.init();
    const screen = await render(FeatureSwitch, { feature });
    await expect.element(screen.getByRole("switch")).toBeDisabled();
    await expect.element(screen.getByRole("switch")).not.toBeChecked();
    await expect
      .element(
        screen.getByText("This Linux preview is for browsing. AI and Work are not available yet."),
      )
      .toBeVisible();
  });
}
