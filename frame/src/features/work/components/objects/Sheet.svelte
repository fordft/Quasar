<script lang="ts">
  import DataTable, {
    type TableColumn,
    type TableRow,
    type TableSort,
  } from "$shared/ui/data/DataTable";
  import * as m from "$shared/i18n/messages";
  import type { ObjectActions, SheetView } from "../../lib/board/types";
  import {
    bests,
    cellOrder,
    figure,
    filled,
    fitColumns,
    host,
    numeric,
    pickOf,
    rating,
    yesNo,
  } from "./sheet";
  import { versus } from "../../lib/board/versus";
  import { vendorHost } from "../../lib/vendors";
  import Title from "./Title.svelte";
  import Mark, { hasMark } from "./Mark.svelte";
  import QuasarMark, { isQuasar } from "./QuasarMark.svelte";
  import YesNo from "./YesNo.svelte";
  import Dots from "./Dots.svelte";
  let {
    object,
    actions = {},
    rows: shown = 8,
    centre = false,
  }: {
    object: SheetView;
    actions?: ObjectActions;
    /** Rows the canvas shows before the rest are read in the centre. */
    rows?: number;
    /** Opened in the centre: every row, every column. */
    centre?: boolean;
  } = $props();
  const first = $derived(object.columns[0]);
  /** A few named things compared: they stand as columns, the measures read down. */
  const facing = $derived(versus(object));
  const pick = $derived(facing ? pickOf(object) : null);
  const best = $derived(bests(object));
  let width = $state(0);
  /** Every column once the person asks for them; they scroll inside the sheet. */
  let wide = $state(false);
  /** The columns that say something; one empty in every row is left out. */
  const said = $derived(
    object.columns.map((_, index) => index).filter((index) => index > 0 && filled(object, index)),
  );
  /** Those that carry most in the width the sheet stands at. */
  const fitted = $derived(width ? fitColumns(object, width - 44) : said);
  const kept = $derived(centre || wide ? said : fitted);
  const hidden = $derived(said.length - fitted.length);
  const columns = $derived<TableColumn[]>(
    kept.map((at) => {
      const column = object.columns[at]!;
      return {
        key: String(at),
        label:
          column.unit && column.kind !== "money"
            ? `${column.label} (${column.unit})`
            : column.label,
        numeric: numeric(column),
        centered: column.kind === "yes_no" || column.kind === "rating",
        sortable: object.rows.length > 2 && column.kind !== "link",
      };
    }),
  );
  const rows = $derived<TableRow[]>(
    object.rows.map((row, index) => ({
      key: String(index),
      label: row.cells[0] ?? "",
      cells: Object.fromEntries(row.cells.map((cell, at) => [String(at), cell])),
      sort: Object.fromEntries(
        object.columns.map((column, at) => [String(at), cellOrder(column, row.cells[at] ?? "")]),
      ),
    })),
  );
  let sort = $state<TableSort | null>(null);
  const labels = $derived({
    rowHeading: first?.label ?? "",
    actions: m.work_actions(),
    empty: m.work_empty_data(),
    missing: "",
    unavailable: m.work_artifact_unavailable(),
    previous: m.work_previous(),
    next: m.work_next(),
    range: (from: number, to: number, total: number) =>
      m.work_table_range({ first: from, last: to, total }),
  });
  /**
   * The row's own logo, else a known product its name is, else the site of
   * the page it was read from: the first whose mark is at hand, or the first
   * at all, so its mark is asked for.
   */
  function logo(index: number): string | null {
    const row = object.rows[index];
    const cited = (row?.sources ?? []).flatMap((key) => {
      const url = object.sources?.[key]?.url;
      return url ? [url] : [];
    });
    const candidates = [
      row?.entity?.logo,
      ...(first?.kind === "entity" ? [vendorHost(undefined, row?.cells[0] ?? "")] : []),
      ...(first?.kind === "entity" ? cited : []),
    ].filter((entry): entry is string => !!entry);
    return candidates.find((entry) => hasMark(entry)) ?? candidates[0] ?? null;
  }
  const limit = $derived(centre ? object.rows.length : shown);
  /** Where the columns run on past the sheet's edge, each side. */
  let more = $state({ before: false, after: false });
  let table = $state<HTMLElement>();
  function measure(scroller: Element | null | undefined) {
    if (!(scroller instanceof HTMLElement)) return;
    const end = scroller.scrollWidth - scroller.clientWidth;
    more = { before: scroller.scrollLeft > 1, after: end - scroller.scrollLeft > 1 };
  }
  $effect(() => {
    void kept;
    void width;
    const scroller = table?.querySelector(".table-scroll");
    const frame = requestAnimationFrame(() => measure(scroller));
    return () => cancelAnimationFrame(frame);
  });
</script>

