import { beforeEach, expect, test, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import type {
  PageAssistAskResponseV1,
  PageAssistPickResponseV1,
  PageAssistStatusV1,
} from "$shared/ipc/bindings";
import { PROFILE } from "$shared/testing/work-models";
import { emitNativeEvent } from "$shared/testing/native-events";
import PanelStage from "./PanelStage.svelte";
import { setTabs, tab } from "./tabs-stub.svelte";
import {
  CONVERSATION,
  HANDOFF,
  ITEMS,
  SPACE,
  TAB,
  event,
  person,
  reply,
  resetSeq,
  status,
  text,
} from "./fixtures";

const native = vi.hoisted(() => ({
  status: null as PageAssistStatusV1 | null,
  statusCalls: 0,
  ask: vi.fn(async (): Promise<PageAssistAskResponseV1> => ({ reply: 2, fault: null })),
  fresh: vi.fn(async () => true),
  stop: vi.fn(async () => true),
  focus: vi.fn(async () => true),
  decide: vi.fn(async () => true),
  continueWork: vi.fn(async () => ({ work: "w1", fault: null })),
  pick: vi.fn(async (): Promise<PageAssistPickResponseV1> => ({ pick: null, fault: null })),
  pickCancel: vi.fn(async () => true),
  detach: vi.fn(async () => true),
  close: vi.fn(async () => true),
  history: vi.fn(async () => ({
    version: 2,
    conversations: [
      {
        id: "c0",
        title: "Pricing on the Pro plan",
        space: "space-1",
        updated_ms: Date.now() - 3_600_000,
      },
    ],
  })),
  open: vi.fn(async () => true),
  remove: vi.fn(async () => true),
  url: vi.fn(async () => ({ accepted: true, operation_id: null })),
  surface: vi.fn(async () => true),
}));

vi.mock("$domain/tabs", async () => ({ tabs: (await import("./tabs-stub.svelte")).tabs }));
vi.mock("$domain/surface", () => ({ surface: { open: native.surface } }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  const { models } = await import("$shared/testing/work-models");
  const { accountFixture } = await import("$shared/testing/account");
  return mockBindings({
    pageAssistStatus: async () => {
      native.statusCalls++;
      return native.status!;
    },
    pageAssistAsk: native.ask,
    pageAssistNew: native.fresh,
    pageAssistStop: native.stop,
    pageAssistFocus: native.focus,
    pageAssistDecide: native.decide,
    pageAssistContinue: native.continueWork,
    pageAssistPick: native.pick,
    pageAssistPickCancel: native.pickCancel,
    pageAssistDetach: native.detach,
    pageAssistClose: native.close,
    pageAssistHistory: native.history,
    pageAssistOpen: native.open,
    pageAssistDelete: native.remove,
    browserOpenUrl: native.url,
    workModels: async () => {
      const view = models({ keys: { open_ai: "valid" }, lead: "openai/gpt-6-luna" });
      return { ...view, effective: { ...view.effective, page: "openai/gpt-6-luna" } };
    },
    accountGet: async () => ({ account: accountFixture(), fault: null, retry_after: null }),
    settingGet: async () => null,
    runCommand: async () => ({ accepted: true, operation_id: null }),
  });
});

beforeEach(() => {
  vi.clearAllMocks();
  native.statusCalls = 0;
  resetSeq();
  setTabs({
    tabs: [tab(TAB, "https://news.ycombinator.com/", "Hacker News")],
    active: TAB,
    space: SPACE,
  });
});

async function open(next: PageAssistStatusV1) {
  native.status = next;
  await page.viewport(400, 820);
  const screen = await render(PanelStage, { profile: PROFILE });
  const field = screen.getByRole("textbox", { name: "Ask Quasar…" });
  await expect.element(field).toBeVisible();
  await expect.poll(() => native.statusCalls).toBeGreaterThan(0);
  return { screen, field };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("Enter sends with the page in front; Shift+Enter only adds a line; × leaves the page out", async () => {
  const { screen, field } = await open(status());
  await expect.element(field).toHaveFocus();
  await field.fill("What's worth reading?");
  await userEvent.keyboard("{Shift>}{Enter}{/Shift}");
  expect(native.ask).not.toHaveBeenCalled();
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  expect(native.ask).toHaveBeenLastCalledWith({
    space: SPACE,
    conversation: CONVERSATION,
    text: "What's worth reading?",
    suggestion: null,
    attachments: [{ kind: "page", tab: TAB }],
  });
  await expect.element(field).toHaveValue("");

  await screen.getByRole("button", { name: "Remove news.ycombinator.com" }).click();
  await field.fill("Just between us");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(2);
  expect(native.ask).toHaveBeenLastCalledWith(expect.objectContaining({ attachments: [] }));
});

test("typing the next draft and attaching an element during admission survives the earlier send", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValueOnce(admission.promise);
  native.pick.mockResolvedValueOnce({
    pick: { tab: TAB, pick: "next", label: "Next element", host: "news.ycombinator.com" },
    fault: null,
  });
  const { screen, field } = await open(status());
  await field.fill("First draft");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  await expect.element(field).not.toBeDisabled();
  await field.fill("Next draft");
  // Even returning to the submitted text is a new edit, not the submitted draft.
  await field.fill("First draft");
  await screen.getByRole("button", { name: "Add" }).click();
  await page.getByRole("menuitem", { name: "Pick an element" }).click();
  await expect.element(screen.getByText("Next element")).toBeVisible();
  admission.resolve({ reply: 2, fault: null });
  await new Promise((done) => setTimeout(done, 0));
  await expect.element(screen.getByRole("button", { name: "Send" })).not.toBeDisabled();
  await expect.element(field).toHaveValue("First draft");
  await expect.element(screen.getByText("Next element")).toBeVisible();
});

test("a delayed admission in Space A cannot clear the same text now shown in Space B", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValueOnce(admission.promise);
  const { screen, field } = await open(status());
  await field.fill("A draft to keep");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  native.status = status([], {
    space: "space-b",
    tab: "tab-b",
    page: { tab: "tab-b", host: "example.com", title: "Space B page" },
    conversation: { id: "conversation-b", messages: [], title: null, seq: 0 },
  });
  setTabs({
    space: "space-b",
    active: "tab-b",
    tabs: [tab("tab-b", "https://example.com/", "Space B page")],
  });
  await expect.element(screen.getByRole("button", { name: "Remove example.com" })).toBeVisible();
  await screen.getByRole("button", { name: "Remove example.com" }).click();
  admission.resolve({ reply: 2, fault: null });
  await new Promise((done) => setTimeout(done, 0));
  await expect.element(screen.getByRole("button", { name: "Send" })).not.toBeDisabled();
  await expect.element(field).toHaveValue("A draft to keep");
  await expect
    .element(screen.getByRole("button", { name: "Remove example.com" }))
    .not.toBeInTheDocument();
});

