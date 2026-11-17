import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import { flushSync } from "svelte";
import "$styles/global.css";
import * as sidebar from "$session/sidebar-mode.svelte";
import SidebarFocus from "./SidebarFocus.svelte";

vi.mock("$shared/platform", () => ({ IS_MAC: true, IS_WINDOWS: false, IS_LINUX: false }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    sidebarSetWidth: async () => {},
    sidebarResize: async () => true,
    sidebarResizeGuide: async () => true,
    settingSet: async () => ({ accepted: true, operation_id: null }),
  });
});

let screen: Awaited<ReturnType<typeof render>> | undefined;
const releases: (() => void)[] = [];
afterEach(() => {
  screen?.unmount();
  screen = undefined;
  for (const release of releases.splice(0)) release();
  sidebar.setAutoHide(false);
  sidebar.pointerLeft();
});

const waitForHide = () => new Promise((resolve) => setTimeout(resolve, sidebar.HIDE_DELAY + 80));

async function mount() {
  await page.viewport(500, 700);
  sidebar.setMode("default");
  screen = await render(SidebarFocus);
  sidebar.setAutoHide(true);
  const release = sidebar.revealHeld();
  releases.push(release);
  flushSync();
  // The test browser's pointer may start on the reveal edge.
  sidebar.pointerLeft();
  return screen;
}

test("Tab keeps the column available across fields and buttons until focus leaves", async () => {
  const screen = await mount();
  const input = screen.getByRole("textbox", { name: "Sidebar field" });
  await input.click();
  await userEvent.keyboard("{Tab}");
  await expect.element(screen.getByRole("button", { name: "First action" })).toHaveFocus();
  releases.pop()!();
  sidebar.pointerLeft();
  await waitForHide();
  expect(sidebar.isHidden()).toBe(false);
  expect(document.querySelector("aside")!.inert).toBe(false);

  await userEvent.keyboard("{Tab}");
  await expect.element(screen.getByRole("button", { name: "Second action" })).toHaveFocus();
  await waitForHide();
  expect(sidebar.isHidden()).toBe(false);

  await userEvent.keyboard("{Tab}");
  await expect.element(screen.getByRole("button", { name: "Outside action" })).toHaveFocus();
  await waitForHide();
  expect(sidebar.isHidden()).toBe(true);
  expect(document.querySelector("aside")!.inert).toBe(true);
});

test("a pointer click on a button releases keyboard focus protection", async () => {
  const screen = await mount();
  await screen.getByRole("textbox", { name: "Sidebar field" }).click();
  await userEvent.keyboard("{Tab}");
  await screen.getByRole("button", { name: "Second action" }).click();
  releases.pop()!();
  sidebar.pointerLeft();
  await waitForHide();
  expect(sidebar.isHidden()).toBe(true);
});

test("keyboard navigation from a pointer-focused button acquires its own hold", async () => {
  const screen = await mount();
  await screen.getByRole("button", { name: "First action" }).click();
  await userEvent.keyboard("{Tab}");
  await expect.element(screen.getByRole("button", { name: "Second action" })).toHaveFocus();
  releases.pop()!();
  sidebar.pointerLeft();
  await waitForHide();
  expect(sidebar.isHidden()).toBe(false);
  await userEvent.keyboard("{Tab}");
  await waitForHide();
  expect(sidebar.isHidden()).toBe(true);
});

test("window blur releases focus protection while other holds remain independent", async () => {
  const screen = await mount();
  await screen.getByRole("textbox", { name: "Sidebar field" }).click();
  await userEvent.keyboard("{Tab}");
  window.dispatchEvent(new Event("blur"));
  await waitForHide();
  expect(sidebar.isHidden()).toBe(false);
  releases.pop()!();
  await waitForHide();
  expect(sidebar.isHidden()).toBe(true);
});
