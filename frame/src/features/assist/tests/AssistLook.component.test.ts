import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import type { PageAssistMessageV1, PageAssistStatusV1 } from "$shared/ipc/bindings";
import { PROFILE } from "$shared/testing/work-models";
import { emitNativeEvent } from "$shared/testing/native-events";
import PanelStage from "./PanelStage.svelte";
import { setTabs, tab } from "./tabs-stub.svelte";
import {
  HANDOFF,
  ITEMS,
  OPENED,
  SPACE,
  TAB,
  event,
  person,
  reply,
  resetSeq,
  status,
  text,
} from "./fixtures";

const native = vi.hoisted(() => ({ status: null as PageAssistStatusV1 | null }));

vi.mock("$domain/tabs", async () => ({ tabs: (await import("./tabs-stub.svelte")).tabs }));
vi.mock("$domain/surface", () => ({ surface: { open: async () => true } }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  const { models } = await import("$shared/testing/work-models");
  const { accountFixture } = await import("$shared/testing/account");
  return mockBindings({
    pageAssistStatus: async () => native.status!,
    pageAssistClose: async () => true,
    pageAssistFocus: async () => true,
    pageStyles: async () => ({
      version: 1,
      tab: "tab-1",
      site: "ycombinator.com",
      styles: [{ id: "s1", summary: "Stories as cards", enabled: true }],
    }),
    workModels: async () => {
      const view = models({
        keys: { open_ai: "valid" },
        lead: "openai/gpt-6-luna",
        effort: "high",
      });
      return { ...view, effective: { ...view.effective, page: "openai/gpt-6-luna" } };
    },
    accountGet: async () => ({ account: accountFixture(), fault: null, retry_after: null }),
    settingGet: async () => null,
    runCommand: async () => ({ accepted: true, operation_id: null }),
  });
});

beforeEach(() => {
  resetSeq();
  setTabs({
    tabs: [tab(TAB, "https://news.ycombinator.com/", "Hacker News")],
    active: TAB,
    space: SPACE,
  });
});
afterEach(() => {
  delete document.documentElement.dataset.theme;
});

// The native glass is not in WebKit: a blurred wallpaper and the effect view's tint stand in for it.
function glass(theme: "light" | "dark") {
  document.getElementById("wallpaper")?.remove();
  const ground = document.createElement("div");
  ground.id = "wallpaper";
  ground.style.cssText =
    "position:fixed;inset:0;z-index:-1;background:radial-gradient(90% 80% at 20% 15%, #8f7fd6 0%, #4b3f86 40%, #1d1838 75%)";
  const frost = document.createElement("div");
  frost.style.cssText = `position:absolute;inset:0;backdrop-filter:blur(40px) saturate(1.6);background:${
    theme === "light"
      ? "linear-gradient(rgb(245 245 247 / 59%), rgb(245 245 247 / 59%)), rgb(255 255 255 / 30%)"
      : "linear-gradient(rgb(23 23 26 / 47%), rgb(23 23 26 / 47%)), rgb(0 0 0 / 12%)"
  }`;
  ground.append(frost);
  document.body.prepend(ground);
}

const settled = () =>
  Promise.all(
    document
      .getAnimations()
      .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
      .map((animation) => animation.finished.catch(() => undefined)),
  );

async function capture(container: HTMLElement, name: string) {
  for (const theme of ["dark", "light"]) {
    document.documentElement.dataset.theme = theme;
    await settled();
    await page
      .elementLocator(container.firstElementChild as HTMLElement)
      .screenshot({ path: `../../../../../target/look/assist/${name}-${theme}.png` });
  }
}

async function open(next: PageAssistStatusV1, height = 760) {
  native.status = next;
  await page.viewport(400, height + 40);
  const screen = await render(PanelStage, { profile: PROFILE, height });
  await expect.element(screen.getByRole("textbox", { name: "Ask Quasar…" })).toBeVisible();
  return screen;
}

const CITES = (() => {
  const block = text("", [1, 2]);
  return block.kind === "text" ? block.cites : [];
})();

const thread = (...messages: PageAssistMessageV1[]) => status(messages);

test("empty: a greeting for the page in front and at most four starts", async () => {
  const screen = await open(status());
  await expect.element(screen.getByText("What can I do on Hacker News?")).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Worth opening" })).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Redesign this page" })).toBeVisible();
  await expect
    .element(screen.getByRole("button", { name: "Make it darker" }))
    .not.toBeInTheDocument();
  await expect.element(screen.getByRole("button", { name: /Model: GPT-6 Luna/u })).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Dictate" })).toBeVisible();
  await capture(screen.container, "empty");
  await screen.getByRole("button", { name: "Worth opening" }).hover();
  await capture(screen.container, "empty-hover");
});