test("starting a fresh conversation invalidates the old draft acknowledgement immediately", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  const fresh = deferred<boolean>();
  native.ask.mockReturnValueOnce(admission.promise);
  native.fresh.mockReturnValueOnce(fresh.promise);
  const { screen, field } = await open(status([person(1, "Earlier"), reply(2, [text("Done")])]));
  await field.fill("Keep this draft");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  await screen.getByRole("button", { name: "New conversation", exact: true }).click();
  await expect.poll(() => native.fresh.mock.calls.length).toBe(1);
  admission.resolve({ reply: 4, fault: null });
  await new Promise((done) => setTimeout(done, 0));
  await expect.element(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  await expect.element(field).toHaveValue("Keep this draft");
  fresh.resolve(false);
});

test.each([
  ["fresh", "before"],
  ["fresh", "after"],
  ["history", "before"],
  ["history", "after"],
] as const)(
  "a refused %s switch clears the unchanged admitted draft when admission arrives %s refusal",
  async (choice, order) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const switching = deferred<boolean>();
    native.ask.mockReturnValueOnce(admission.promise);
    if (choice === "fresh") native.fresh.mockReturnValueOnce(switching.promise);
    else native.open.mockReturnValueOnce(switching.promise);
    native.pick.mockResolvedValueOnce({
      pick: { tab: TAB, pick: "A", label: "Submitted element", host: "news.ycombinator.com" },
      fault: null,
    });
    const { screen, field } = await open(status([person(1, "Earlier")]));
    await screen.getByRole("button", { name: "Add" }).click();
    await page.getByRole("menuitem", { name: "Pick an element" }).click();
    await expect.element(screen.getByText("Submitted element")).toBeVisible();
    await field.fill("Admitted once");
    await userEvent.keyboard("{Enter}");
    await expect.poll(() => native.ask.mock.calls.length).toBe(1);
    if (choice === "fresh") {
      await screen.getByRole("button", { name: "New conversation", exact: true }).click();
      await expect.poll(() => native.fresh.mock.calls.length).toBe(1);
    } else {
      await screen.getByRole("button", { name: "More" }).click();
      await page.getByRole("menuitem", { name: "History" }).click();
      await screen.getByRole("button", { name: /^Pricing on the Pro plan/u }).click();
      await expect.poll(() => native.open.mock.calls.length).toBe(1);
    }
    if (order === "before") {
      admission.resolve({ reply: 2, fault: null });
      await new Promise((done) => setTimeout(done, 0));
      await expect.element(field).toHaveValue("Admitted once");
    }
    switching.resolve(false);
    if (order === "after") admission.resolve({ reply: 2, fault: null });
    await expect.element(field).toHaveValue("");
    await expect.element(screen.getByText("Submitted element")).not.toBeInTheDocument();
  },
);

