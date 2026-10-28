import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({
  width: vi.fn(async (..._args: unknown[]) => undefined),
  set: vi.fn(async (..._args: unknown[]) => ({ operation_id: null, accepted: true })),
}));

vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    sidebarSetWidth: native.width,
    settingSet: native.set,
    settingGet: vi.fn(async () => null),
  });
});

type Sidebar = typeof import("../sidebar-mode.svelte");
let sidebar: Sidebar;

beforeEach(async () => {
  vi.useFakeTimers();
  vi.resetModules();
  native.width.mockClear();
  native.set.mockClear();
  sidebar = await import("../sidebar-mode.svelte");
});
afterEach(() => vi.useRealTimers());

/** The widths native was told, in order. */
const widths = () => native.width.mock.calls.map(([width]) => width);

describe("auto-hide", () => {
  it("leaves the sidebar alone until it is turned on", () => {
    expect(sidebar.isHidden()).toBe(false);
    expect(sidebar.autoHides()).toBe(false);
    expect(sidebar.effectiveWidth()).toBe(240);
    // The edge is inert: nothing waits there.
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY * 4);
    expect(sidebar.isHidden()).toBe(false);
    expect(widths()).toEqual([]);
  });

  it("hides as a width of nothing, travelling, and saves the choice", () => {
    sidebar.setAutoHide(true);
    expect(sidebar.isHidden()).toBe(true);
    expect(sidebar.effectiveWidth()).toBe(sidebar.HIDDEN_WIDTH);
    expect(native.width).toHaveBeenCalledWith(0, true, expect.any(Number));
    expect(native.set).toHaveBeenCalledWith("sidebar.autohide", "true");

    sidebar.setAutoHide(false);
    expect(sidebar.isHidden()).toBe(false);
    expect(widths()).toEqual([0, 240]);
    expect(native.set).toHaveBeenLastCalledWith("sidebar.autohide", "false");
  });

  it("restores a stored preference without travelling or saving it back", () => {
    sidebar.adoptAutoHide(true);
    expect(widths()).toEqual([0]);
    expect(native.width).toHaveBeenCalledWith(0, false, expect.any(Number));
    expect(native.set).not.toHaveBeenCalled();
  });

  it("hides under the shape it will come back to", () => {
    sidebar.setMode("compact");
    native.width.mockClear();
    sidebar.setAutoHide(true);
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY);
    expect(widths()).toEqual([0, sidebar.COMPACT_WIDTH]);
  });

  it("comes out once the pointer has rested on the edge, not while it crosses", () => {
    sidebar.setAutoHide(true);
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY - 1);
    expect(sidebar.isHidden()).toBe(true);
    // Passing over on the way to the window controls reveals nothing.
    sidebar.pointerLeft();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY * 4);
    expect(sidebar.isHidden()).toBe(true);

    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY);
    expect(sidebar.isHidden()).toBe(false);
    expect(sidebar.effectiveWidth()).toBe(240);
    expect(native.width).toHaveBeenLastCalledWith(240, true, expect.any(Number));
  });

  it("goes back a moment after the pointer leaves, and not if it returns", () => {
    sidebar.setAutoHide(true);
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY);
    sidebar.pointerEnteredSidebar();

    sidebar.pointerLeft();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY - 1);
    expect(sidebar.isHidden()).toBe(false);
    sidebar.pointerEnteredSidebar();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY * 4);
    expect(sidebar.isHidden()).toBe(false);

    sidebar.pointerLeft();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY);
    expect(sidebar.isHidden()).toBe(true);
    expect(widths().at(-1)).toBe(0);
  });

  it("counts the window's very edge as still being on the sidebar", () => {
    sidebar.setAutoHide(true);
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY);
    // Onto the column and back to the edge: one trip, never out of it.
    sidebar.pointerLeft();
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY * 4);
    expect(sidebar.isHidden()).toBe(false);
  });

  it("stays out while something holds it, then goes once that lets go", () => {
    sidebar.setAutoHide(true);
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY);
    const release = sidebar.holdRevealed();

    sidebar.pointerLeft();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY * 5);
    expect(sidebar.isHidden()).toBe(false);

    release();
    release(); // a second release is no release
    vi.advanceTimersByTime(sidebar.HIDE_DELAY);
    expect(sidebar.isHidden()).toBe(true);
  });

  it("two holds need two releases", () => {
    sidebar.setAutoHide(true);
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY);
    const first = sidebar.holdRevealed();
    const second = sidebar.holdRevealed();
    sidebar.pointerLeft();
    first();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY * 3);
    expect(sidebar.isHidden()).toBe(false);
    second();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY);
    expect(sidebar.isHidden()).toBe(true);
  });

  it("stays in view where the column has work to do, and hides again after", () => {
    sidebar.setAutoHide(true);
    expect(sidebar.isHidden()).toBe(true);
    sidebar.setAutoHideApplies(false);
    expect(sidebar.isHidden()).toBe(false);
    expect(sidebar.effectiveWidth()).toBe(240);
    sidebar.setAutoHideApplies(true);
    expect(sidebar.isHidden()).toBe(true);
    // The edge cannot call the column in a place it is already showing.
    sidebar.setAutoHideApplies(false);
    native.width.mockClear();
    sidebar.pointerEnteredEdge();
    vi.advanceTimersByTime(sidebar.REVEAL_DELAY * 2);
    expect(widths()).toEqual([]);
  });

  it("is not hidden while a panel borrows width beside the rail", () => {
    sidebar.setAutoHide(true);
    sidebar.setPanelExtent(264);
    expect(sidebar.isHidden()).toBe(false);
    expect(sidebar.effectiveWidth()).toBe(sidebar.COMPACT_WIDTH + sidebar.PANEL_GAP + 264);
    sidebar.setPanelExtent(0);
    expect(sidebar.isHidden()).toBe(true);
  });

  it("is brought out for as long as a caller needs it, then goes", () => {
    sidebar.setAutoHide(true);
    const restore = sidebar.expandBriefly();
    expect(sidebar.isHidden()).toBe(false);
    // Nobody is on it, but somebody is using it.
    vi.advanceTimersByTime(sidebar.HIDE_DELAY * 5);
    expect(sidebar.isHidden()).toBe(false);
    restore();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY);
    expect(sidebar.isHidden()).toBe(true);
  });

  it("brings out a hidden rail as the full column, and puts both back", () => {
    sidebar.setMode("compact");
    sidebar.setAutoHide(true);
    native.width.mockClear();
    const restore = sidebar.expandBriefly();
    expect(sidebar.sidebarMode()).toBe("default");
    expect(sidebar.isHidden()).toBe(false);
    // Out and wide in one change of width, not out as a rail and then widened.
    expect(widths()).toEqual([240]);
    restore();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY);
    expect(sidebar.sidebarMode()).toBe("compact");
    expect(sidebar.isHidden()).toBe(true);
    expect(widths().at(-1)).toBe(0);
  });

  it("brings a hidden sidebar out for a request and keeps it until answered", () => {
    sidebar.setAutoHide(true);
    native.width.mockClear();
    const answered = sidebar.revealHeld();
    expect(sidebar.isHidden()).toBe(false);
    expect(widths()).toEqual([240]);
    vi.advanceTimersByTime(sidebar.HIDE_DELAY * 5);
    expect(sidebar.isHidden()).toBe(false);
    answered();
    vi.advanceTimersByTime(sidebar.HIDE_DELAY);
    expect(sidebar.isHidden()).toBe(true);

    // With the sidebar already out, asking changes nothing.
    sidebar.setAutoHide(false);
    native.width.mockClear();
    sidebar.revealHeld()();
    expect(widths()).toEqual([]);
  });

  it("ignores a preference echo of its own save, as the shape does", () => {
    sidebar.setAutoHide(true);
    native.width.mockClear();
    // The store reports back what was just saved: nothing to apply.
    sidebar.adoptAutoHide(true);
    expect(widths()).toEqual([]);
    // Later, a change from elsewhere is adopted.
    sidebar.adoptAutoHide(false);
    expect(sidebar.isHidden()).toBe(false);
  });
});
