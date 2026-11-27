import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import "$styles/global.css";
import type {
  KeymapEntry,
  LauncherTrigger,
  ShortcutRecordingRequest,
  ShortcutRecordingView,
} from "$shared/ipc/bindings";
import { keymap } from "$domain/keymap";
import KeyboardPage from "../components/sections/KeyboardPage.svelte";
import { IS_MAC } from "$shared/platform";
import { acceleratorKeys } from "$shared/lib/accelerator";
import { recordingNative } from "./shortcut-recording-native";
import * as m from "$shared/i18n/messages";

// These tests read the macOS keycaps (⌘, ⌥, ⇧); Windows spells its shortcuts out.
vi.mock("$shared/platform", () => ({ IS_MAC: true, IS_WINDOWS: false }));

const entry = (id: string, title: string, accelerator: string | null, customized = false) =>
  ({
    id,
    title,
    group: "file",
    accelerator,
    default_accelerator: accelerator,
    customizable: true,
    customized,
  }) satisfies KeymapEntry;

const trigger: LauncherTrigger = {
  shortcut: "CmdOrCtrl+Shift+Space",
  default_shortcut: "CmdOrCtrl+Shift+Space",
  registered: true,
  editable: true,
  double_tap: "off",
  double_tap_supported: false,
  accessibility: false,
};

const native = vi.hoisted(() => ({
  entries: vi.fn(),
  bind: vi.fn(),
  reset: vi.fn(),
  record: vi.fn<(request: ShortcutRecordingRequest) => Promise<ShortcutRecordingView | null>>(),
  launcherRecord:
    vi.fn<(request: ShortcutRecordingRequest) => Promise<ShortcutRecordingView | null>>(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    keymapEntries: native.entries,
    keymapBind: native.bind,
    keymapReset: native.reset,
    keymapRecord: native.record,
    launcherTrigger: async () => trigger,
    launcherRecordShortcut: native.launcherRecord,
  });
});

let authority = recordingNative();
beforeEach(async () => {
  authority = recordingNative();
  native.record.mockImplementation(async (request) => authority.apply(request));
  native.launcherRecord.mockImplementation(async (request) => authority.apply(request));
  native.entries.mockResolvedValue([
    entry("tab.new", "New Tab", IS_MAC ? "Cmd+T" : "Ctrl+T"),
    entry("tab.close", "Close Tab", IS_MAC ? "Cmd+W" : "Ctrl+W"),
    entry("page.print", "Print…", "Cmd+Shift+P", true),
    { ...entry("profile.next", "Next Profile", "Ctrl+Cmd+P"), group: "keys" },
    { ...entry("profile.previous", "Previous Profile", "Ctrl+Cmd+Shift+P"), group: "keys" },
  ]);
  await keymap.init();
});
afterEach(() => {
  keymap.dispose();
  vi.clearAllMocks();
});

test("shows both profile shortcuts and rebinds the previous-profile command", async () => {
  native.bind.mockResolvedValue({ kind: "applied" });
  const screen = await render(KeyboardPage);
  await expect
    .element(screen.getByRole("heading", { name: "Profiles", exact: true }))
    .toBeVisible();
  await expect
    .element(screen.getByRole("button", { name: "Change the shortcut for Next profile" }))
    .toBeVisible();
  const previous = screen.getByRole("button", { name: "Change the shortcut for Previous profile" });
  await expect.element(previous).toBeVisible();
  await page.viewport(1000, 900);
  await page.screenshot({ path: "../../../../../target/settings-profile-shortcuts.png" });
  await previous.click();
  await userEvent.keyboard("{Control>}{Meta>}k{/Meta}{/Control}");
  await vi.waitFor(() =>
    expect(native.bind).toHaveBeenLastCalledWith("profile.previous", "Ctrl+Cmd+KeyK", false),
  );
});