test.each(["fresh", "history"] as const)(
  "reconciling a refused %s switch preserves a newer draft edit and element",
  async (choice) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const switching = deferred<boolean>();
    native.ask.mockReturnValueOnce(admission.promise);
    if (choice === "fresh") native.fresh.mockReturnValueOnce(switching.promise);
    else native.open.mockReturnValueOnce(switching.promise);
    const { screen, field } = await open(status([person(1, "Earlier")]));
    await field.fill("Admitted once");
    await userEvent.keyboard("{Enter}");
    await expect.poll(() => native.ask.mock.calls.length).toBe(1);
    if (choice === "fresh") {
      await screen.getByRole("button", { name: "New conversation", exact: true }).click();
      await expect.poll(() => native.fresh.mock.calls.length).toBe(1);
    } else {
      await screen.getByRole("button", { name: "More" }).click();
      await page.getByRole("menuitem", { name: "History" }).click();
      await screen.getByRole("button", { name: /^Pricing on the Pro plan/u }).click();
      await expect.poll(() => native.open.mock.calls.length).toBe(1);
    }
    await field.fill("Next draft");
    await field.fill("Admitted once");
    native.pick.mockResolvedValueOnce({
      pick: { tab: TAB, pick: "B", label: "Next element", host: "news.ycombinator.com" },
      fault: null,
    });
    await screen.getByRole("button", { name: "Add" }).click();
    await page.getByRole("menuitem", { name: "Pick an element" }).click();
    await expect.element(screen.getByText("Next element")).toBeVisible();
    admission.resolve({ reply: 2, fault: null });
    await new Promise((done) => setTimeout(done, 0));
    switching.resolve(false);
    await new Promise((done) => setTimeout(done, 0));
    await expect.element(field).toHaveValue("Admitted once");
    await expect.element(screen.getByText("Next element")).toBeVisible();
    await expect.element(screen.getByRole("button", { name: "Send" })).not.toBeDisabled();
  },
);

test("a refused open cannot reconcile an admitted draft or a new pick in another Space", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  const switching = deferred<boolean>();
  native.ask.mockReturnValueOnce(admission.promise);
  native.open.mockReturnValueOnce(switching.promise);
  const { screen, field } = await open(status());
  await field.fill("Same draft");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  await screen.getByRole("button", { name: "More" }).click();
  await page.getByRole("menuitem", { name: "History" }).click();
  await screen.getByRole("button", { name: /^Pricing on the Pro plan/u }).click();
  await expect.poll(() => native.open.mock.calls.length).toBe(1);
  admission.resolve({ reply: 2, fault: null });
  await new Promise((done) => setTimeout(done, 0));
  native.status = status([], {
    space: "space-b",
    conversation: {
      id: "conversation-b",
      messages: [person(1, "Space B history")],
      title: null,
      seq: 0,
    },
  });
  setTabs({
    space: "space-b",
    active: TAB,
    tabs: [tab(TAB, "https://news.ycombinator.com/", "Hacker News")],
  });
  await expect.element(screen.getByText("Space B history")).toBeVisible();
  native.pick.mockResolvedValueOnce({
    pick: { tab: TAB, pick: "A", label: "Space B element", host: "news.ycombinator.com" },
    fault: null,
  });
  await screen.getByRole("button", { name: "Add" }).click();
  await page.getByRole("menuitem", { name: "Pick an element" }).click();
  await expect.element(screen.getByText("Space B element")).toBeVisible();
  switching.resolve(false);
  await new Promise((done) => setTimeout(done, 0));
  await expect.element(field).toHaveValue("Same draft");
  await expect.element(screen.getByText("Space B element")).toBeVisible();
});

