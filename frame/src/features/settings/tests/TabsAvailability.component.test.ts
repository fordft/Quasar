import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import TabsPage from "../components/sections/TabsPage.svelte";
import "$styles/global.css";

vi.mock("$shared/platform", () => ({ IS_MAC: false, IS_WINDOWS: false, IS_LINUX: true }));
vi.mock("$domain/preferences", () => ({
  preferences: {
    value: () => "true",
    saving: () => false,
    set: vi.fn(),
  },
}));

test("Linux offers tab settings without advertising unavailable automatic PiP", async () => {
  const screen = await render(TabsPage);
  expect(screen.container.querySelector('[data-setting="tabs.duplicates"]')).not.toBeNull();
  expect(screen.container.querySelector('[data-setting="tabs.automatic-pip"]')).toBeNull();
  expect(screen.container.querySelector('[data-setting="tabs.sidebar-autohide"]')).toBeNull();
});
