import "$styles/global.css";
import { expect, test } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import PageCard from "../components/cards/PageCard.svelte";
import type { CanvasItem } from "../lib/canvas-model";

const GIF = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

function card(frame: string | null): CanvasItem {
  return {
    id: "page",
    type: "page",
    kind: "Page",
    title: "shop.example",
    detail: "https://shop.example/p/1",
    status: "Read",
    page: { url: "https://shop.example/p/1", host: "shop.example", frame, live: false },
  };
}

test("a settled page draws its frame on first mount and follows it when it changes", async () => {
  await page.viewport(1200, 800);
  // Served files: a frame is drawn at its seen size, asked for by width.
  const first = "/quasar-icon@2x.png";
  const second = "/quasar-icon@2x.png?generation=2";
  const screen = await render(PageCard, { item: card(first), selected: false });
  const drawn = () => screen.container.querySelector<HTMLCanvasElement>(".frame canvas");
  expect(screen.container.querySelector(".frame img")).toBeNull();
  await expect.poll(() => drawn()?.width ?? 0).toBeGreaterThan(0);
  await screen.rerender({ item: card(second), selected: false });
  await expect.poll(() => drawn()?.width ?? 0).toBeGreaterThan(0);
  // Nothing recorded yet: the site's mark when the cache holds one, never a broken image,
  // a stock globe or an initial.
  await screen.rerender({ item: card(null), selected: false });
  expect(screen.container.querySelector(".frame canvas")).toBeNull();
  expect(
    screen.container.querySelector(".placeholder [data-favicon] .favicon-fallback"),
  ).toBeNull();
  expect(screen.container.querySelector(".placeholder")?.textContent?.trim()).toBe("");
  await screen.unmount();
});

test("a page the run is holding says why it needs you, and the countdown only near the end", async () => {
  await page.viewport(1200, 800);
  const waiting = card(GIF);
  waiting.status = "Waiting for you";
  waiting.page!.human = {
    attempt: "attempt",
    step: "read",
    generation: 2,
    phase: "waiting_for_human",
    reason: "sign_in",
    remaining: 120_000,
    canContinue: false,
    canExtend: false,
    workMemory: false,
  };
  const screen = await render(PageCard, { item: waiting, selected: false });
  // What to do and where, action first.
  await expect.element(screen.getByText("shop.example needs you signed in")).toBeVisible();
  expect(screen.container.querySelector(".needs .left")).toBeNull();
  await screen.rerender({
    item: {
      ...waiting,
      page: { ...waiting.page!, human: { ...waiting.page!.human!, remaining: 24_000 } },
    },
    selected: false,
  });
  await expect.element(screen.getByText("0:24 left")).toBeVisible();
  // Presented: the header carries that state and the card stops asking.
  await screen.rerender({
    item: {
      ...waiting,
      status: "Yours right now",
      page: { ...waiting.page!, human: { ...waiting.page!.human!, phase: "presented" } },
    },
    selected: false,
  });
  expect(screen.container.querySelector(".needs")).toBeNull();
  await expect.element(screen.getByText("Yours right now")).toBeVisible();
  // Handed back: the card is an ordinary page again.
  await screen.rerender({ item: card(GIF), selected: false });
  expect(screen.container.querySelector(".needs")).toBeNull();
  await screen.unmount();
});