test("an open projected before its acknowledgement retires the old submitted pick and keeps the new composer", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  const switching = deferred<boolean>();
  native.ask.mockReturnValueOnce(admission.promise);
  native.open.mockReturnValueOnce(switching.promise);
  native.pick.mockResolvedValueOnce({
    pick: { tab: TAB, pick: "A", label: "Submitted element", host: "news.ycombinator.com" },
    fault: null,
  });
  const { screen, field } = await open(status());
  await screen.getByRole("button", { name: "Add" }).click();
  await page.getByRole("menuitem", { name: "Pick an element" }).click();
  await expect.element(screen.getByText("Submitted element")).toBeVisible();
  await field.fill("Same draft");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  await screen.getByRole("button", { name: "More" }).click();
  await page.getByRole("menuitem", { name: "History" }).click();
  await screen.getByRole("button", { name: /^Pricing on the Pro plan/u }).click();
  await expect.poll(() => native.open.mock.calls.length).toBe(1);
  native.status = status([], {
    conversation: { id: "c0", messages: [person(1, "Opened history")], title: null, seq: 0 },
  });
  const reads = native.statusCalls;
  emitNativeEvent("pageAssistChanged", {
    ...event(0, { type: "invalidated" }),
    conversation: "c0",
  });
  await expect.poll(() => native.statusCalls).toBeGreaterThan(reads);
  switching.resolve(true);
  await expect.element(screen.getByText("Opened history")).toBeVisible();
  native.pick.mockResolvedValueOnce({
    pick: { tab: TAB, pick: "B", label: "Next element", host: "news.ycombinator.com" },
    fault: null,
  });
  await screen.getByRole("button", { name: "Add" }).click();
  await page.getByRole("menuitem", { name: "Pick an element" }).click();
  await expect.element(screen.getByText("Next element")).toBeVisible();
  admission.resolve({ reply: 2, fault: null });
  await expect.element(screen.getByText("Submitted element")).not.toBeInTheDocument();
  await expect.element(screen.getByText("Next element")).toBeVisible();
  await expect.element(field).toHaveValue("Same draft");
});

