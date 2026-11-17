import { afterEach, describe, expect, it, vi } from "vitest";
import { browserImport, type ImportAdapter, type ImportJob } from "$domain/browser-import";

function adapter(overrides: Partial<ImportAdapter> = {}) {
  let listener: ((job: ImportJob) => void) | null = null;
  const port: ImportAdapter = {
    sources: async () => [
      {
        id: "chrome:default",
        browser: "chrome",
        name: "Google Chrome",
        profiles: [{ id: "Default", name: "Person 1" }],
        kinds: ["bookmarks", "history"],
        needsPermission: false,
        running: false,
      },
    ],
    start: vi.fn(async () => true),
    cancel: vi.fn(async () => {}),
    onProgress: (next) => {
      listener = next;
      return () => (listener = null);
    },
    openPermissionSettings: vi.fn(async () => {}),
    ...overrides,
  };
  return { port, emit: (job: ImportJob) => listener?.(job) };
}

const running = (finished: boolean): ImportJob => ({
  source: "chrome:default",
  profile: "Default",
  kinds: [{ kind: "bookmarks", state: finished ? "done" : "running", done: 12, total: 40 }],
  finished,
  cancelled: false,
});

afterEach(() => browserImport.provide(null));

describe("browser import", () => {
  it("claims nothing until native provides a way to import", async () => {
    browserImport.provide(null);
    expect(browserImport.available()).toBe(false);
    await browserImport.detect();
    expect(browserImport.found()).toBeNull();
    expect(await browserImport.start("chrome:default", "Default", ["history"])).toBe(false);
  });

  it("lists sources once native provides them", async () => {
    browserImport.provide(adapter().port);
    expect(browserImport.available()).toBe(true);
    await browserImport.detect();
    expect(browserImport.found()?.map((source) => source.name)).toEqual(["Google Chrome"]);
  });

  it("follows the job native reports and is busy until it finishes", async () => {
    const { port, emit } = adapter();
    browserImport.provide(port);
    await browserImport.detect();
    expect(await browserImport.start("chrome:default", "Default", ["bookmarks"])).toBe(true);
    emit(running(false));
    expect(browserImport.busy()).toBe(true);
    expect(await browserImport.start("chrome:default", "Default", ["history"])).toBe(false);
    emit(running(true));
    expect(browserImport.busy()).toBe(false);
    expect(browserImport.current()?.kinds[0]?.state).toBe("done");
  });

  it("keeps following progress after looking for sources again", async () => {
    const { port, emit } = adapter();
    browserImport.provide(port);
    await browserImport.detect();
    await browserImport.detect();
    emit(running(false));
    expect(browserImport.current()?.finished).toBe(false);
  });

  it("keeps a running job when dismissed or redetected, including pending cancellation", async () => {
    const { port, emit } = adapter();
    browserImport.provide(port);
    await browserImport.detect();
    emit(running(false));
    await browserImport.cancel();
    browserImport.dismiss();
    await browserImport.detect();
    expect(port.cancel).toHaveBeenCalledOnce();
    expect(browserImport.busy()).toBe(true);
    expect(browserImport.current()).toEqual(running(false));
    expect(await browserImport.start("other", "Default", ["history"])).toBe(false);
    expect(port.start).not.toHaveBeenCalled();
  });

  it("can dismiss a failed attempt, refresh permission, and retry without claiming success", async () => {
    const { port, emit } = adapter();
    const sources = vi.fn(port.sources);
    port.sources = sources;
    browserImport.provide(port);
    await browserImport.detect();
    expect(await browserImport.start("chrome:default", "Default", ["history"])).toBe(true);
    const failed: ImportJob = {
      ...running(true),
      kinds: [{ kind: "history", state: "failed", done: 0, total: null, problem: "permission" }],
    };
    emit(failed);
    await browserImport.openPermissionSettings(failed.source);
    await browserImport.detect();
    expect(port.openPermissionSettings).toHaveBeenCalledWith(failed.source);
    expect(browserImport.current()).toEqual(failed);
    browserImport.dismiss();
    expect(browserImport.current()).toBeNull();
    expect(await browserImport.start(failed.source, failed.profile, ["history"])).toBe(true);
    expect(port.start).toHaveBeenCalledTimes(2);
    expect(sources).toHaveBeenCalledTimes(2);
    expect(browserImport.current()).toBeNull();
    emit(running(false));
    expect(browserImport.busy()).toBe(true);
  });

  it("can put a finished result away and import another browser", async () => {
    const { port, emit } = adapter();
    browserImport.provide(port);
    await browserImport.detect();
    emit(running(true));
    browserImport.dismiss();
    expect(await browserImport.start("firefox", "profile-2", ["bookmarks"])).toBe(true);
    expect(port.start).toHaveBeenCalledWith("firefox", "profile-2", ["bookmarks"]);
    emit({ ...running(false), source: "firefox", profile: "profile-2" });
    expect(browserImport.current()?.source).toBe("firefox");
  });

  it("refuses to start with nothing chosen", async () => {
    const { port } = adapter();
    browserImport.provide(port);
    expect(await browserImport.start("chrome:default", "Default", [])).toBe(false);
    expect(port.start).not.toHaveBeenCalled();
  });

  it("keeps an empty list when native cannot list sources", async () => {
    browserImport.provide(
      adapter({
        sources: async () => {
          throw new Error("unavailable");
        },
      }).port,
    );
    await browserImport.detect();
    expect(browserImport.found()).toEqual([]);
  });
});
