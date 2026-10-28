import { commands } from "$shared/ipc/bindings";

export type SidebarMode = "default" | "compact";

/** The rail width. Native admits it as a real sidebar width, not a mode flag. */
export const COMPACT_WIDTH = 56;
export const MIN_EXPANDED_WIDTH = 180;
export const MAX_EXPANDED_WIDTH = 460;

/**
 * Dragging the handle below this snaps to the rail, and dragging back out of
 * the rail snaps to the minimum expanded width. Nothing settles in the gap,
 * so the sidebar is always in one of its two designed shapes.
 */
export const SNAP_THRESHOLD = 140;

const MODE_SETTING = "sidebar.mode";

let mode = $state.raw<SidebarMode>("default");
let desiredMode: SidebarMode = "default";
let expandedWidth = $state.raw(240);
let drag: { mode: SidebarMode; desired: SidebarMode; expanded: number; pending: number } | null =
  null;
let resizeSettlement = $state(0);
// One document-local ordering counter, seeded above earlier document owners.
let resizeRevision = $state(Math.trunc(performance.timeOrigin * 1024));
export const sidebarResizeRevision = () => resizeRevision;
export function claimSidebarResizeOwner() {
  return ++resizeRevision;
}
export const sidebarResizeActive = () => drag !== null;
export const sidebarResizeSettlement = () => resizeSettlement;

export const sidebarMode = () => mode;
export const isCompact = () => mode === "compact";
export const expanded = () => expandedWidth;
/** The chrome gap between the rail and an open utility panel. */
export const PANEL_GAP = 8;
let panelExtent = $state.raw(0);

/** The width native admits for a sidebar that is out of the way. */
export const HIDDEN_WIDTH = 0;

export const effectiveWidth = () =>
  hidden()
    ? HIDDEN_WIDTH
    : panelExtent > 0
      ? COMPACT_WIDTH + PANEL_GAP + panelExtent
      : mode === "compact"
        ? COMPACT_WIDTH
        : expandedWidth;

export const hasPanel = () => panelExtent > 0;

/** A utility panel borrows width beside the rail without changing the persisted mode. */
export function setPanelExtent(extent: number) {
  const next = Math.max(0, Math.min(MAX_EXPANDED_WIDTH - COMPACT_WIDTH - PANEL_GAP, extent));
  if (next === panelExtent) return;
  panelExtent = next;
  publish(true);
}

function clampExpanded(value: number) {
  return Math.max(MIN_EXPANDED_WIDTH, Math.min(MAX_EXPANDED_WIDTH, Math.round(value)));
}

/** Resolves a raw drag width to whichever designed shape it lands in. */
export function resolveDragWidth(value: number): { mode: SidebarMode; expanded: number } {
  if (!Number.isFinite(value) || value < SNAP_THRESHOLD) {
    return { mode: "compact", expanded: expandedWidth };
  }
  return { mode: "default", expanded: clampExpanded(value) };
}

/**
 * Tells native the column's width. `travel` marks a deliberate change of
 * shape — a toggle, a snap, a tool opening — which the page slides with;
 * a drag in progress or a restored preference moves it without ceremony.
 */
function publish(travel = false) {
  resizeRevision++;
  if (drag) cancelSidebarResize();
  publishedHidden = hidden();
  const width = effectiveWidth();
  void commands.sidebarSetWidth(width, travel, resizeRevision);
}

// ── Auto-hide ────────────────────────────────────────────────────────────
// The sidebar can step out of the way. It is a width of nothing, not a drawn
// state: native gives the page the room, chrome keeps only the window's
// gutter, and moving the pointer onto that gutter brings the sidebar back
// until the pointer leaves. Chrome and page tile, never overlap, so the page
// is pushed aside while the sidebar is out, as it is for any change of width.

const AUTOHIDE_SETTING = "sidebar.autohide";
/** Rest on the edge this long before the sidebar comes out, so crossing it
 *  on the way to the window controls does not. */
export const REVEAL_DELAY = 120;
/** Leave the sidebar this long before it goes back, so a stray move off
 *  the column does not. */
export const HIDE_DELAY = 450;

/** The preference. */
let autoHide = $state.raw(false);
/** Whether this page and these tools let the sidebar hide at all: Settings,
 *  Tasks, Notes, Work and an open tool panel each live in the column. */
