import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const native = vi.hoisted(() => ({
  get: vi.fn(),
  set: vi.fn(),
  settle: vi.fn(),
  stop: vi.fn(),
  listener: null as null | ((event: { payload: string }) => void),
  listen: vi.fn(),
  linux: false,
}));
vi.mock("$shared/platform", () => ({
  get IS_LINUX() {
    return native.linux;
  },
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ settingGet: native.get, settingSet: native.set });
});
vi.mock("$domain/operations", () => ({ settle: native.settle }));
vi.mock("$shared/ipc/native-events", () => ({ events: { uiCommand: { listen: native.listen } } }));
beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers();
  vi.resetAllMocks();
  native.listener = null;
  native.linux = false;
  native.listen.mockImplementation((listener: typeof native.listener) => {
    native.listener = listener;
    return Promise.resolve(native.stop);
  });
  native.get.mockResolvedValue(null);
  native.set.mockResolvedValue({ accepted: true, operation_id: "0000000000000001" });
  native.settle.mockResolvedValue({ outcome: "deferred", disposition: null });
});
afterEach(() => vi.useRealTimers());
describe("preference observation lifecycle", () => {
  it("resets Linux preferences without enabling unsupported AI or Work", async () => {
    native.linux = true;
    const stored = new Map([
      ["ui.language", "fr"],
      ["tabs.after-close", "previous"],
      ["performance.after", "30"],
    ]);
    native.get.mockImplementation(async (key: string) =>
      key === "ai.enabled" || key === "work.enabled" ? "false" : (stored.get(key) ?? null),
    );
    native.set.mockImplementation(async (key: string, next: string) => {
      if (key === "ai.enabled" || key === "work.enabled") throw new Error("unsupported feature");
      stored.set(key, next);
      return { accepted: true, operation_id: "0000000000000001" };
    });
    native.settle.mockImplementation(async (admission: Promise<unknown>) => {
      await admission;
      return { outcome: "applied", disposition: null };
    });
    const preferences = await import("../preferences.svelte");
    await preferences.init();
    expect(await preferences.resetAll()).toBe(true);
    expect(preferences.aiEnabled()).toBe(false);
    expect(preferences.enabled("work")).toBe(false);
    expect(preferences.value("ui.language")).toBe("system");
    expect(preferences.value("tabs.after-close")).toBe("next");
    expect(preferences.value("performance.after")).toBe("15");
    expect(native.set.mock.calls.map(([key]) => key)).toEqual([
      "ui.language",
      "tabs.after-close",
      "performance.after",
    ]);
    preferences.dispose();
  });
  it("initializes once and keeps a live update ahead of an older startup read", async () => {
    const preferences = await import("../preferences.svelte");
    let finish!: (value: string) => void;
    native.get.mockImplementation((key: string) =>
      key === "ui.accent"
        ? new Promise((resolve) => {
            finish = resolve;
          })
        : Promise.resolve(null),
    );
    const first = preferences.init();
    expect(preferences.init()).toBe(first);
    await Promise.resolve();
    native.listener?.({ payload: "preference.ui.accent=sage" });
    finish("sky");
    await first;
    expect(native.listen).toHaveBeenCalledOnce();
    expect(preferences.value("ui.accent")).toBe("sage");
    preferences.dispose();
    expect(native.stop).toHaveBeenCalledOnce();
  });
  it("cannot leave a hung readback pending forever", async () => {
    const preferences = await import("../preferences.svelte");
    await preferences.init();
    native.get.mockImplementation(() => new Promise(() => {}));
    const pending = preferences.set("ui.accent", "sky");
    await vi.advanceTimersByTimeAsync(1500);
    await pending;
    expect(preferences.saving()).toBe(false);
    expect(preferences.saveFailed()).toBe(true);
    expect(preferences.value("ui.accent")).toBe("graphite");
    preferences.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });
  it("cancels readback observation on disposal without overwriting the next generation", async () => {
    const preferences = await import("../preferences.svelte");
    await preferences.init();
    let finish!: (value: string) => void;
    native.get.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const pending = preferences.set("ui.accent", "sky");
    await Promise.resolve();
    preferences.dispose();
    await pending;
    native.get.mockResolvedValue(null);
    await preferences.init();
    finish("sky");
    await Promise.resolve();
    expect(preferences.value("ui.accent")).toBe("graphite");
    expect(preferences.saveFailed()).toBe(false);
    preferences.dispose();
  });
  it("uses confirmed readback when the event was missed, without replacing a newer event", async () => {
    const preferences = await import("../preferences.svelte");
    await preferences.init();
    native.get.mockResolvedValue("sky");
    await preferences.set("ui.accent", "sky");
    expect(preferences.value("ui.accent")).toBe("sky");
    native.get.mockImplementation(async () => {
      native.listener?.({ payload: "preference.ui.accent=rose" });
      return "sage";
    });
    await preferences.set("ui.accent", "sage");
    expect(preferences.value("ui.accent")).toBe("rose");
    preferences.dispose();
  });
});
