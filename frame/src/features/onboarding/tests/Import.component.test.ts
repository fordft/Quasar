import { afterEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { flushSync } from "svelte";
import {
  browserImport,
  type ImportAdapter,
  type ImportJob,
  type ImportProblem,
  type ImportSource,
} from "$domain/browser-import";
import Import from "../components/steps/Import.svelte";

vi.mock("$domain/preferences", () => ({
  preferences: { value: () => "google", saving: () => false, set: vi.fn(async () => {}) },
}));

function native() {
  let sources: ImportSource[] = [
    {
      id: "chrome",
      browser: "chrome",
      name: "Chrome",
      profiles: [{ id: "Default", name: "Person 1" }],
      kinds: ["bookmarks", "history"],
      needsPermission: false,
      running: false,
    },
    {
      id: "safari",
      browser: "safari",
      name: "Safari",
      profiles: [{ id: "safari-default", name: "Default" }],
      kinds: ["bookmarks", "history"],
      needsPermission: false,
      running: false,
    },
  ];
  let listener: ((job: ImportJob) => void) | null = null;
  const port: ImportAdapter = {
    sources: vi.fn(async () => sources),
    start: vi.fn(async () => true),
    cancel: vi.fn(async () => {}),
    openPermissionSettings: vi.fn(async () => {}),
    onProgress: (next) => {
      listener = next;
      return () => (listener = null);
    },
  };
  return {
    port,
    emit: (job: ImportJob) => flushSync(() => listener?.(job)),
    permission: (needed: boolean) => {
      sources = sources.map((source) => ({ ...source, needsPermission: needed }));
    },
  };
}

function failed(problem: ImportProblem): ImportJob {
  return {
    source: "chrome",
    profile: "Default",
    kinds: [
      { kind: "bookmarks", state: "done", done: 12, total: 12 },
      { kind: "history", state: "failed", done: 0, total: null, problem },
    ],
    finished: true,
    cancelled: false,
  };
}

afterEach(() => browserImport.provide(null));

test.each([
  ["busy", "Quit Chrome first. It keeps its data locked while open."],
  ["unreadable", "Couldn't read what Chrome keeps."],
] as const)(
  "a %s failure can be retried without repeating successful kinds",
  async (problem, hint) => {
    const nativeImport = native();
    browserImport.provide(nativeImport.port);
    await browserImport.detect();
    const screen = await render(Import);
    await screen.getByRole("radio", { name: "Chrome", exact: true }).click();
    await screen.getByRole("button", { name: "Import", exact: true }).click();
    nativeImport.emit(failed(problem));
    await expect.element(screen.getByText(hint, { exact: true })).toBeVisible();
    await expect.element(screen.getByText("12 items brought over")).toBeVisible();

    // Returning after quitting the source refreshes discovery, keeping the
    // partial result until the person explicitly asks to try again.
    const detections = vi.mocked(nativeImport.port.sources).mock.calls.length;
    window.dispatchEvent(new Event("focus"));
    await expect
      .poll(() => vi.mocked(nativeImport.port.sources).mock.calls.length)
      .toBeGreaterThan(detections);
    expect(browserImport.current()?.finished).toBe(true);
    await screen.getByRole("button", { name: "Try again", exact: true }).click();
    await expect
      .element(screen.getByRole("button", { name: "History", exact: true }))
      .toHaveAttribute("aria-pressed", "true");
    await expect
      .element(screen.getByRole("button", { name: "Bookmarks", exact: true }))
      .toHaveAttribute("aria-pressed", "false");
    expect(nativeImport.port.start).toHaveBeenCalledOnce();
    await screen.getByRole("button", { name: "Import", exact: true }).click();
    expect(nativeImport.port.start).toHaveBeenLastCalledWith("chrome", "Default", ["history"]);
    await screen.unmount();
  },
);

test("permission can be granted from a failed result, then refreshed and retried on revisit", async () => {
  const nativeImport = native();
  browserImport.provide(nativeImport.port);
  await browserImport.detect();
  nativeImport.emit(failed("permission"));
  nativeImport.permission(true);
  await browserImport.detect();
  const screen = await render(Import);
  await expect
    .element(screen.getByText("Chrome needs access in System Settings first."))
    .toBeVisible();
  await expect.element(screen.getByText("12 items brought over")).toBeVisible();
  await screen.getByRole("button", { name: "Allow access", exact: true }).click();
  expect(nativeImport.port.openPermissionSettings).toHaveBeenCalledWith("chrome");
  nativeImport.permission(false);
  window.dispatchEvent(new Event("focus"));
  await expect.poll(() => browserImport.found()?.[0]?.needsPermission).toBe(false);
  expect(browserImport.current()).toEqual(failed("permission"));
  await screen.getByRole("button", { name: "Try again", exact: true }).click();
  await screen.getByRole("button", { name: "Import", exact: true }).click();
  expect(nativeImport.port.start).toHaveBeenCalledWith("chrome", "Default", ["history"]);
  await screen.unmount();
});

test("choosing another browser dismisses a completed result and admits a new source", async () => {
  const nativeImport = native();
  browserImport.provide(nativeImport.port);
  await browserImport.detect();
  nativeImport.emit({ ...failed("busy"), kinds: [failed("busy").kinds[0]!] });
  const screen = await render(Import);
  await expect.element(screen.getByText("12 items brought over")).toBeVisible();
  await screen.getByRole("radio", { name: "Safari", exact: true }).click();
  expect(browserImport.current()).toBeNull();
  expect(nativeImport.port.start).not.toHaveBeenCalled();
  await screen.getByRole("button", { name: "Import", exact: true }).click();
  expect(nativeImport.port.start).toHaveBeenCalledWith("safari", "safari-default", [
    "bookmarks",
    "history",
  ]);
  await screen.unmount();
});

test("source selection stays disabled until native confirms cancellation", async () => {
  const nativeImport = native();
  browserImport.provide(nativeImport.port);
  await browserImport.detect();
  nativeImport.emit({
    ...failed("busy"),
    finished: false,
    kinds: [{ kind: "history", state: "running", done: 0, total: null }],
  });
  const screen = await render(Import);
  await expect.element(screen.getByRole("radio", { name: "Safari", exact: true })).toBeDisabled();
  await screen.getByRole("button", { name: "Stop", exact: true }).click();
  expect(nativeImport.port.cancel).toHaveBeenCalledOnce();
  expect(browserImport.current()?.finished).toBe(false);
  nativeImport.emit({
    ...failed("busy"),
    cancelled: true,
    kinds: [{ kind: "history", state: "skipped", done: 0, total: null }],
  });
  await screen.getByRole("button", { name: "Try again", exact: true }).click();
  await screen.getByRole("button", { name: "Import", exact: true }).click();
  expect(nativeImport.port.start).toHaveBeenCalledWith("chrome", "Default", ["history"]);
  await screen.unmount();
});
