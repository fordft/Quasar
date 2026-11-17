import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page, userEvent } from "vitest/browser";
import "$styles/global.css";
import type {
  LauncherTrigger,
  TriggerChange,
  ShortcutRecordingRequest,
  ShortcutRecordingView,
} from "$shared/ipc/bindings";
import { IS_MAC } from "$shared/platform";
import { acceleratorKeys } from "$shared/lib/accelerator";
import LauncherShortcut from "../components/LauncherShortcut.svelte";
import { recordingNative } from "./shortcut-recording-native";
import * as m from "$shared/i18n/messages";

const trigger = (shortcut: string): LauncherTrigger => ({
  shortcut,
  default_shortcut: "CmdOrCtrl+Shift+Space",
  registered: true,
  editable: true,
  double_tap: "off",
  double_tap_supported: true,
  accessibility: false,
});
const native = vi.hoisted(() => ({
  state: vi.fn(),
  set: vi.fn(),
  record: vi.fn<(request: ShortcutRecordingRequest) => Promise<ShortcutRecordingView | null>>(),
  doubleTap: vi.fn(),
  accessibility: vi.fn(async () => {}),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    launcherTrigger: native.state,
    launcherSetShortcut: native.set,
    launcherRecordShortcut: native.record,
    launcherSetDoubleTap: native.doubleTap,
    launcherOpenAccessibility: native.accessibility,
  });
});
let authority = recordingNative();
const cleanups: (() => void)[] = [];
beforeEach(() => {
  vi.clearAllMocks();
  authority = recordingNative();
  native.record.mockReset().mockImplementation(async (request) => authority.apply(request));
  native.set.mockReset();
  native.state.mockResolvedValue(trigger("CmdOrCtrl+Shift+Space"));
});
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
});

function browserShortcuts() {
  const run = vi.fn();
  const listener = (event: KeyboardEvent) => {
    if (!event.defaultPrevented && (event.ctrlKey || event.metaKey) && event.code === "KeyT") run();
  };
  // BrowserApp dispatches from document bubble, before a window bubble listener.
  document.addEventListener("keydown", listener);
  cleanups.push(() => document.removeEventListener("keydown", listener));
  return run;
}

