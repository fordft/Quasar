import { afterEach, expect, test, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import { flushSync } from "svelte";
import "$styles/global.css";
import type { SidebarNodeView } from "$shared/ipc/bindings";
import { emitNativeEvent } from "$shared/testing/native-events";
import { revision, tabFixture } from "$shared/testing/fixtures";
import { surface } from "$domain/surface";
import { tabs } from "$domain/tabs";
import * as launch from "$session/motion.svelte";
import * as sidebar from "$session/sidebar-mode.svelte";
import * as tabDrag from "$session/tab-drag.svelte";
import Shell from "../Shell.svelte";

// Hiding the sidebar is macOS's; the page is asked as if it were one.
vi.mock("$shared/platform", () => ({ IS_MAC: true, IS_WINDOWS: false, IS_LINUX: false }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    tabsBootstrap: async () => {},
    sidebarSetWidth: async () => {},
    sidebarResize: async () => true,
    sidebarResizeGuide: async () => true,
    tabDragOver: async () => {},
    settingSet: async () => ({ accepted: true, operation_id: null }),
  });
});
afterEach(() => {
  tabDrag.end();
  sidebar.setAutoHide(false);
  sidebar.adoptMode("default");
  surface.dispose();
  tabs.dispose();
});
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function mount() {
  await page.viewport(268, 700);
  launch.dispose();
  await surface.init();
  await tabs.init();
  const { uiCommands } = await import("$domain/ui-commands");
  await uiCommands.init();
  // The store keeps the last command; a new column would act on it again.
  emitNativeEvent("uiCommand", "test.settled");
  const all = ["a", "b"].map((id) => tabFixture({ id, title: `Tab ${id}` }));
  emitNativeEvent("itemsChanged", {
    projection_revision: revision(Date.now()),
    profile: null,
    spaces: [],
    active_space_id: null,
    nodes: all.map((tab) => ({
      id: tab.id,
      parent_id: null,
      section: "today",
      kind: { type: "tab", tab_id: tab.id },
    })) as SidebarNodeView[],
    tabs: all,
    active: "a",
    split_group: null,
  });
  flushSync();
  render(Shell);
  await wait(400);
  // The test browser's own pointer rests in the window's corner, on the edge.
  sidebar.pointerLeft();
  return {
    aside: () => document.querySelector<HTMLElement>("aside.browser-sidebar")!,
    edge: () => document.querySelector<HTMLElement>(".sidebar-edge")!,
  };
}

const enter = (target: Element) => target.dispatchEvent(new PointerEvent("pointerenter"));
const leave = (target: Element) => target.dispatchEvent(new PointerEvent("pointerleave"));

test("a hidden sidebar is a width of nothing that takes no focus, and the edge brings it back", async () => {
  const { aside, edge } = await mount();
  expect(aside().dataset.hidden).toBe("false");
  expect(aside().inert).toBe(false);
  expect(aside().getBoundingClientRect().width).toBeGreaterThan(200);

  sidebar.setAutoHide(true);
  await wait(500);
  expect(aside().dataset.hidden).toBe("true");
  expect(aside().inert).toBe(true);
  expect(aside().getBoundingClientRect().width).toBe(0);
  expect(getComputedStyle(aside()).visibility).toBe("hidden");

  // The edge waits in the gutter: resting on it brings the sidebar out.
  enter(edge());
  await wait(sidebar.REVEAL_DELAY - 60);
  expect(aside().dataset.hidden).toBe("true");
  await wait(120);
  expect(aside().dataset.hidden).toBe("false");
  expect(aside().inert).toBe(false);
  await wait(500);
  expect(aside().getBoundingClientRect().width).toBeGreaterThan(200);
  expect(getComputedStyle(aside()).visibility).toBe("visible");

  // And it leaves a moment after the pointer does.
  enter(aside());
  leave(aside());
  await wait(sidebar.HIDE_DELAY - 100);
  expect(aside().dataset.hidden).toBe("false");
  await wait(250);
  expect(aside().dataset.hidden).toBe("true");
});

test("crossing the edge without resting on it brings out nothing", async () => {
  const { aside, edge } = await mount();
  sidebar.setAutoHide(true);
  await wait(500);
  enter(edge());
  await wait(40);
  leave(edge());
  await wait(sidebar.REVEAL_DELAY * 3);
  expect(aside().dataset.hidden).toBe("true");
});

test("a tab being carried keeps the sidebar out until it is dropped", async () => {
  const { aside, edge } = await mount();
  sidebar.setAutoHide(true);
  await wait(500);
  enter(edge());
  await wait(sidebar.REVEAL_DELAY + 40);
  expect(aside().dataset.hidden).toBe("false");

  tabDrag.begin("b");
  flushSync();
  leave(aside());
  await wait(sidebar.HIDE_DELAY * 2);
  expect(aside().dataset.hidden).toBe("false");

  tabDrag.end();
  flushSync();
  await wait(sidebar.HIDE_DELAY + 150);
  expect(aside().dataset.hidden).toBe("true");
});

test("the command toggles it, and the sidebar does not reveal itself under the pointer", async () => {
  const { aside } = await mount();
  expect(sidebar.autoHides()).toBe(false);
  emitNativeEvent("uiCommand", "sidebar.toggleAutoHide");
  await wait(500);
  expect(sidebar.autoHides()).toBe(true);
  expect(aside().dataset.hidden).toBe("true");

  emitNativeEvent("uiCommand", "sidebar.toggleAutoHide");
  await wait(500);
  expect(sidebar.autoHides()).toBe(false);
  expect(aside().dataset.hidden).toBe("false");
});

test("a place that lives in the column shows it, and hiding resumes after", async () => {
  const { aside } = await mount();
  sidebar.setAutoHide(true);
  await wait(500);
  expect(aside().dataset.hidden).toBe("true");
  // History opens as a panel beside the rail: the column has work to do.
  sidebar.setAutoHideApplies(false);
  await wait(500);
  expect(aside().dataset.hidden).toBe("false");
  // Nobody is on the column, so it goes back.
  sidebar.pointerLeft();
  sidebar.setAutoHideApplies(true);
  await wait(500);
  expect(aside().dataset.hidden).toBe("true");
});