let autoHideApplies = $state.raw(true);
/** The sidebar out for now, though the preference is on. */
let revealed = $state.raw(false);
let publishedHidden = false;
let pointerInside = false;
let holds = 0;
let revealTimer: ReturnType<typeof setTimeout> | undefined;
let hideTimer: ReturnType<typeof setTimeout> | undefined;

const hidden = () => autoHide && autoHideApplies && !revealed && panelExtent === 0;
/** The sidebar is out of the way right now. */
export const isHidden = hidden;
/** The preference is on, whether or not the sidebar is out at the moment. */
export const autoHides = () => autoHide;

function clearTimers() {
  clearTimeout(revealTimer);
  clearTimeout(hideTimer);
}

/** Publishes the width when hiding or revealing changed it. */
function settleHidden(travel: boolean) {
  if (hidden() !== publishedHidden) publish(travel);
}

function applyAutoHide(next: boolean, travel: boolean) {
  if (next === autoHide) return;
  autoHide = next;
  clearTimers();
  revealed = false;
  pointerInside = false;
  settleHidden(travel);
}

let savingAutoHide: { value: boolean; timer: ReturnType<typeof setTimeout> } | null = null;

function saveAutoHide(next: boolean) {
  if (savingAutoHide) clearTimeout(savingAutoHide.timer);
  savingAutoHide = { value: next, timer: setTimeout(() => (savingAutoHide = null), 2000) };
  void commands.settingSet(AUTOHIDE_SETTING, String(next));
}

/** A stored preference reached this column: from here, or from elsewhere. */
export function adoptAutoHide(next: boolean) {
  if (savingAutoHide) {
    if (next === savingAutoHide.value) {
      clearTimeout(savingAutoHide.timer);
      savingAutoHide = null;
    }
    return;
  }
  // A restored preference lands without ceremony; native lays out once.
  applyAutoHide(next, false);
}

export function setAutoHide(next: boolean) {
  if (next === autoHide) return;
  applyAutoHide(next, true);
  saveAutoHide(next);
}

export function toggleAutoHide() {
  setAutoHide(!autoHide);
}

/** Page and tools tell whether the sidebar may hide; back from a place it
 *  could not, it stays out only if the pointer is still on it. */
export function setAutoHideApplies(applies: boolean) {
  if (applies === autoHideApplies) return;
  autoHideApplies = applies;
  clearTimers();
  revealed = applies && pointerInside;
  settleHidden(true);
}

function reveal() {
  if (revealed) return;
  revealed = true;
  settleHidden(true);
}

function scheduleHide() {
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => {
    if (pointerInside) return;
    // Something is using the sidebar: look again shortly.
    if (holds > 0) return scheduleHide();
    revealed = false;
    settleHidden(true);
  }, HIDE_DELAY);
}

/** The pointer reached the window's edge, where a hidden sidebar waits. */
export function pointerEnteredEdge() {
  pointerInside = true;
  clearTimeout(hideTimer);
  if (!autoHide || !autoHideApplies || revealed || panelExtent > 0) return;
  clearTimeout(revealTimer);
  revealTimer = setTimeout(reveal, REVEAL_DELAY);
}

/** The pointer is on the sidebar itself. */
export function pointerEnteredSidebar() {
  pointerInside = true;
  clearTimeout(hideTimer);
}

/** The pointer left the sidebar and its edge, or the window lost it. */
export function pointerLeft() {
  pointerInside = false;
  clearTimeout(revealTimer);
  if (revealed) scheduleHide();
}

/**
 * Keeps a revealed sidebar out while something needs it: a tab being dragged
 * toward the page, a field being used. Returns the release.
 */
export function holdRevealed(): () => void {
  holds++;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holds--;
    if (holds === 0 && revealed && !pointerInside) scheduleHide();
  };
}

/**
 * Brings a hidden sidebar out for as long as something there needs an answer:
 * a request from a page, a permission prompt. Nothing changes when the sidebar
 * is already out. Returns the release.
 */
export function revealHeld(): () => void {
  const release = holdRevealed();
  if (hidden()) {
    clearTimers();
    revealed = true;
    settleHidden(true);
  }
  return release;
}

export function applyDragWidth(value: number) {
  if (drag) {
    drag.pending = value;
    return;
  }
  const next = resolveDragWidth(value);
  if (next.mode === mode && next.expanded === expandedWidth) return;
  const modeChanged = next.mode !== mode;
  mode = next.mode;
  expandedWidth = next.expanded;
  desiredMode = mode;
  publish(modeChanged);
  if (modeChanged) save(mode);
}