function press(primary: "ctrlKey" | "metaKey" = "ctrlKey") {
  const event = new KeyboardEvent("keydown", {
    code: "KeyT",
    key: "t",
    [primary]: true,
    bubbles: true,
    cancelable: true,
  });
  document.body.dispatchEvent(event);
  return event;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const settled = () => new Promise((done) => setTimeout(done, 0));

test("successful apply reads the registration result after native release", async () => {
  native.state
    .mockResolvedValueOnce(trigger("CmdOrCtrl+Shift+Space"))
    .mockResolvedValue({ ...trigger("Ctrl+Alt+Space"), registered: false });
  native.set.mockResolvedValue({ type: "applied", trigger: trigger("Ctrl+Alt+Space") });
  const screen = await render(LauncherShortcut);
  await screen.getByRole("button", { name: "Record a shortcut" }).click();
  await settled();
  press();
  await settled();
  expect(native.state).toHaveBeenCalledTimes(2);
  await expect
    .element(screen.getByText(m.launcher_shortcut_unregistered(), { exact: true }))
    .toBeVisible();
  await expect
    .element(screen.getByText(m.launcher_shortcut_ready(), { exact: true }))
    .not.toBeInTheDocument();
});

test.each(["null", "error"] as const)(
  "a %s post-release refresh cannot retain a ready claim",
  async (failure) => {
    native.state.mockReset().mockResolvedValueOnce(trigger("CmdOrCtrl+Shift+Space"));
    if (failure === "error") native.state.mockRejectedValueOnce(new Error("refresh unavailable"));
    else native.state.mockResolvedValueOnce(null);
    native.set.mockResolvedValue({ type: "applied", trigger: trigger("Ctrl+Alt+Space") });
    const screen = await render(LauncherShortcut);
    await screen.getByRole("button", { name: "Record a shortcut" }).click();
    await settled();
    press();
    await settled();
    expect(native.state).toHaveBeenCalledTimes(2);
    await expect.element(screen.getByText(m.panel_action_failed(), { exact: true })).toBeVisible();
    await expect
      .element(screen.getByText(m.launcher_shortcut_ready(), { exact: true }))
      .not.toBeInTheDocument();
  },
);

test("post-release verification stays pending and cannot overwrite a newer recording", async () => {
  const snapshot = deferred<LauncherTrigger | null>();
  native.state
    .mockResolvedValueOnce(trigger("CmdOrCtrl+Shift+Space"))
    .mockReturnValueOnce(snapshot.promise);
  native.set.mockResolvedValue({ type: "applied", trigger: trigger("Ctrl+Alt+Space") });
  const screen = await render(LauncherShortcut);
  await screen.getByRole("button", { name: "Record a shortcut" }).click();
  await settled();
  press();
  await settled();
  expect(native.state).toHaveBeenCalledTimes(2);
  await expect
    .element(screen.getByRole("button", { name: "Applying change…" }))
    .toHaveAttribute("aria-pressed", "false");
  await expect
    .element(screen.getByText(m.launcher_shortcut_ready(), { exact: true }))
    .not.toBeInTheDocument();
  await screen.getByRole("button", { name: "Applying change…" }).click();
  await settled();
  snapshot.resolve({ ...trigger("Ctrl+Alt+Space"), registered: false });
  await settled();
  await expect
    .element(screen.getByRole("button", { name: "Press the new shortcut. Esc cancels." }))
    .toHaveAttribute("aria-pressed", "true");
  await expect
    .element(screen.getByText(m.launcher_shortcut_unregistered(), { exact: true }))
    .not.toBeInTheDocument();
  await userEvent.keyboard("{Escape}");
  await settled();
});

test("a pending suspension is preparing, not an armed recorder", async () => {
  const ack = deferred<ShortcutRecordingView>();
  native.record.mockImplementation(async (request) => {
    const view = authority.apply(request);
    return request.kind === "start" ? ack.promise : view;
  });
  const screen = await render(LauncherShortcut);
  try {
    await screen.getByRole("button", { name: "Record a shortcut" }).click();
    await settled();
    expect(authority.view().owner).not.toBeNull();
    expect(
      screen.getByRole("button", { name: "Press the new shortcut. Esc cancels." }).query(),
    ).toBeNull();
    await expect
      .element(screen.getByRole("button", { name: "Applying change…" }))
      .toHaveAttribute("aria-pressed", "false");
    press();
    expect(native.set).not.toHaveBeenCalled();
  } finally {
    ack.resolve(authority.view());
    await settled();
    await userEvent.keyboard("{Escape}");
    await settled();
  }
});

test("cancelling a native suspension with a lost acknowledgement sends cleanup immediately", async () => {
  const ack = deferred<ShortcutRecordingView>();
  let applied!: ShortcutRecordingView;
  native.record.mockImplementation(async (request) => {
    const view = authority.apply(request);
    if (request.kind === "start") {
      applied = view;
      return ack.promise;
    }
    return view;
  });
  const screen = await render(LauncherShortcut);
  try {
    await screen.getByRole("button", { name: "Record a shortcut" }).click();
    await settled();
    await userEvent.keyboard("{Escape}");
    await settled();
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" }));
    expect(authority.view().owner).toBeNull();
  } finally {
    ack.resolve(applied);
    await settled();
  }
});

test("records a shortcut, keeps listening after a refusal, and applies the next", async () => {
  native.state.mockResolvedValue(trigger("CmdOrCtrl+Shift+Space"));
  native.set
    .mockResolvedValueOnce({
      type: "rejected",
      reason: "app_command",
      trigger: trigger("CmdOrCtrl+Shift+Space"),
    })
    .mockResolvedValueOnce({ type: "applied", trigger: trigger("Ctrl+Alt+Space") });
  const screen = await render(LauncherShortcut);
  const recorder = screen.getByRole("button", { name: "Record a shortcut" });
  await expect
    .element(recorder)
    .toHaveTextContent(acceleratorKeys("CmdOrCtrl+Shift+Space", IS_MAC).join(""));

  await recorder.click();
  expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "start" }));
  await userEvent.keyboard("{Meta>}{Shift>}z{/Shift}{/Meta}");
  await expect.element(screen.getByText(/like Redo/u)).toBeVisible();
  // Still listening: the next press is taken without another click.
  native.state.mockResolvedValue(trigger("Ctrl+Alt+Space"));
  await userEvent.keyboard("{Control>}{Alt>}{Space}{/Alt}{/Control}");
  await vi.waitFor(() =>
    expect(native.set).toHaveBeenLastCalledWith("Ctrl+Alt+Space", expect.anything()),
  );
  await expect
    .element(screen.getByRole("button", { name: "Record a shortcut" }))
    .toHaveTextContent(acceleratorKeys("Ctrl+Alt+Space", IS_MAC).join(""));
  expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" }));
  await expect.element(screen.getByRole("button", { name: "Reset" })).toBeVisible();
  document.documentElement.dataset.theme = "dark";
  await page.viewport(720, 220);
  await page.screenshot({ path: "../../../../../target/settings-launcher-shortcut.png" });
  delete document.documentElement.dataset.theme;
});