test("empty over the glass sidebar", async () => {
  const screen = await open(status());
  await expect.element(screen.getByText("What can I do on Hacker News?")).toBeVisible();
  document.documentElement.dataset.material = "liquid_glass";
  try {
    for (const theme of ["dark", "light"] as const) {
      document.documentElement.dataset.theme = theme;
      glass(theme);
      await settled();
      await page.screenshot({ path: `../../../../../target/look/assist/glass-${theme}.png` });
    }
  } finally {
    delete document.documentElement.dataset.material;
    document.getElementById("wallpaper")?.remove();
  }
});

test("empty, typing: Send takes the mic's place", async () => {
  const screen = await open(status());
  await screen.getByRole("textbox").hover();
  await screen.getByRole("textbox").fill("Which of these is worth an hour tonight?");
  await expect.element(screen.getByRole("button", { name: "Send" })).toBeVisible();
  await capture(screen.container, "typing");
});

test("empty on a page it does not read: says what it can still do", async () => {
  const screen = await open(status([], { availability: "sensitive", page: null }));
  await expect.element(screen.getByText("Quasar stays out of this site")).toBeVisible();
  await expect.element(screen.getByText(/You can still ask anything/u)).toBeVisible();
  await capture(screen.container, "away");
});

test("streaming: text grows with numbered cites under one working line", async () => {
  const screen = await open(
    thread(
      person(1, "What's worth reading here today?"),
      reply(2, [], {
        state: "working",
        phase: { phase: "reading", site: "news.ycombinator.com" },
        cost: null,
      }),
    ),
  );
  await expect.element(screen.getByText("Reading news.ycombinator.com…").first()).toBeVisible();
  emitNativeEvent("pageAssistChanged", event(2, { type: "phase", phase: { phase: "thinking" } }));
  emitNativeEvent(
    "pageAssistChanged",
    event(2, {
      type: "text",
      block: 0,
      delta: "Three stories stand out today. The **top one** is ",
    }),
  );
  emitNativeEvent(
    "pageAssistChanged",
    event(2, {
      type: "text",
      block: 0,
      delta: "about a certificate authority ⟦1⟧, and two are `Show HN` posts ⟦2⟧.",
    }),
  );
  emitNativeEvent("pageAssistChanged", event(2, { type: "cites", block: 0, cites: CITES }));
  await expect.element(screen.getByRole("button", { name: /Source 1/u })).toBeVisible();
  await expect.element(screen.getByText("Thinking…").first()).toBeVisible();
  expect(screen.container.querySelectorAll(".orb")).toHaveLength(1);
  await screen.getByRole("button", { name: /Source 1/u }).hover();
  await expect.element(screen.getByRole("tooltip")).toHaveTextContent(/On news\.ycombinator\.com/u);
  await capture(screen.container, "streaming");
});

test("items: numbered cards, the unmarked one outlined", async () => {
  const screen = await open(
    thread(
      person(1, "Which stories are worth opening?"),
      reply(2, [text("These three are worth your time ⟦1⟧.", [1]), ITEMS]),
    ),
  );
  await expect.element(screen.getByRole("list", { name: "3 items" })).toBeVisible();
  await capture(screen.container, "items");
});

test("opened: one inset checklist, one moving thing", async () => {
  const screen = await open(
    thread(
      person(1, "Open them for me"),
      reply(2, [text("Opening the three in the background."), OPENED], {
        state: "working",
        phase: { phase: "opening" },
        cost: null,
      }),
    ),
  );
  await expect.element(screen.getByText("Couldn’t open")).toBeVisible();
  expect(screen.container.querySelector(".working")).toBeNull();
  expect(screen.container.querySelectorAll(".orb")).toHaveLength(1);
  await capture(screen.container, "opened");
});

test("confirm: the exact control, a quiet Not now, and what it does", async () => {
  const screen = await open(
    thread(
      person(1, "Upvote the first one"),
      reply(
        2,
        [
          {
            kind: "steps",
            steps: [
              {
                index: 0,
                label: "Scroll to “eMudhra to exit”",
                category: null,
                state: "done",
                reason: null,
              },
              {
                index: 1,
                label: "Click “upvote”",
                category: "save",
                state: "confirm",
                reason: null,
              },
              {
                index: 2,
                label: "Check the score",
                category: null,
                state: "skipped",
                reason: null,
              },
            ],
          },
        ],
        { state: "waiting", cost: null },
      ),
    ),
  );
  await expect.element(screen.getByRole("button", { name: "upvote", exact: true })).toBeVisible();
  await expect
    .element(screen.getByText("This saves a change on news.ycombinator.com."))
    .toBeVisible();
  await capture(screen.container, "confirm");
});