/** The non-native capture fallback also keeps the rendered layout unchanged. */
export function beginSidebarResize() {
  if (drag) return;
  drag = { mode, desired: desiredMode, expanded: expandedWidth, pending: effectiveWidth() };
}

export function finishSidebarResize(value: number) {
  if (!drag) return;
  drag = null;
  const before = mode;
  adoptResizeWidth(value);
  // Only a change of shape travels; the column animates its width then too.
  void commands.sidebarSetWidth(effectiveWidth(), mode !== before, resizeRevision);
}

export function cancelSidebarResize() {
  drag = null;
}

/** Native has admitted one final width; adopt its display shape without another layout command. */
export function adoptResizeWidth(value: number, revision = resizeRevision) {
  if (revision !== resizeRevision) return;
  if (!Number.isFinite(value)) return;
  // A fast drag overshoots the bounds; it lands on the nearest one.
  const next = resolveDragWidth(Math.max(COMPACT_WIDTH, Math.min(MAX_EXPANDED_WIDTH, value)));
  const changed = next.mode !== mode;
  mode = next.mode;
  desiredMode = mode;
  expandedWidth = next.expanded;
  resizeSettlement++;
  if (changed) save(mode);
}

let resizeListenerInstalled = false;
function installResizeListener() {
  if (resizeListenerInstalled) return;
  resizeListenerInstalled = true;
  window.addEventListener("zephium:sidebar-width-selected", (event) => {
    if (!(event instanceof CustomEvent) || !event.detail || typeof event.detail !== "object")
      return;
    const { width, revision } = event.detail;
    if (typeof width === "number" && typeof revision === "number")
      adoptResizeWidth(width, revision);
  });
}

// The shape this column last saved, until the store reports it back. Values
// arriving meanwhile are the store catching up with saves made here, not a
// change from elsewhere: adopting them would snap the column back to a shape
// it has already left. A save whose report never comes stops counting.
let saving: { mode: SidebarMode; timer: ReturnType<typeof setTimeout> } | null = null;

function save(next: SidebarMode) {
  if (saving) clearTimeout(saving.timer);
  saving = { mode: next, timer: setTimeout(() => (saving = null), 2000) };
  void commands.settingSet(MODE_SETTING, next);
}

/** A stored preference reached this column: from here, or from elsewhere. */
export function adoptMode(next: SidebarMode) {
  if (saving) {
    if (next === saving.mode) {
      clearTimeout(saving.timer);
      saving = null;
    }
    return;
  }
  cancelSidebarResize();
  if (next === mode) return;
  mode = next;
  desiredMode = next;
  publish();
}

export function setMode(next: SidebarMode) {
  cancelSidebarResize();
  if (next === mode) return;
  mode = next;
  desiredMode = next;
  publish(true);
  save(next);
}

/**
 * Shows the full column for a moment without touching the preference. The
 * returned function puts the rail back, unless a shape was chosen meanwhile.
 */
export function expandBriefly(): () => void {
  cancelSidebarResize();
  const wasHidden = hidden();
  const wasCompact = mode === "compact";
  if (wasCompact) mode = "default";
  // A hidden sidebar comes out for as long as the caller needs it, in the
  // shape it is being given: one change of width, not two.
  const release = wasHidden ? revealHeld() : () => {};
  if (!wasCompact) return release;
  if (!wasHidden) publish(true);
  return () => {
    release();
    if (mode !== "default" || desiredMode !== "compact") return;
    cancelSidebarResize();
    mode = "compact";
    publish(true);
  };
}

export function toggleMode() {
  cancelSidebarResize();
  desiredMode = desiredMode === "compact" ? "default" : "compact";
  setMode(desiredMode);
}

/**
 * Adopts the persisted mode. The width is published afterwards so native
 * layout matches the restored shape on the first paint rather than after the
 * first interaction.
 */
export async function init(hideSupported = false): Promise<void> {
  installResizeListener();
  try {
    const [stored, hide] = await Promise.all([
      commands.settingGet(MODE_SETTING),
      commands.settingGet(AUTOHIDE_SETTING).catch(() => null),
    ]);
    if (stored === "compact") {
      mode = "compact";
      desiredMode = "compact";
    }
    if (hide === "true" && hideSupported) autoHide = true;
  } catch {
    // A missing or unreadable preference is not a startup failure; the
    // default shape is always valid.
  }
  publish();
}