test.each(["success", "fault"] as const)(
  "a retained submitted pick disables Send until its delayed admission %s releases it",
  async (outcome) => {
    const admission = deferred<PageAssistAskResponseV1>();
    native.ask.mockReturnValueOnce(admission.promise);
    native.pick.mockResolvedValueOnce({
      pick: { tab: TAB, pick: "A", label: "Pending element", host: "news.ycombinator.com" },
      fault: null,
    });
    const { screen, field } = await open(status());
    await screen.getByRole("button", { name: "Add" }).click();
    await page.getByRole("menuitem", { name: "Pick an element" }).click();
    await expect.element(screen.getByText("Pending element")).toBeVisible();
    await field.fill("Original draft");
    await userEvent.keyboard("{Enter}");
    await expect.poll(() => native.ask.mock.calls.length).toBe(1);
    native.status = status([], {
      conversation: { id: "c0", messages: [person(1, "Opened history")], title: null, seq: 0 },
    });
    await screen.getByRole("button", { name: "More" }).click();
    await page.getByRole("menuitem", { name: "History" }).click();
    await screen.getByRole("button", { name: /^Pricing on the Pro plan/u }).click();
    await expect.element(screen.getByText("Opened history")).toBeVisible();
    await field.fill("Next draft");
    native.pick.mockResolvedValueOnce({
      pick: { tab: TAB, pick: "B", label: "Next element", host: "news.ycombinator.com" },
      fault: null,
    });
    await screen.getByRole("button", { name: "Add" }).click();
    await page.getByRole("menuitem", { name: "Pick an element" }).click();
    await expect.element(screen.getByText("Next element")).toBeVisible();
    await expect.element(screen.getByText("Pending element")).toBeVisible();
    await expect.element(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    await field.click();
    await userEvent.keyboard("{Enter}");
    expect(native.ask).toHaveBeenCalledOnce();
    admission.resolve(
      outcome === "success"
        ? { reply: 2, fault: null }
        : { reply: null, fault: { reason: "busy" } },
    );
    await expect.element(screen.getByRole("button", { name: "Send" })).not.toBeDisabled();
    await expect.element(field).toHaveValue("Next draft");
    if (outcome === "success")
      await expect.element(screen.getByText("Pending element")).not.toBeInTheDocument();
    else await expect.element(screen.getByText("Pending element")).toBeVisible();
    await userEvent.keyboard("{Enter}");
    await expect.poll(() => native.ask.mock.calls.length).toBe(2);
    expect(native.ask).toHaveBeenLastCalledWith(
      expect.objectContaining({
        conversation: "c0",
        attachments: [
          { kind: "page", tab: TAB },
          ...(outcome === "fault" ? [{ kind: "element", tab: TAB, pick: "A" }] : []),
          { kind: "element", tab: TAB, pick: "B" },
        ],
      }),
    );
  },
);

test.each(["first", "last"] as const)(
  "a superseded successful history open acknowledged %s preserves the draft and projects native history",
  async (order) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const x = deferred<boolean>();
    const y = deferred<boolean>();
    native.ask.mockReturnValueOnce(admission.promise);
    native.open.mockReturnValueOnce(x.promise).mockReturnValueOnce(y.promise);
    native.history.mockResolvedValueOnce({
      version: 2,
      conversations: [
        { id: "X", title: "History X", space: SPACE, updated_ms: Date.now() },
        { id: "Y", title: "History Y", space: SPACE, updated_ms: Date.now() },
      ],
    });
    const { screen, field } = await open(status());
    await field.fill("Original draft");
    await userEvent.keyboard("{Enter}");
    await expect.poll(() => native.ask.mock.calls.length).toBe(1);
    await screen.getByRole("button", { name: "More" }).click();
    await page.getByRole("menuitem", { name: "History" }).click();
    await screen.getByRole("button", { name: /^History X/u }).click();
    await screen.getByRole("button", { name: /^History Y/u }).click();
    await expect.poll(() => native.open.mock.calls.length).toBe(2);
    if (order === "last") {
      y.resolve(false);
      admission.resolve({ reply: 2, fault: null });
      await new Promise((done) => setTimeout(done, 0));
      await expect.element(field).toHaveValue("Original draft");
    }
    native.status = status([], {
      conversation: { id: "X", messages: [person(1, "Native history X")], title: null, seq: 0 },
    });
    const reads = native.statusCalls;
    x.resolve(true);
    await expect.poll(() => native.statusCalls).toBeGreaterThan(reads);
    if (order === "first") {
      y.resolve(false);
      admission.resolve({ reply: 2, fault: null });
    }
    await new Promise((done) => setTimeout(done, 0));
    // A successful X may close history; otherwise the back control exposes its projection.
    const back = screen.getByRole("button", { name: "History", exact: true });
    if (back.query()) await back.click();
    await expect.element(screen.getByText("Native history X")).toBeVisible();
    await expect.element(field).toHaveValue("Original draft");
  },
);

test("a suggestion sends itself with the page", async () => {
  const { screen } = await open(status());
  await screen.getByRole("button", { name: "Worth opening" }).click();
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({
      text: "",
      suggestion: "picks",
      attachments: [{ kind: "page", tab: TAB }],
    }),
  );
});

test("while a reply works, Send is Stop, and Escape stops it before closing anything", async () => {
  const onclose = vi.fn();
  native.status = status([person(1, "Summarize"), reply(2, [], { state: "working", cost: null })]);
  await page.viewport(400, 820);
  const screen = await render(PanelStage, { profile: PROFILE, onclose });
  const field = screen.getByRole("textbox", { name: "Ask Quasar…" });
  await expect.element(screen.getByRole("button", { name: "Stop" })).toBeVisible();
  await field.click();
  await userEvent.keyboard("{Escape}");
  await expect.poll(() => native.stop.mock.calls.length).toBe(1);
  expect(native.stop).toHaveBeenCalledWith(SPACE);
  expect(onclose).not.toHaveBeenCalled();
  await screen.getByRole("button", { name: "Stop" }).click();
  await expect.poll(() => native.stop.mock.calls.length).toBe(2);

  // Rust's snapshot moves on with its events; a focus reads it again.
  native.status = status([person(1, "Summarize"), reply(2, [text("Done.")])]);
  native.status.conversation.seq = 42;
  emitNativeEvent("pageAssistChanged", event(2, { type: "text", block: 0, delta: "Done." }));
  emitNativeEvent("pageAssistChanged", event(2, { type: "done", cost: null }));
  await expect.element(screen.getByRole("button", { name: "Dictate" })).toBeVisible();
  await field.click();
  await userEvent.keyboard("{Escape}");
  expect(onclose).toHaveBeenCalledOnce();
});