test("Ctrl/Cmd+T is captured before browser dispatch, stays captured after refusal, and works after Escape", async () => {
  const run = browserShortcuts();
  native.set.mockResolvedValue({
    type: "rejected",
    reason: "app_command",
    trigger: trigger("CmdOrCtrl+Shift+Space"),
  });
  const screen = await render(LauncherShortcut);
  await screen.getByRole("button", { name: "Record a shortcut" }).click();
  await settled();
  for (const primary of ["ctrlKey", "metaKey"] as const) {
    const previous = native.set.mock.calls.length;
    expect(press(primary).defaultPrevented).toBe(true);
    await vi.waitFor(() => expect(native.set).toHaveBeenCalledTimes(previous + 1));
    await expect.element(screen.getByText(/like Redo/u)).toBeVisible();
    await settled();
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "start" }));
    expect(run).not.toHaveBeenCalled();
  }
  await userEvent.keyboard("{Escape}");
  await vi.waitFor(() =>
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
  );
  press("ctrlKey");
  press("metaKey");
  expect(run).toHaveBeenCalledTimes(2);
});

test("a deferred apply keeps capturing keys and success restores native shortcuts", async () => {
  const run = browserShortcuts();
  const applying = deferred<TriggerChange | null>();
  native.set.mockReturnValueOnce(applying.promise);
  const screen = await render(LauncherShortcut);
  await screen.getByRole("button", { name: "Record a shortcut" }).click();
  await settled();
  expect(press().defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(native.set).toHaveBeenCalledOnce());
  expect(press("metaKey").defaultPrevented).toBe(true);
  expect(native.set).toHaveBeenCalledOnce();
  expect(run).not.toHaveBeenCalled();
  native.state.mockResolvedValue(trigger("Ctrl+Alt+Space"));
  applying.resolve({ type: "applied", trigger: trigger("Ctrl+Alt+Space") });
  await vi.waitFor(() =>
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
  );
  await expect
    .element(screen.getByRole("button", { name: "Record a shortcut" }))
    .toHaveTextContent(acceleratorKeys("Ctrl+Alt+Space", IS_MAC).join(""));
  press();
  expect(run).toHaveBeenCalledOnce();
});

test.each(["null", "error"] as const)(
  "a native apply %s ends recording and restores shortcuts",
  async (failure) => {
    const run = browserShortcuts();
    if (failure === "error") native.set.mockRejectedValueOnce(new Error("native unavailable"));
    else native.set.mockResolvedValueOnce(null);
    const screen = await render(LauncherShortcut);
    await screen.getByRole("button", { name: "Record a shortcut" }).click();
    await settled();
    press();
    await vi.waitFor(() =>
      expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
    );
    await expect
      .element(screen.getByRole("button", { name: "Record a shortcut" }))
      .toHaveAttribute("aria-pressed", "false");
    press();
    expect(run).toHaveBeenCalledOnce();
  },
);

test("failure to suspend native shortcuts cleans up before accepting a chord", async () => {
  const run = browserShortcuts();
  native.record
    .mockImplementationOnce(async (request) => authority.apply(request))
    .mockRejectedValueOnce(new Error("recording unavailable"));
  const screen = await render(LauncherShortcut);
  await screen.getByRole("button", { name: "Record a shortcut" }).click();
  await vi.waitFor(() =>
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
  );
  await expect
    .element(screen.getByRole("button", { name: "Record a shortcut" }))
    .toHaveAttribute("aria-pressed", "false");
  press();
  expect(native.set).not.toHaveBeenCalled();
  expect(run).toHaveBeenCalledOnce();
});

