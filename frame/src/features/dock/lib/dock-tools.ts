import * as m from "$shared/i18n/messages";
import {
  HourglassIcon,
  CheckListIcon,
  Download01Icon,
  HistoryIcon,
  Bookmark02Icon,
  Note01Icon,
  BubbleChatIcon,
} from "@hugeicons/core-free-icons";
import type { IconSvgElement } from "@hugeicons/svelte";
import type { ToolKind } from "$session/tools.svelte";

/**
 * How a tool is drawn. The art matches the tool panel's own header, so a
 * shelf glyph and the panel it opens can never disagree about what a tool is.
 */
const PRESENTATION: Record<ToolKind, { icon: IconSvgElement; label: () => string }> = {
  notes: { icon: Note01Icon, label: m.panel_notes },
  tasks: { icon: CheckListIcon, label: m.panel_tasks },
  time: { icon: HourglassIcon, label: m.panel_time },
  ai: { icon: BubbleChatIcon, label: m.panel_ask_zephium },
  history: { icon: HistoryIcon, label: m.menu_history },
  downloads: { icon: Download01Icon, label: m.menu_downloads },
  bookmarks: { icon: Bookmark02Icon, label: m.tool_bookmarks },
};

export const toolPresentation = (kind: ToolKind) => PRESENTATION[kind];

/**
 * The shelf's two groups, in the native menu's own order so the stack and the
 * menu never disagree: first the things you make, then what you keep and what
 * happened. Settings closes the second group as a destination of its own.
 */
export const SHELF_TOOLS = ["notes", "tasks", "time"] as const satisfies readonly ToolKind[];
/** Quasar reads the page in front of you, so it leads the things you make, where AI is on and the window is not private. */
export const ASK_TOOL = "ai" as const satisfies ToolKind;
export const RECORD_TOOLS = [
  "history",
  "downloads",
  "bookmarks",
] as const satisfies readonly ToolKind[];
