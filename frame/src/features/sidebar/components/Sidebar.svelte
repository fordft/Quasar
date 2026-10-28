<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import type { Snippet } from "svelte";
  import { IS_MAC, IS_WINDOWS } from "$shared/platform";
  import * as motion from "$session/motion.svelte";
  import { surface as browserPage } from "$domain/surface";
  import * as tools from "$session/tools.svelte";
  import * as tabDrag from "$session/tab-drag.svelte";
  import { tabs } from "$domain/tabs";
  import { untrack } from "svelte";
  import {
    COMPACT_WIDTH,
    PANEL_GAP,
    effectiveWidth,
    holdRevealed,
    isCompact,
    isHidden,
    pointerEnteredEdge,
    pointerEnteredSidebar,
    pointerLeft,
    revealHeld,
    setAutoHideApplies,
    sidebarResizeSettlement,
    toggleAutoHide,
    toggleMode,
  } from "$session/sidebar-mode.svelte";
  import { uiCommands as ui } from "$domain/ui-commands";
  import SidebarHeader from "./SidebarHeader.svelte";
  import * as shapeMorph from "../lib/shape-morph";
  import { duration, reducedMotion } from "$shared/lib/motion";
  import SidebarResizeHandle from "./SidebarResizeHandle.svelte";

  let {
    settingsNavigation,
    toolPanel,
    browserBody,
    dock,
  }: {
    settingsNavigation: Snippet;
    toolPanel: Snippet<[tools.ToolKind]>;
    browserBody: Snippet<[boolean]>;
    dock: Snippet<[boolean]>;
  } = $props();
  let settings = $derived(browserPage.currentPage() === "settings");
  const SETTINGS_PANEL = 264;
  let taskPage = $derived(
    browserPage.currentPage() === "tasks" || browserPage.currentPage() === "notes",
  );
  // Work gives the window to its canvas: the column is always the rail there.
  let inWork = $derived(browserPage.currentPage() === "work");
  let railPage = $derived(taskPage || inWork);
  // Work keeps the column as its compact rail, always: a panel open in Browse
  // closes on the way in, and a tool asked for while in Work opens over its
  // canvas instead of beside it.
  let settledInWork = false;
  $effect(() => {
    const tool = tools.activeTool();
    const work = inWork;
    untrack(() => {
      if (work && tool !== null) {
        tools.close();
        if (settledInWork)
          window.dispatchEvent(new CustomEvent("zephium:work-tool", { detail: tool }));
      }
      settledInWork = work;
    });
  });
  // Settings opens like a tool: the column keeps its tab rail, and its sections
  // stand in the panel beside it while the page shows the section. Tasks and
  // Notes are part of browsing too: a tab is one click away and choosing it
  // returns to that page.
  let navigating = $derived(settings);
  let paneled = $derived(settings || tools.activeTool() !== null);
  let compact = $derived(navigating || railPage || isCompact() || tools.activeTool() !== null);

  // Only plain browsing lets the sidebar hide: Settings, Tasks, Notes, Work and
  // an open tool each keep their place in the column.
  $effect.pre(() => {
    const applies = !navigating && !railPage && tools.activeTool() === null;
    untrack(() => setAutoHideApplies(applies));
  });
  // A tab carried toward the page leaves the sidebar where it is, so it can
  // still be dropped to split a view or kept among the essentials.
  $effect(() => {
    if (tabDrag.draggedId() !== null) return holdRevealed();
  });
  // A page asking something of the person is answered in the address field, so
  // the column comes out for it and stays until it is answered.
  let asking = $derived(tabs.activeTab()?.page_request != null);
  $effect(() => {
    if (asking) return revealHeld();
  });
  // Typing in the column keeps it out, wherever the pointer has gone.
  let editing = $state(false);
  $effect(() => {
    if (editing) return holdRevealed();
  });
  let hidden = $derived(isHidden());
  // Coming out or going away is one change of width, travelled like any other.
  let hideTravel = $state(false);
  let hiddenShown: boolean | null = null;
  let hideTraveled: ReturnType<typeof setTimeout> | undefined;
  $effect(() => () => clearTimeout(hideTraveled));
  $effect.pre(() => {
    const next = hidden;
    untrack(() => {
      if (hiddenShown !== null && hiddenShown !== next && !reducedMotion()) {
        hideTravel = true;
        clearTimeout(hideTraveled);
        hideTraveled = setTimeout(() => (hideTravel = false), duration("page") + 40);
      }
      hiddenShown = next;
    });
  });

  // A change of shape is one continuous change: every mark travels from the
  // old shape into the new one while native slides the page to match. Not
  // on the first shape, which launch brings in with its own cascade.
  let columns = $state<HTMLElement>();
  let shown: boolean | null = null;
  let morph: ReturnType<typeof shapeMorph.capture> = null;
  // While the shape changes, the column's width travels with the page
  // instead of jumping, and its contents hold their final width so only the
  // space beside them moves. A resize guide commits its width immediately on release.
  let reshaping = $state(false);
  let resizeSettling = $state(false);
  let observedResizeSettlement = sidebarResizeSettlement();
  // The header only fades in when it is the other header, not merely beside
  // a column that changed (a tool opening keeps the same header).
  let headerCompact = $derived(compact && !paneled);
  let headerFresh = $state(false);
  let shownHeader: boolean | null = null;
  let reshaped: ReturnType<typeof setTimeout> | undefined;
  $effect(() => () => clearTimeout(reshaped));
  $effect.pre(() => {
    const next = compact;
    const head = headerCompact;
    const settlement = sidebarResizeSettlement();
    untrack(() => {
      // A drag that only changes the width lands at once; one that snaps the
      // column to the other shape travels like any change of shape, since
      // native slides the page with it.
      if (settlement !== observedResizeSettlement && (shown === null || shown === next)) {
        observedResizeSettlement = settlement;
        resizeSettling = true;
        queueMicrotask(() => {
          if (observedResizeSettlement === settlement) resizeSettling = false;
        });
        morph = null;
        reshaping = false;
        headerFresh = false;
        clearTimeout(reshaped);
        shown = next;
        shownHeader = head;
        return;
      }
      observedResizeSettlement = settlement;
      const headChanged = shownHeader !== null && shownHeader !== head;
      shownHeader = head;
      if (shown === null || shown === next) return;
      morph = shapeMorph.capture(columns);
      if (reducedMotion()) return;
      reshaping = true;
      headerFresh = headChanged;
      clearTimeout(reshaped);
      reshaped = setTimeout(
        () => {
          reshaping = false;
          headerFresh = false;
        },
        duration("page") + 40,
      );
    });
  });
  $effect(() => {
    const next = compact;
    untrack(() => {
      if (shown !== null && shown !== next) shapeMorph.play(morph);
      morph = null;
      shown = next;
    });
  });
  let width = $derived(
    navigating
      ? COMPACT_WIDTH + PANEL_GAP + SETTINGS_PANEL
      : railPage && tools.activeTool() === null
        ? COMPACT_WIDTH
        : effectiveWidth(),
  );
  // The column's contents hold the width they have while it travels in or
  // out; what leaves with it is the space beside them. Derived, not stored by
  // an effect: an effect that wrote it while reading the width left the
  // column's own width a step behind.
  let lastOpenWidth = 240;
  let openWidth = $derived.by(() => {
    if (width > 0) lastOpenWidth = width;
    return lastOpenWidth;
  });

  // Dispatch runs untracked and behind a sequence guard. Handlers read the
  // state they mutate (the sidebar shape, the tab projection), so a tracked
  // effect would subscribe to its own writes and re-fire on the same command,
  // which made the collapsed menu's Compact Mode toggle immediately undo
  // itself.
  // Whether the pointer is over the column, for what shows only while it is.
  // Tracked rather than left to :hover, which a move straight onto a page's
  // own view can leave standing; leaving the document or the window losing
  // focus ends it too.
  let pointerIn = $state(false);
  $effect(() => {
    const out = () => {
      pointerIn = false;
      pointerLeft();
    };
    document.documentElement.addEventListener("mouseleave", out);
    window.addEventListener("blur", out);
    return () => {
      document.documentElement.removeEventListener("mouseleave", out);
      window.removeEventListener("blur", out);
    };
  });

  let handledCommand = 0;
  $effect(() => {
    const command = ui.uiCommand();
    if (command.seq === 0 || command.seq === handledCommand) return;
    handledCommand = command.seq;

    // Changed after this pass rather than inside it, as a click would: a
    // shape changed mid-pass re-renders before the old shape is measured,
    // so nothing could travel from it.
    if (command.id === "sidebar.toggleCompact") queueMicrotask(toggleShape);
    // Only macOS keeps its window controls outside the sidebar, so only there
    // can it go away.
    if (command.id === "sidebar.toggleAutoHide" && IS_MAC) queueMicrotask(toggleAutoHide);
  });

  function toggleShape() {
    if (tools.activeTool() !== null) tools.close();
    else toggleMode();
  }