test.each(["Escape", "blur", "unmount"] as const)(
  "%s restores shortcuts during a pending apply, whose late refusal cannot affect a newer recording",
  async (exit) => {
    const run = browserShortcuts();
    const applying = deferred<TriggerChange | null>();
    native.set.mockReturnValueOnce(applying.promise);
    let screen = await render(LauncherShortcut);
    await screen.getByRole("button", { name: "Record a shortcut" }).click();
    await settled();
    press();
    await vi.waitFor(() => expect(native.set).toHaveBeenCalledOnce());
    if (exit === "Escape") await userEvent.keyboard("{Escape}");
    else if (exit === "blur") window.dispatchEvent(new Event("blur"));
    else await screen.unmount();
    await vi.waitFor(() =>
      expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
    );
    await settled();
    press();
    expect(run).toHaveBeenCalledOnce();
    if (exit === "unmount") screen = await render(LauncherShortcut);
    await screen.getByRole("button", { name: "Record a shortcut" }).click();
    await settled();
    const requests = native.record.mock.calls.length;
    applying.resolve({
      type: "rejected",
      reason: "app_command",
      trigger: trigger("Ctrl+Alt+KeyY"),
    });
    await settled();
    expect(native.record).toHaveBeenCalledTimes(requests);
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "start" }));
    await expect
      .element(screen.getByRole("button", { name: "Press the new shortcut. Esc cancels." }))
      .toHaveAttribute("aria-pressed", "true");
    await expect.element(screen.getByText(/like Redo/u)).not.toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await vi.waitFor(() =>
      expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
    );
  },
);

test("a delayed native stop ACK cannot block a new Start or arm its pending UI", async () => {
  const stop = deferred<ShortcutRecordingView>();
  const start = deferred<ShortcutRecordingView>();
  const run = browserShortcuts();
  let released!: ShortcutRecordingView;
  let starts = 0;
  native.record.mockImplementation(async (request) => {
    const view = authority.apply(request);
    if (request.kind === "stop" && starts === 1) {
      released = view;
      return stop.promise;
    }
    if (request.kind === "start" && ++starts === 2) return start.promise;
    return view;
  });
  const screen = await render(LauncherShortcut);
  await screen.getByRole("button", { name: "Record a shortcut" }).click();
  await settled();
  await userEvent.keyboard("{Escape}");
  await vi.waitFor(() =>
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
  );
  await screen.getByRole("button", { name: "Applying change…" }).click();
  await settled();
  expect(starts).toBe(2);
  expect(press().defaultPrevented).toBe(true);
  expect(run).not.toHaveBeenCalled();
  expect(native.set).not.toHaveBeenCalled();
  stop.resolve(released);
  await settled();
  await expect
    .element(screen.getByRole("button", { name: "Applying change…" }))
    .toHaveAttribute("aria-pressed", "false");
  start.resolve(authority.view());
  await settled();
  expect(native.state).toHaveBeenCalledOnce();
  await expect
    .element(screen.getByRole("button", { name: "Press the new shortcut. Esc cancels." }))
    .toHaveAttribute("aria-pressed", "true");
  await userEvent.keyboard("{Escape}");
  await vi.waitFor(() =>
    expect(native.record).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "stop" })),
  );
});

test("Reset restores the original shortcut without starting a recording", async () => {
  native.state.mockResolvedValue(trigger("Ctrl+Alt+KeyK"));
  native.set.mockResolvedValue({ type: "applied", trigger: trigger("CmdOrCtrl+Shift+Space") });
  const screen = await render(LauncherShortcut);
  await screen.getByRole("button", { name: "Reset", exact: true }).click();
  expect(native.set).toHaveBeenCalledWith("CmdOrCtrl+Shift+Space", null);
  await expect
    .element(screen.getByRole("button", { name: "Reset", exact: true }))
    .not.toBeInTheDocument();
  expect(native.record).not.toHaveBeenCalled();
});

test("double tap says what it is waiting for, and where to grant it", async () => {
  native.state.mockResolvedValue({ ...trigger("CmdOrCtrl+Shift+Space"), double_tap: "command" });
  const screen = await render(LauncherShortcut);
  await expect.element(screen.getByText(/Needs Accessibility permission/u)).toBeVisible();
  await screen.getByRole("button", { name: "Open System Settings" }).click();
  expect(native.accessibility).toHaveBeenCalledOnce();

  // Granted in System Settings: the page notices on its own.
  native.state.mockResolvedValue({
    ...trigger("CmdOrCtrl+Shift+Space"),
    double_tap: "command",
    accessibility: true,
  });
  await expect
    .element(screen.getByText(/never clashes with a shortcut/u), { timeout: 4000 })
    .toBeVisible();
});
