import { afterEach, expect, test, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import type {
  BookmarkCall,
  BookmarkResponse,
  BookmarkView,
  DownloadCall,
  DownloadView,
  HistoryCall,
  HistoryVisitView,
} from "$shared/ipc/bindings";
import ToolPanelsStage from "./ToolPanelsStage.svelte";

const native = vi.hoisted(() => ({
  run: vi.fn(async () => ({ accepted: true, operation_id: null })),
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    bookmarkCall: async (_profile: string, call: BookmarkCall) => answer(call),
    historyCall: async (_profile: string, call: HistoryCall) =>
      call.kind === "page"
        ? { kind: "page" as const, visits: visits(), next: null }
        : { kind: "removed" as const, count: 0 },
    downloadCall: async (_profile: string, call: DownloadCall) =>
      call.kind === "list"
        ? {
            kind: "page" as const,
            cleanup: { running: false, error: null },
            entries: downloads(),
            next: null,
            supported: true,
          }
        : { kind: "applied" as const },
    runCommand: native.run,
    keymapEntries: async () => [],
    settingGet: async () => null,
  });
});
vi.mock("$shared/ipc/native-events", () => ({
  events: {
    downloadsChanged: { listen: () => Promise.resolve(() => {}) },
    uiCommand: { listen: () => Promise.resolve(() => {}) },
  },
}));

const view = (
  id: string,
  title: string,
  url: string | null,
  children = 0,
  parent: string | null = null,
): BookmarkView => ({ id, title, url, icon: null, children, parent });

const TOP = [
  view("f1", "Reading", null, 4),
  view("f2", "Design", null, 12),
  view("1", "Quasar", "https://zephium.app/"),
  view("2", "Rust documentation", "https://doc.rust-lang.org/std/"),
  view("3", "The Verge", "https://www.theverge.com/"),
  view(
    "4",
    "A very long bookmark title that has to be cut somewhere sensible",
    "https://example.com/x",
  ),
];
const INSIDE = [
  view("5", "Long read: the history of the sidebar", "https://essay.example/a", 0, "f1"),
  view("6", "Notes on motion", "https://motion.example/b", 0, "f1"),
];

function answer(call: BookmarkCall): BookmarkResponse {
  if (call.kind === "list" && call.folder === "f1")
    return {
      kind: "listing",
      folder: "f1",
      path: [{ id: "f1", title: "Reading" }],
      items: INSIDE,
    };
  if (call.kind === "list" || call.kind === "reveal")
    return { kind: "listing", folder: null, path: [], items: TOP };
  return { kind: "saved", id: null };
}

function visits(): HistoryVisitView[] {
  const now = Math.floor(Date.now() / 1000);
  return [
    "Inside the new Quasar sidebar",
    "Rust async book",
    "Apple Human Interface Guidelines",
    "Hacker News",
    "A page with a very long title that should be cut off",
  ].map((title, index) => ({
    id: String(100 - index),
    url: `https://site-${index}.example/page`,
    title,
    visited_at: String(now - index * 1500),
    icon: null,
  }));
}

function downloads(): DownloadView[] {
  const now = Math.floor(Date.now() / 1000);
  const base = { revision: "1", source_is_context: false, error: null, received: "0" };
  return [
    {
      ...base,
      id: "d1",
      created_at: String(now - 60),
      filename: "Quasar-0.9.0-arm64.dmg",
      source: "https://zephium.app",
      state: "receiving",
      received: "48000000",
      total: "120000000",
    },
    {
      ...base,
      id: "d2",
      created_at: String(now - 4000),
      filename: "quarterly-report.pdf",
      source: "https://files.example.com",
      state: "completed",
      received: "2400000",
      total: "2400000",
    },
    {
      ...base,
      id: "d3",
      created_at: String(now - 90_000),
      filename: "archive.zip",
      source: "https://files.example.com",
      state: "completed",
      received: "81000000",
      total: "81000000",
    },
  ] as DownloadView[];
}

afterEach(() => vi.clearAllMocks());

const settle = (ms = 450) => new Promise((done) => setTimeout(done, ms));
const shots = "../../../../../target/look/tool-panels";

async function shoot(container: HTMLElement, name: string) {
  for (const theme of ["light", "dark"]) {
    document.documentElement.dataset.theme = theme;
    await settle(250);
    await page.elementLocator(container).screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
}

test("bookmarks, history and downloads share one header, one search and one rhythm", async () => {
  await page.viewport(420, 700);
  const bookmarks = await render(ToolPanelsStage, { tool: "bookmarks" });
  await expect.element(bookmarks.getByText("Quasar").first()).toBeVisible();
  await settle();
  await shoot(bookmarks.container, "bookmarks");
  await bookmarks.getByRole("button", { name: "Add to Bookmarks" }).click();
  await settle();
  await shoot(bookmarks.container, "bookmarks-menu");
  await bookmarks.getByRole("menuitem", { name: /Add Link/ }).click();
  await settle();
  await shoot(bookmarks.container, "bookmarks-add");
  await bookmarks.unmount();

  const history = await render(ToolPanelsStage, { tool: "history" });
  await expect.element(history.getByText("Rust async book")).toBeVisible();
  await settle();
  await shoot(history.container, "history");
  await history.unmount();

  const downloadsView = await render(ToolPanelsStage, { tool: "downloads" });
  await expect.element(downloadsView.getByText("quarterly-report")).toBeVisible();
  await settle();
  await shoot(downloadsView.container, "downloads");
  await downloadsView.unmount();
});