{#snippet subject(index: number, size: number)}
  {@const row = object.rows[index]}
  {#if row?.entity?.picture}<span class="picture"
      ><img
        src={row.entity.picture.src}
        alt=""
        decoding="async"
        loading="lazy"
        width={size + 8}
        height={size + 8}
      /></span
    >{:else if isQuasar(row?.cells[0] ?? "")}<QuasarMark {size} />{:else if logo(index)}<Mark
      address={logo(index)!}
      {size}
    />{/if}
{/snippet}

{#snippet value(index: number, at: number)}
  {@const column = object.columns[at]!}
  {@const text = object.rows[index]?.cells[at] ?? ""}
  {@const marked = best[at]?.has(String(index))}
  {#if !text.trim() && column.kind !== "yes_no"}<span class="none" title={m.work_object_unknown()}
      >–</span
    >
  {:else if column.kind === "yes_no"}<YesNo value={yesNo(text)} size={16} />
  {:else if column.kind === "rating"}
    {@const score = rating(text)}
    {#if score}<Dots
        value={score.value}
        max={score.max}
        label={m.work_object_rating({ value: score.value, max: score.max })}
      />{:else}<span class="text" title={text}>{text}</span>{/if}
  {:else if column.kind === "entity"}
    {@const known = vendorHost(undefined, text)}
    <span class="subject"
      >{#if isQuasar(text)}<QuasarMark size={16} />{:else if known}<Mark
          address={known}
          size={16}
        />{/if}<span class="name">{text}</span></span
    >
  {:else if column.kind === "tag"}{#if text.trim()}<span class="tag">{text}</span>{/if}
  {:else if column.kind === "link"}
    <a
      class="link nodrag"
      href={text}
      onclick={(event) => {
        if (!actions.link) return;
        event.preventDefault();
        actions.link(text);
      }}><Mark address={text} size={14} /><span>{host(text)}</span></a
    >
  {:else if numeric(column)}<span class="figure" class:best={marked}>{figure(column, text)}</span>
  {:else}<span class="text" title={text}>{text}</span>{/if}
{/snippet}

{#if facing}
  <section class="sheet versus" class:centre aria-label={object.title}>
    {#if object.title}<Title text={object.title} />{/if}
    <div class="grid" role="table" aria-label={object.title} style:--columns={object.rows.length}>
      <div class="line contenders" role="row">
        <span class="measure" role="columnheader"></span>
        {#each object.rows as row, index (index)}<div
            class="contender"
            class:pick={pick === index}
            role="columnheader"
          >
            <span class="face">{@render subject(index, 28)}</span>
            <span class="who">{row.cells[0] ?? ""}</span>
            {#if pick === index}<span class="pick-tag">{m.work_sheet_pick()}</span>{/if}
          </div>{/each}
      </div>
      {#each said as at (at)}{@const column = object.columns[at]!}
        <div class="line" role="row">
          <span class="measure" role="rowheader"
            >{column.unit && column.kind !== "money"
              ? `${column.label} (${column.unit})`
              : column.label}</span
          >
          {#each object.rows as _, index (index)}<span
              class="value"
              class:pick={pick === index}
              class:centred={column.kind === "yes_no" || column.kind === "rating"}
              role="cell">{@render value(index, at)}</span
            >{/each}
        </div>{/each}
    </div>
    {#if object.note}<footer><p class="note">{object.note}</p></footer>{/if}
  </section>
{:else}
  <section class="sheet" class:centre aria-label={object.title} bind:clientWidth={width}>
    {#if object.title}<Title text={object.title} />{/if}
    <div
      class="nodrag table"
      class:before={more.before}
      class:after={more.after}
      class:nowheel={more.before || more.after}
      bind:this={table}
      onscrollcapture={(event) => measure(event.target as Element)}
    >
      <DataTable
        caption={object.title ?? ""}
        showCaption={false}
        {columns}
        {rows}
        {labels}
        plain
        headSortable={object.rows.length > 2}
        bind:sort
        {limit}
      >
        {#snippet head(row)}
          <span class="subject"
            >{@render subject(Number(row.key), 18)}<span class="name">{row.label}</span></span
          >
        {/snippet}
        {#snippet cell(row, column)}{@render value(Number(row.key), Number(column.key))}{/snippet}
      </DataTable>
    </div>
    {#if object.rows.length > limit || object.note || (hidden > 0 && !centre)}
      <footer>
        {#if object.note}<p class="note">{object.note}</p>{/if}
        <span class="more">
          {#if hidden > 0 && !centre}<button
              type="button"
              class="all nodrag nopan"
              aria-expanded={wide}
              onclick={() => (wide = !wide)}
              >{wide
                ? m.work_sheet_fewer_columns()
                : hidden === 1
                  ? m.work_sheet_more_column()
                  : m.work_sheet_more_columns({ count: hidden })}</button
            >{/if}
          {#if object.rows.length > limit}<button
              type="button"
              class="all nodrag nopan"
              onclick={() => actions.open?.(object.id)}
              >{m.work_object_show_all({ count: object.rows.length })}</button
            >{/if}
        </span>
      </footer>
    {/if}
  </section>
{/if}

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 14px;
    box-sizing: border-box;
    inline-size: 100%;
    padding: 20px 22px 16px;
    border-radius: var(--radius-card);
    background: var(--color-surface);
    box-shadow: var(--shadow-raised);
    color: var(--color-text);
    font-size: var(--text-body);
  }

  .subject {
    display: inline-flex;
    align-items: center;
    gap: 10px;
  }

  /* A name wraps between its words, never inside one. */
  .name {
    overflow-wrap: normal;
    text-wrap: pretty;
  }

  .picture {
    display: inline-flex;
    flex: none;
    overflow: hidden;
    border-radius: var(--radius-inset);
    background: var(--color-fill);
  }

  /* A row thumbnail is a few dozen pixels: a photo fills its square, whatever it shows. */
  .picture img {
    display: block;
    object-fit: cover;
  }

  .text {
    min-inline-size: 12ch;
    max-inline-size: 34ch;
    color: var(--color-label-secondary);
    overflow-wrap: break-word;
    text-wrap: pretty;
    display: -webkit-box;
    overflow: hidden;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 4;
    line-clamp: 4;
  }

  /* Columns that run on past the edge fade into it; the subject column stays. */
  .table.after :global(.table-scroll) {
    mask-image: linear-gradient(to left, transparent, black 48px);
  }

  .table.before :global(tbody th),
  .table.before :global(thead th:first-child) {
    box-shadow: 1px 0 0 var(--color-border);
  }

  .table :global(thead th:first-child) {
    position: sticky;
    inset-inline-start: 0;
    z-index: 1;
    background: var(--color-surface);
  }

  .figure {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  /* The best figure is set heavier, in place: every figure keeps one shape. */
  .figure.best {
    font-weight: 650;
  }

  /* Nothing comparable to say: a quiet dash, never a blank a reader takes for a gap. */
  .none {
    color: var(--color-faint);
  }

  .tag {
    display: inline-block;
    padding: 2px 8px;
    border-radius: var(--radius-capsule);
    background: var(--color-fill);
    color: var(--color-label-secondary);
    font-size: var(--text-label);
    white-space: nowrap;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--color-label-secondary);
    text-decoration: none;
  }

  .link:hover span {
    color: var(--color-text);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  footer {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
  }

  .more {
    display: flex;
    flex: none;
    gap: 16px;
    margin-inline-start: auto;
  }

  .note {
    margin: 0;
    color: var(--color-faint);
    font-size: var(--text-caption);
  }

  .all {
    flex: none;
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-muted);
    font: inherit;
    font-size: var(--text-label);
    font-weight: 500;
    cursor: default;
  }

  .all:hover {
    color: var(--color-text);
  }

  /* Contenders as columns under their marks, the measures read down, the pick on a soft ground. */
  .grid {
    display: flex;
    flex-direction: column;
  }

  .line {
    display: grid;
    grid-template-columns: 144px repeat(var(--columns), minmax(0, 1fr));
    column-gap: 4px;
  }

  .contenders {
    align-items: end;
  }

  .contender {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 12px 12px 14px;
    border-start-start-radius: var(--radius-row);
    border-start-end-radius: var(--radius-row);
  }

  .face {
    display: flex;
    align-items: center;
    min-block-size: 28px;
  }

  .face :global(.picture) {
    border-radius: var(--radius-inset);
  }

  .who {
    font-size: var(--text-reading);
    font-weight: 650;
    line-height: 1.25;
    letter-spacing: -0.01em;
    text-wrap: balance;
  }

  .pick-tag {
    color: var(--color-term);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .measure {
    padding: 12px 12px 12px 0;
    color: var(--color-muted);
    font-size: var(--text-label);
    font-weight: 500;
    line-height: 18px;
  }

  .value {
    min-inline-size: 0;
    padding: 12px;
    color: var(--color-label-secondary);
    line-height: 18px;
    overflow-wrap: anywhere;
    text-wrap: pretty;
  }

  .line + .line .measure,
  .line + .line .value {
    border-block-start: 1px solid var(--color-border);
  }

  .value.centred {
    display: flex;
    align-items: flex-start;
  }

  .pick {
    background: var(--color-term-wash);
  }

  .grid > .line:last-child .value.pick {
    border-end-start-radius: var(--radius-row);
    border-end-end-radius: var(--radius-row);
  }

  .versus footer {
    padding-block-start: 2px;
  }
</style>