test("one round button: the mic on an empty field, Send once there are words", async () => {
  const { screen, field } = await open(status());
  await expect.element(screen.getByRole("button", { name: "Dictate" })).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Send" })).toHaveAttribute("inert");
  await field.fill("Hi");
  await expect.element(screen.getByRole("button", { name: "Send" })).toBeVisible();
  await expect.element(screen.getByRole("button", { name: "Dictate" })).toHaveAttribute("inert");
  await field.fill("");
  await expect.element(screen.getByRole("button", { name: "Dictate" })).toBeVisible();
});

test("the starts are a list the arrows walk, from the field up and back down", async () => {
  const { screen, field } = await open(status());
  await expect.element(screen.getByText("What can I do on Hacker News?")).toBeVisible();
  await field.click();
  await userEvent.keyboard("{ArrowUp}");
  await expect.element(screen.getByRole("button", { name: "Redesign this page" })).toHaveFocus();
  await userEvent.keyboard("{ArrowUp}");
  await expect.element(screen.getByRole("button", { name: "Key points" })).toHaveFocus();
  await userEvent.keyboard("{ArrowDown}{ArrowDown}");
  await expect.element(field).toHaveFocus();
});

test("+ attaches another tab of the Space alongside the page in front", async () => {
  setTabs({
    tabs: [
      tab(TAB, "https://news.ycombinator.com/", "Hacker News"),
      tab("tab-2", "https://github.com/zephium", "Quasar · GitHub"),
      tab("tab-3", "about:blank", ""),
    ],
  });
  const { screen, field } = await open(status());
  await screen.getByRole("button", { name: "Add" }).click();
  await page.getByRole("menuitem", { name: "Attach another tab" }).click();
  await expect
    .element(page.getByRole("menuitem", { name: /about:blank/u }))
    .not.toBeInTheDocument();
  await page.getByRole("menuitem", { name: /Quasar · GitHub/u }).click();
  await expect.element(screen.getByRole("button", { name: "Remove github.com" })).toBeVisible();
  await field.fill("Compare these");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({
      attachments: [
        { kind: "page", tab: TAB },
        { kind: "page", tab: "tab-2" },
      ],
    }),
  );
  await expect
    .element(screen.getByRole("button", { name: "Remove github.com" }))
    .not.toBeInTheDocument();
});

test("an item's card points at its mark while hovered; ⌘-click opens it in a new tab", async () => {
  const { screen } = await open(status([person(1, "Picks?"), reply(2, [text("Three."), ITEMS])]));
  const first = screen.getByRole("button", { name: /eMudhra to exit/u });
  await first.hover();
  await expect.poll(() => native.focus.mock.calls.at(-1)).toEqual([CONVERSATION, 2, 1, 1]);
  await screen.getByRole("textbox").hover();
  await expect.poll(() => native.focus.mock.calls.at(-1)).toEqual([CONVERSATION, 2, 1, null]);
  await first.click({ modifiers: ["Meta"] });
  await expect.poll(() => native.url.mock.calls.at(-1)).toEqual(["https://emudhra.com/", true]);
});

test("a confirm is answered on its own block, with the exact control", async () => {
  const { screen } = await open(
    status([
      person(1, "Upvote it"),
      reply(
        2,
        [
          text("One step needs you."),
          {
            kind: "steps",
            steps: [
              {
                index: 0,
                label: "Click “upvote”",
                category: "save",
                state: "confirm",
                reason: null,
              },
            ],
          },
        ],
        { state: "waiting", cost: null },
      ),
    ]),
  );
  await screen.getByRole("button", { name: "upvote", exact: true }).click();
  await expect.poll(() => native.decide.mock.calls.at(-1)).toEqual([CONVERSATION, 2, 1, "approve"]);
});