test("steps: each names its control; a failed one says why", async () => {
  const screen = await open(
    thread(
      person(1, "Search the site for Terax"),
      reply(
        2,
        [
          {
            kind: "steps",
            steps: [
              {
                index: 0,
                label: "Type “Terax” into “Search”",
                category: null,
                state: "done",
                reason: null,
              },
              {
                index: 1,
                label: "Press Return in “Search”",
                category: null,
                state: "failed",
                reason: "no_effect",
              },
              {
                index: 2,
                label: "Open “Terax — release notes”",
                category: null,
                state: "skipped",
                reason: null,
              },
            ],
          },
        ],
        { state: "failed", cost: null, fault: { reason: "step_failed" } },
      ),
    ),
  );
  await expect.element(screen.getByText("It ran, but nothing changed")).toBeVisible();
  await capture(screen.container, "steps");
});

test("style: a proposal, then the applied badge with its switches", async () => {
  const proposal = (state: "proposed" | "applied") => ({
    kind: "style" as const,
    style: {
      id: state === "applied" ? "s1" : null,
      summary: "Stories as cards",
      site: "ycombinator.com",
      rules: 14,
      state,
    },
  });
  const screen = await open(
    status(
      [
        person(1, "Make the front page easier to scan"),
        reply(2, [text("Here is a calmer layout."), proposal("applied")]),
        person(3, "And a darker one?"),
        reply(4, [proposal("proposed")], { state: "waiting", cost: null }),
      ],
      { styled: true },
    ),
  );
  await expect.element(screen.getByRole("button", { name: "Apply" })).toBeVisible();
  await expect.element(screen.getByText("Style on for ycombinator.com")).toBeVisible();
  await capture(screen.container, "style");
});

test("handoff: the hero card in each of its states", async () => {
  for (const [name, state] of [
    ["offered", { state: "offered" }],
    ["starting", { state: "starting" }],
    ["started", { state: "started", work: "w1" }],
    ["dismissed", { state: "dismissed" }],
  ] as const) {
    const screen = await open(
      thread(
        person(1, "Set up the status page for the next release"),
        reply(2, [
          text("This reaches past the browser, so Work is the better place."),
          HANDOFF(state),
        ]),
      ),
      820,
    );
    if (name === "offered")
      await expect.element(screen.getByRole("button", { name: "Continue in Work" })).toBeVisible();
    if (name === "starting")
      await expect.element(screen.getByText("Starting Work…").first()).toBeVisible();
    if (name === "started")
      await expect.element(screen.getByRole("button", { name: "Open in Work" })).toBeVisible();
    if (name === "dismissed")
      await expect.element(screen.getByText(/Not handed to Work/u)).toBeVisible();
    await capture(screen.container, `handoff-${name}`);
    screen.unmount();
  }
});

test("faults: a rate limit counts down, credits lead to the account", async () => {
  const screen = await open(
    thread(
      person(1, "Summarize this"),
      reply(2, [], {
        state: "failed",
        cost: null,
        fault: { reason: "model", fault: "rate_limited", retry_after_ms: 12_000 },
      }),
      person(3, "Try again"),
      reply(4, [], {
        state: "failed",
        cost: null,
        fault: { reason: "model", fault: "hosted_usage", retry_after_ms: null },
      }),
    ),
  );
  await expect.element(screen.getByRole("button", { name: "Buy credits" })).toBeVisible();
  await expect.element(screen.getByText("Zephium AI needs a short breather.")).toBeVisible();
  await capture(screen.container, "faults");
});

test("composer: page and selection chips; + holds the page, the picker and other tabs", async () => {
  setTabs({
    tabs: [
      tab(TAB, "https://news.ycombinator.com/", "Hacker News"),
      tab("tab-2", "https://github.com/zephium/zephium", "zephium/zephium · GitHub"),
      tab("tab-3", "https://developer.mozilla.org/", "MDN Web Docs"),
    ],
  });
  const screen = await open(status([], { selection: "The quick brown fox" }));
  await expect.element(screen.getByRole("button", { name: "Selection" })).toBeVisible();
  await expect
    .element(screen.getByRole("button", { name: "Remove news.ycombinator.com" }))
    .toBeVisible();
  await capture(screen.container, "composer");
  for (const theme of ["dark", "light"]) {
    document.documentElement.dataset.theme = theme;
    await screen.getByRole("button", { name: "Add" }).click();
    await expect.element(page.getByRole("menuitem", { name: "Pick an element" })).toBeVisible();
    await settled();
    await page.screenshot({ path: `../../../../../target/look/assist/add-${theme}.png` });
    await page.getByRole("menuitem", { name: "Attach another tab" }).click();
    await expect.element(page.getByRole("menuitem", { name: /MDN Web Docs/u })).toBeVisible();
    await settled();
    await page.screenshot({ path: `../../../../../target/look/assist/add-tabs-${theme}.png` });
    await userEvent.keyboard("{Escape}");
    await expect.element(page.getByRole("menu")).not.toBeInTheDocument();
  }
});