</script>

<!-- Where a hidden sidebar waits: the window's gutter, which chrome keeps when
     the column is gone. Resting the pointer on it brings the sidebar back. -->
<div
  class="sidebar-edge"
  aria-hidden="true"
  onpointerenter={pointerEnteredEdge}
  onpointerleave={pointerLeft}
></div>
<aside
  data-tauri-drag-region="deep"
  aria-label={m.ui_browser_sidebar()}
  style:width={`${width}px`}
  style:--sidebar-width={`${width}px`}
  style:--sidebar-open-width={`${openWidth}px`}
  data-menu-material={IS_WINDOWS && !settings ? "opaque" : undefined}
  data-reshaping={reshaping}
  data-hidden={hidden}
  data-hide-travel={hideTravel}
  data-sidebar-resize-settling={resizeSettling}
  data-header-fresh={headerFresh}
  data-pointer-in={pointerIn}
  inert={hidden || undefined}
  onfocusin={(event) =>
    (editing =
      event.target instanceof Element &&
      event.target.matches("input, textarea, [contenteditable]:not([contenteditable=false])"))}
  onfocusout={() => (editing = false)}
  onpointerenter={() => {
    pointerIn = true;
    pointerEnteredSidebar();
  }}
  onpointerleave={() => {
    pointerIn = false;
    pointerLeft();
  }}
  class="browser-sidebar relative flex shrink-0 flex-col text-text select-none"