test("Continue in Work starts it; once started, Open in Work goes to Work", async () => {
  const { screen } = await open(
    status([person(1, "Set up the status page"), reply(2, [text("Work fits."), HANDOFF()])]),
  );
  await screen.getByRole("button", { name: "Continue in Work" }).click();
  await expect.poll(() => native.continueWork.mock.calls.at(-1)).toEqual([CONVERSATION, 2, 1]);
  emitNativeEvent(
    "pageAssistChanged",
    event(2, { type: "block", block: 1, value: HANDOFF({ state: "started", work: "w1" }) }),
  );
  await screen.getByRole("button", { name: "Open in Work" }).click();
  await expect.poll(() => native.surface.mock.calls.at(-1)).toEqual(["work"]);
});

test("Not now declines the handoff on its block", async () => {
  const { screen } = await open(status([person(1, "Set it up"), reply(2, [HANDOFF()])]));
  await screen.getByRole("button", { name: "Not now" }).click();
  await expect.poll(() => native.decide.mock.calls.at(-1)).toEqual([CONVERSATION, 2, 0, "decline"]);
});

test("the picker attaches what was picked; × detaches it; Escape cancels a pick", async () => {
  native.pick.mockResolvedValueOnce({
    pick: { tab: TAB, pick: "7", label: "Story list", host: "news.ycombinator.com" },
    fault: null,
  });
  const { screen, field } = await open(status());
  const pickFromMenu = async () => {
    await screen.getByRole("button", { name: "Add" }).click();
    await page.getByRole("menuitem", { name: "Pick an element" }).click();
  };
  await pickFromMenu();
  await expect.poll(() => native.pick.mock.calls.at(-1)).toEqual([SPACE, TAB]);
  await expect.element(screen.getByText("Story list")).toBeVisible();
  await field.fill("Summarize this part");
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => native.ask.mock.calls.length).toBe(1);
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({
      attachments: [
        { kind: "page", tab: TAB },
        { kind: "element", tab: TAB, pick: "7" },
      ],
    }),
  );
  await expect.element(screen.getByText("Story list")).not.toBeInTheDocument();

  native.pick.mockResolvedValueOnce({
    pick: { tab: TAB, pick: "8", label: "Pricing table", host: "news.ycombinator.com" },
    fault: null,
  });
  await pickFromMenu();
  await screen.getByRole("button", { name: "Remove Pricing table" }).click();
  await expect.poll(() => native.detach.mock.calls.at(-1)).toEqual([TAB, "8"]);

  native.pick.mockImplementationOnce(() => new Promise(() => {}));
  await pickFromMenu();
  await expect
    .element(screen.getByRole("status").filter({ hasText: "Esc to cancel" }))
    .toBeVisible();
  await userEvent.keyboard("{Escape}");
  await expect.poll(() => native.pickCancel.mock.calls.at(-1)).toEqual([TAB]);
});

test("history lists past conversations, opens one and deletes one", async () => {
  const { screen } = await open(status([person(1, "Hi"), reply(2, [text("Hello.")])]));
  await screen.getByRole("button", { name: "More" }).click();
  await page.getByRole("menuitem", { name: "History" }).click();
  const row = screen.getByRole("button", { name: /^Pricing on the Pro plan/u });
  await expect.element(row).toBeVisible();
  await screen.getByRole("button", { name: "Delete “Pricing on the Pro plan”" }).click();
  await expect.poll(() => native.remove.mock.calls.at(-1)).toEqual([SPACE, "c0"]);
  await expect.element(row).not.toBeInTheDocument();

  native.history.mockResolvedValueOnce({
    version: 2,
    conversations: [
      { id: "c0", title: "Pricing on the Pro plan", space: SPACE, updated_ms: Date.now() },
    ],
  });
  await screen.getByRole("button", { name: "History" }).click();
  await screen.getByRole("button", { name: "More" }).click();
  await page.getByRole("menuitem", { name: "History" }).click();
  await screen.getByRole("button", { name: /^Pricing on the Pro plan/u }).click();
  await expect.poll(() => native.open.mock.calls.at(-1)).toEqual([SPACE, "c0"]);
  await expect.element(screen.getByRole("log")).toBeVisible();
});

test("a gap in the events reads the conversation again; closing the panel lets the page go", async () => {
  const { screen } = await open(
    status([person(1, "Hi"), reply(2, [], { state: "working", cost: null })]),
  );
  const before = native.statusCalls;
  emitNativeEvent("pageAssistChanged", { ...event(2, { type: "cancelled" }), seq: 99 });
  await expect.poll(() => native.statusCalls).toBe(before + 1);
  screen.unmount();
  expect(native.close).toHaveBeenCalledWith(SPACE);
});