test("records a shortcut, names a conflict, and takes the keys when asked", async () => {
  native.bind
    .mockResolvedValueOnce({ kind: "conflict", command: "tab.close" })
    .mockResolvedValueOnce({ kind: "applied" });
  const screen = await render(KeyboardPage);
  const newTab = screen.getByRole("button", { name: "Change the shortcut for New tab" });
  await expect
    .element(newTab)
    .toHaveTextContent(acceleratorKeys(IS_MAC ? "Cmd+T" : "Ctrl+T", IS_MAC).join(""));

  await newTab.click();
  expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "start" }));
  await userEvent.keyboard(IS_MAC ? "{Meta>}w{/Meta}" : "{Control>}w{/Control}");
  await vi.waitFor(() =>
    expect(native.bind).toHaveBeenLastCalledWith(
      "tab.new",
      IS_MAC ? "Cmd+KeyW" : "Ctrl+KeyW",
      false,
    ),
  );
  expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" }));
  await expect.element(screen.getByText("Already used by Close tab.")).toBeVisible();

  await screen.getByRole("button", { name: "Use here" }).click();
  await vi.waitFor(() =>
    expect(native.bind).toHaveBeenLastCalledWith(
      "tab.new",
      IS_MAC ? "Cmd+KeyW" : "Ctrl+KeyW",
      true,
    ),
  );
  await expect.element(screen.getByText("Already used by Close tab.")).not.toBeInTheDocument();

  document.documentElement.dataset.theme = "dark";
  await page.viewport(720, 640);
  await page.screenshot({ path: "../../../../../target/settings-keyboard.png" });
  delete document.documentElement.dataset.theme;
});

test("Delete clears a shortcut, Escape keeps it, and Reset restores the default", async () => {
  native.bind.mockResolvedValue({ kind: "applied" });
  native.reset.mockResolvedValue(true);
  const screen = await render(KeyboardPage);
  const close = screen.getByRole("button", { name: "Change the shortcut for Close tab" });

  await close.click();
  await userEvent.keyboard("{Escape}");
  expect(native.bind).not.toHaveBeenCalled();
  await expect.element(close).toHaveAttribute("aria-pressed", "false");

  await close.click();
  await userEvent.keyboard("{Backspace}");
  await vi.waitFor(() => expect(native.bind).toHaveBeenLastCalledWith("tab.close", null, false));

  await screen.getByRole("button", { name: "Reset" }).click();
  await vi.waitFor(() => expect(native.reset).toHaveBeenLastCalledWith("page.print"));
});

test("starting the launcher recorder exclusively replaces the command recorder", async () => {
  const screen = await render(KeyboardPage);
  const command = screen.getByRole("button", { name: "Change the shortcut for New tab" });
  await command.click();
  await screen.getByRole("button", { name: "Record a shortcut" }).click();
  await expect.element(command).toHaveAttribute("aria-pressed", "false");
  await expect
    .element(screen.getByRole("button", { name: "Press the new shortcut. Esc cancels." }))
    .toHaveAttribute("aria-pressed", "true");
  const owner = authority.view();
  const staleStop = native.record.mock.calls
    .map(([request]) => request)
    .find((request) => request.kind === "stop")!;
  expect(authority.apply(staleStop)).toEqual(owner);
  // The original field now starts a fresh exclusive owner, rather than sending
  // a stale stop that could release the launcher's lease.
  await command.click();
  await expect.element(command).toHaveAttribute("aria-pressed", "true");
  await expect
    .element(screen.getByRole("button", { name: "Record a shortcut" }))
    .toHaveAttribute("aria-pressed", "false");
  await userEvent.keyboard("{Escape}");
});

test("an unconfirmed command-recorder release remains visible on its row", async () => {
  native.record.mockImplementation(async (request) => {
    if (request.kind === "stop") throw new Error("release acknowledgement lost");
    return authority.apply(request);
  });
  const screen = await render(KeyboardPage);
  const command = screen.getByRole("button", { name: "Change the shortcut for New tab" });
  await command.click();
  await expect.element(command).toHaveAttribute("aria-pressed", "true");
  await userEvent.keyboard("{Escape}");
  await expect.element(command).toHaveAttribute("aria-pressed", "false");
  await expect.element(screen.getByText(m.keymap_failed(), { exact: true })).toBeVisible();
});
