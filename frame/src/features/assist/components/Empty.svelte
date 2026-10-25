<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import QuasarTile from "$shared/ui/QuasarTile";
  import {
    commands,
    type PageAssistStatusV1,
    type PageAssistSuggestionV1,
  } from "$shared/ipc/bindings";
  import { availabilityView } from "../lib/faults";
  import { siteName, startsFor, suggestionGlyph, suggestionLabel } from "../lib/suggestions";
  import { ArrowRight01Icon, InformationCircleIcon } from "../lib/icons";
  import "./assist.css";

  let {
    status,
    onsuggest,
    onexit,
  }: {
    status: PageAssistStatusV1;
    onsuggest: (suggestion: PageAssistSuggestionV1) => void;
    /** Arrow Down past the last start: back to the field. */
    onexit: () => void;
  } = $props();

  const page = $derived(status.availability === "ready" ? status.page : null);
  const away = $derived(
    status.availability === "ready" ? null : availabilityView(status.availability),
  );
  const starts = $derived(page ? startsFor(status.suggestions) : []);
  const greeting = $derived(
    page
      ? m.assist_greeting_page({ site: siteName(page.title, page.host) })
      : away && !away.talks
        ? away.title
        : m.assist_greeting(),
  );

  function onkeydown(event: KeyboardEvent) {
    const rows = [
      ...(event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>("[data-start]"),
    ];
    const at = rows.indexOf(document.activeElement as HTMLElement);
    if (at === -1) return;
    const next =
      event.key === "ArrowDown"
        ? at + 1
        : event.key === "ArrowUp"
          ? Math.max(0, at - 1)
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? rows.length - 1
              : null;
    if (next === null) return;
    event.preventDefault();
    if (next >= rows.length) onexit();
    else rows[next]?.focus();
  }
</script>

<div class="start">
  <span class="mark"><QuasarTile size={36} /></span>
  <h3 class="greeting">{greeting}</h3>
  {#if starts.length}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <ul class="starts" aria-label={greeting} {onkeydown}>
      {#each starts as suggestion, index (suggestion)}
        <li class="assist-rise" style:animation-delay="{40 + index * 30}ms">
          <button type="button" class="row" data-start onclick={() => onsuggest(suggestion)}>
            <span class="glyph" aria-hidden="true"
              ><Icon icon={suggestionGlyph(suggestion)} size={16} strokeWidth={1.6} /></span
            >
            <span class="label">{suggestionLabel(suggestion)}</span>
            <span class="go" aria-hidden="true"
              ><Icon icon={ArrowRight01Icon} size={14} strokeWidth={1.8} /></span
            >
          </button>
        </li>
      {/each}
    </ul>
  {:else if away}
    {#if away.talks}
      <p class="aside">
        <Icon icon={InformationCircleIcon} size={14} strokeWidth={1.7} /><span
          ><span class="why">{away.title}.</span> {m.assist_still_ask()}</span
        >
      </p>
    {:else}
      <p class="aside help">{away.help}</p>
      {#if away.fix}{@const fix = away.fix}
        <div class="fix">
          <Button
            size="compact"
            onclick={() => void commands.runCommand(fix.command).catch(() => {})}
            >{fix.label}</Button
          >
        </div>{/if}
    {/if}
  {/if}
</div>

<style>
  /* The start: a calm composition standing on the composer, read top to bottom
     as mark, one line of greeting, and what Quasar can begin with. */
  .start {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    margin-block-start: auto;
    padding: 0 2px 8px;
  }

  .mark {
    display: block;
    inline-size: 36px;
    block-size: 36px;
    margin-block-end: 14px;
  }

  .greeting {
    margin: 0;
    color: var(--color-text);
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.022em;
    line-height: 26px;
    text-wrap: balance;
  }

  .starts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    align-self: stretch;
    margin: 16px -8px 0;
    padding: 0;
    list-style: none;
  }

  .row {
    --shine-radius: var(--radius-row);

    display: flex;
    align-items: center;
    gap: 11px;
    box-sizing: border-box;
    inline-size: 100%;
    block-size: 36px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--radius-row);
    background: transparent;
    color: var(--color-label-secondary);
    font: inherit;
    font-size: 13.5px;
    font-weight: 500;
    letter-spacing: -0.004em;
    text-align: start;
    cursor: default;
    transition:
      background-color var(--motion-fast) var(--ease-out),
      box-shadow var(--motion-fast) var(--ease-out),
      color var(--motion-fast) var(--ease-out),
      scale var(--motion-base) var(--ease-spring);
  }

  .glyph {
    display: grid;
    flex: none;
    place-items: center;
    color: var(--color-muted);
    transition: color var(--motion-fast) var(--ease-out);
  }

  .label {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .go {
    display: grid;
    flex: none;
    place-items: center;
    color: var(--color-faint);
    opacity: 0;
    translate: -4px 0;
    transition:
      opacity var(--motion-fast) var(--ease-out),
      translate var(--motion-base) var(--ease-out);
  }

  .row:is(:hover, :focus-visible) {
    background: var(--row-active);
    box-shadow: var(--shine-top);
    color: var(--color-text);
  }

  .row:is(:hover, :focus-visible) .glyph {
    color: var(--color-text);
  }

  .row:is(:hover, :focus-visible) .go {
    opacity: 1;
    translate: 0 0;
  }

  .row:active {
    background: var(--row-pressed);
    scale: 0.985;
    transition-duration: var(--motion-instant);
  }

  .row:focus-visible {
    outline: 2px solid var(--color-ring);
    outline-offset: -2px;
  }

  .aside {
    display: flex;
    gap: 7px;
    margin: 10px 0 0;
    color: var(--color-muted);
    font-size: 13px;
    line-height: 19px;
    text-wrap: pretty;
  }

  .aside > :global(svg) {
    flex: none;
    margin-block-start: 2.5px;
    color: var(--color-faint);
  }

  .why {
    color: var(--color-label-secondary);
  }

  .help {
    margin-block-start: 8px;
  }

  .fix {
    margin-block-start: 14px;
  }

  @media (prefers-reduced-motion: reduce) {
    .row,
    .go {
      transition: none;
    }
  }

  :global(:root[data-reduce-motion="true"]) :is(.row, .go) {
    transition: none;
  }
</style>
