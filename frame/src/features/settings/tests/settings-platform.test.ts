import { expect, test, vi } from "vitest";
import { searchSettings, searchSections } from "../lib/settings-model";

const platform = vi.hoisted(() => ({ mac: false, windows: false, linux: true }));
vi.mock("$shared/platform", () => ({
  get IS_MAC() {
    return platform.mac;
  },
  get IS_WINDOWS() {
    return platform.windows;
  },
  get IS_LINUX() {
    return platform.linux;
  },
}));

test.each([
  ["Linux", false, false, true, false],
  ["macOS", true, false, false, true],
  ["Windows", false, true, false, true],
] as const)(
  "%s offers automatic PiP only when its backend supports it",
  async (_, mac, windows, linux, supported) => {
    Object.assign(platform, { mac, windows, linux });
    expect(searchSettings("picture").some((field) => field.target === "tabs.automatic-pip")).toBe(
      supported,
    );
    expect(searchSections("picture").some((section) => section.id === "tabs")).toBe(supported);
  },
);
