import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { emitNativeEvent } from "$shared/testing/native-events";
import { revision, tabFixture } from "$shared/testing/fixtures";
import { tabs } from "$domain/tabs";
import { surface } from "$domain/surface";
import { uiCommands } from "$domain/ui-commands";
import * as sidebar from "$session/sidebar-mode.svelte";
import type { TabView } from "$shared/ipc/bindings";
import AddressField from "../components/AddressField.svelte";

const native = vi.hoisted(() => ({
  edit: vi.fn(async (_id: string, _url: string) => false),
  run: vi.fn(async (_id: string) => ({ accepted: true, operation_id: null })),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    tabsBootstrap: async () => {},
    launcherEdit: native.edit,
    runCommand: native.run,
    sidebarSetWidth: async () => {},
    settingSet: async () => ({ accepted: true, operation_id: null }),
  });
});

afterEach(() => {
  sidebar.setAutoHide(false);
  tabs.dispose();
  surface.dispose();
  uiCommands.dispose();
  vi.clearAllMocks();
});

let projectionRevision = Date.now();
const nextRevision = () => revision(++projectionRevision);

async function field(overrides: Partial<TabView> = {}, compact = false) {
  await tabs.init();
  await uiCommands.init();
  emitNativeEvent("uiCommand", "test.settled");
  const currentRevision = nextRevision();
  const tab = tabFixture({
    url: "https://www.wikipedia.org/wiki/Premium",
    ...overrides,
    projection_revision: currentRevision,
  });
  emitNativeEvent("itemsChanged", {
    projection_revision: currentRevision,
    profile: null,
    spaces: [],
    active_space_id: null,
    nodes: [],
    tabs: [tab],
    active: tab.id,
    split_group: null,
  });
  const screen = await render(AddressField, { compact });
  const input = screen.container.querySelector<HTMLInputElement>("[data-zephium-address]")!;
  return { screen, input };
}

test("editing shows the whole address and rests on the host", async () => {
  const { input } = await field();
  expect(input.value).toBe("www.wikipedia.org");
  input.focus();
  await expect.poll(() => input.value).toBe("https://www.wikipedia.org/wiki/Premium");
  input.blur();
  await expect.poll(() => input.value).toBe("www.wikipedia.org");
});

test("a press anywhere else in the chrome ends editing", async () => {
  const { input } = await field();
  const elsewhere = document.createElement("div");
  elsewhere.style.cssText = "width:40px;height:40px";
  document.body.append(elsewhere);

  input.focus();
  expect(document.activeElement).toBe(input);
  elsewhere.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  expect(document.activeElement).not.toBe(input);
  await expect.poll(() => input.value).toBe("www.wikipedia.org");
  elsewhere.remove();
});

test("the page taking focus from the chrome ends editing", async () => {
  const { input } = await field();
  input.focus();
  window.dispatchEvent(new Event("blur"));
  expect(document.activeElement).not.toBe(input);
});

for (const shape of ["compact", "hidden"] as const) {
  for (const tab of [
    { name: "blank tab", url: null },
    { name: "extension tab", url: null, content: "extension_owned" },
  ] as const) {
    test(`${shape} ${tab.name} opens an empty launcher for the address shortcut`, async () => {
      const { input } = await field(tab, shape === "compact");
      if (shape === "hidden") sidebar.setAutoHide(true);
      const resting = input.value;
      expect(tabs.activeTab()?.url).toBe(tab.url);
      emitNativeEvent("uiCommand", "url.focus");
      expect(uiCommands.uiCommand().id).toBe("url.focus");
      await expect.poll(() => native.run.mock.calls).toEqual([["launcher.toggle"]]);
      expect(native.edit).not.toHaveBeenCalled();
      expect(document.activeElement).not.toBe(input);
      expect(input.value).toBe(resting);
      expect(input.isConnected).toBe(true);
    });
  }

  test(`${shape} web tab edits its full URL in the launcher`, async () => {
    const { input } = await field({}, shape === "compact");
    if (shape === "hidden") sidebar.setAutoHide(true);
    emitNativeEvent("uiCommand", "url.focus");
    await expect
      .poll(() => native.edit.mock.calls)
      .toEqual([[tabs.activeId(), "https://www.wikipedia.org/wiki/Premium"]]);
    expect(native.run).not.toHaveBeenCalled();
    expect(input.value).toBe("www.wikipedia.org");
  });
}

test("Work opens an empty launcher instead of editing the background tab", async () => {
  const { input } = await field({}, true);
  await surface.init();
  emitNativeEvent("uiCommand", "browser.work");
  emitNativeEvent("uiCommand", "url.focus");
  await expect.poll(() => native.run.mock.calls).toEqual([["launcher.toggle"]]);
  expect(native.edit).not.toHaveBeenCalled();
  expect(input.value).toBe("www.wikipedia.org");
});

test("an expanded column focuses the address input for the address shortcut", async () => {
  const { input } = await field();
  emitNativeEvent("uiCommand", "url.focus");
  await expect.poll(() => document.activeElement).toBe(input);
  expect(input.value).toBe("https://www.wikipedia.org/wiki/Premium");
  expect(native.edit).not.toHaveBeenCalled();
  expect(native.run).not.toHaveBeenCalled();
});

test("a native blocked-popup projection is visible without changing the address", async () => {
  const { screen, input } = await field();
  const tab = tabs.activeTab()!;
  emitNativeEvent("tabChanged", {
    ...tab,
    projection_revision: nextRevision(),
    page_request: { kind: "popup", host: "example.com" },
  });
  const card = screen.getByRole("alertdialog");
  await expect.element(card).toHaveTextContent("Pop-up blocked");
  await expect.element(card).toHaveTextContent("example.com");
  await expect.element(card.getByRole("button", { name: "Open" })).toBeVisible();
  expect(input.value).toBe("www.wikipedia.org");
});

test("a page's link for another app asks before it opens", async () => {
  const { screen } = await field();
  const tab = tabs.activeTab()!;
  emitNativeEvent("tabChanged", {
    ...tab,
    projection_revision: nextRevision(),
    page_request: { kind: "external_app", site: "zoom.us", scheme: "zoommtg", app: "zoom.us" },
  });
  const card = screen.getByRole("alertdialog");
  await expect.element(card).toHaveTextContent("Open zoom.us?");
  await expect.element(card.getByRole("checkbox")).not.toBeChecked();
  await expect.element(card.getByRole("button", { name: "Cancel" })).toBeVisible();
});