>
  {#if !navigating && !railPage && tools.activeTool() === null && !hidden}<SidebarResizeHandle
      {width}
      disabled={reshaping}
    />{/if}
  <!-- Settings stands beside the rail with no head: its panel runs the full
       height and the rail keeps only the lights' row. A tool keeps the head,
       with search and the page's own controls. -->
  {#if !settings}<SidebarHeader
      compact={headerCompact}
      launcher={paneled}
      ontoggle={toggleShape}
      pageControls={!inWork}
    />{/if}
  <div
    bind:this={columns}
    class="sidebar-columns"
    data-glide-host
    data-launch={motion.launchState()}
  >
    {#key compact}<div class="sidebar-browser-column" class:sidebar-tool-rail={paneled}>
        {#if settings && IS_MAC}<div class="sidebar-lights" aria-hidden="true"></div>{/if}
        {@render browserBody(compact)}
        {@render dock(compact)}
      </div>{/key}
    {#if settings}<div class="sidebar-tool-host">
        {@render settingsNavigation()}
      </div>{:else if tools.activeTool() !== null}<div class="sidebar-tool-host">
        {@render toolPanel(tools.activeTool()!)}
      </div>{/if}
  </div>
</aside>

<style>
  /* A panel open in the column (the space switcher, the tray on the address,
     the profile's stack) is the one thing to read: whatever it lies over
     steps back under it, and comes forward again when it closes. */
  :global([data-under-panel]) {
    transition: opacity var(--motion-base) var(--ease-out);
  }

  :global(
    .browser-sidebar:has(.sidebar-softens[data-open="true"])
      [data-under-panel]:not(:has(.sidebar-softens[data-open="true"]))
  ) {
    opacity: 0.18;
  }

  @media (prefers-reduced-motion: reduce) {
    :global([data-under-panel]) {
      transition: none;
    }
  }
</style>
