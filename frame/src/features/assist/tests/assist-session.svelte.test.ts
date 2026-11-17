import { beforeEach, expect, test, vi } from "vitest";
import type {
  PageAssistAskResponseV1,
  PageAssistAskV1,
  PageAssistStatusV1,
} from "$shared/ipc/bindings";
import { AssistSession } from "../lib/assist-session.svelte";
import { CONVERSATION, SPACE, TAB, person, status } from "./fixtures";

const native = vi.hoisted(() => ({
  ask: vi.fn(),
  status: vi.fn(),
  fresh: vi.fn(),
  open: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    pageAssistAsk: native.ask,
    pageAssistStatus: native.status,
    pageAssistNew: native.fresh,
    pageAssistOpen: native.open,
    pageAssistClose: async () => true,
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function session(view: PageAssistStatusV1 = status()) {
  const owner = new AssistSession("profile");
  owner.space = view.space;
  owner.tab = view.tab;
  owner.status = view;
  owner.conversation = view.conversation;
  return owner;
}

const picked = (id: string) => ({
  tab: TAB,
  pick: id,
  label: `Element ${id}`,
  host: "example.com",
  url: "https://example.com/",
});

beforeEach(() => vi.resetAllMocks());

test("admission consumes only the unchanged submitted attachment choices", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValue(admission.promise);
  const owner = session(status([], { selection: "selected words" }));
  owner.picks = [picked("old")];
  owner.others = ["other"];
  owner.pageOff = true;
  owner.selectionOn = true;
  const sending = owner.ask("First message");
  expect(native.ask).toHaveBeenCalledWith({
    space: SPACE,
    conversation: CONVERSATION,
    text: "First message",
    suggestion: null,
    attachments: [
      { kind: "page", tab: "other" },
      { kind: "selection", tab: TAB },
      { kind: "element", tab: TAB, pick: "old" },
    ],
  });
  // An edit to one choice does not keep the other, unchanged submitted choices.
  owner.attachTab("new-other");
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(owner.picks).toEqual([]);
  expect(owner.others).toEqual(["other", "new-other"]);
  expect(owner.pageOff).toBe(false);
  expect(owner.selectionOn).toBe(false);
  expect(owner.sending).toBe(false);
});

test("same-conversation attachment edits and toggles back to their submitted values survive admission", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValue(admission.promise);
  const owner = session(status([], { selection: "selected words" }));
  owner.picks = [picked("old")];
  owner.others = ["other"];
  owner.pageOff = true;
  owner.selectionOn = true;
  const sending = owner.ask("First message");
  owner.picks = [picked("new")];
  owner.attachTab("next");
  owner.pageOff = false;
  owner.pageOff = true;
  owner.selectionOn = false;
  owner.selectionOn = true;
  owner.notice = { reason: "no_selection" };
  admission.resolve({ reply: 2, fault: null });
  await sending;
  expect(owner.picks.map((pick) => pick.pick)).toEqual(["new"]);
  expect(owner.others).toEqual(["other", "next"]);
  expect(owner.pageOff).toBe(true);
  expect(owner.selectionOn).toBe(true);
  expect(owner.notice).toEqual({ reason: "no_selection" });
});

test("admission consumes pick A after adding B, so the next send contains only the valid new pick", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask
    .mockReturnValueOnce(admission.promise)
    .mockImplementationOnce(async (request: PageAssistAskV1): Promise<PageAssistAskResponseV1> =>
      // Native consumed A at admission and refuses any attempt to reuse it.
      request.attachments.some(
        (attachment) => attachment.kind === "element" && attachment.pick === "A",
      )
        ? { reply: null, fault: { reason: "page_changed" } }
        : { reply: 4, fault: null },
    );
  const owner = session();
  owner.picks = [picked("A")];
  const sending = owner.ask("First message");
  owner.picks = [...owner.picks, picked("B")];
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  const retained = owner.picks;
  const next = await owner.ask("Next message");
  expect(next).toBe(true);
  expect(retained).toEqual([picked("B")]);
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({
      attachments: [
        { kind: "page", tab: TAB },
        { kind: "element", tab: TAB, pick: "B" },
      ],
    }),
  );
  expect(owner.picks).toEqual([]);
});

test("admission preserves unchanged picks excluded by the attachment limit", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValue(admission.promise);
  const owner = session(status([], { selection: "selected words" }));
  owner.others = ["other-1", "other-2", "other-3"];
  owner.selectionOn = true;
  owner.picks = [picked("A"), picked("B")];
  const sending = owner.ask("Bounded attachments");
  expect(native.ask).toHaveBeenCalledWith(
    expect.objectContaining({
      attachments: [
        { kind: "page", tab: TAB },
        { kind: "page", tab: "other-1" },
        { kind: "page", tab: "other-2" },
        { kind: "page", tab: "other-3" },
        { kind: "selection", tab: TAB },
        { kind: "element", tab: TAB, pick: "A" },
      ],
    }),
  );
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(owner.picks).toEqual([picked("B")]);
});

test("admission retires only submitted pick objects retained across a conversation switch", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask
    .mockReturnValueOnce(admission.promise)
    .mockImplementationOnce(async (request: PageAssistAskV1): Promise<PageAssistAskResponseV1> =>
      request.attachments.some(
        (attachment) => attachment.kind === "element" && attachment.pick === "A",
      )
        ? { reply: null, fault: { reason: "page_changed" } }
        : { reply: 4, fault: null },
    );
  const owner = session(status([], { selection: "selected words" }));
  owner.others = ["other-1", "other-2", "other-3"];
  owner.selectionOn = true;
  const a = picked("A");
  const b = picked("B");
  const c = picked("C");
  owner.picks = [a, b];
  const sending = owner.ask("Old conversation");
  expect(native.ask).toHaveBeenCalledWith(
    expect.objectContaining({
      attachments: expect.arrayContaining([{ kind: "element", tab: TAB, pick: "A" }]),
    }),
  );
  const next = status([], {
    selection: "selected words",
    conversation: { id: "history", title: null, messages: [], seq: 0 },
  });
  native.open.mockResolvedValue(true);
  native.status.mockResolvedValue(next);
  expect(await owner.openPast("history")).toBe(true);
  await vi.waitFor(() => expect(owner.conversation.id).toBe("history"));
  owner.picks = [...owner.picks, c];
  owner.notice = { reason: "no_selection" };
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(owner.picks).toEqual([b, c]);
  expect(owner.others).toEqual(["other-1", "other-2", "other-3"]);
  expect(owner.selectionOn).toBe(true);
  expect(owner.notice).toEqual({ reason: "no_selection" });
  expect(await owner.ask("History message")).toBe(true);
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({
      conversation: "history",
      attachments: [
        { kind: "page", tab: TAB },
        { kind: "page", tab: "other-1" },
        { kind: "page", tab: "other-2" },
        { kind: "page", tab: "other-3" },
        { kind: "selection", tab: TAB },
        { kind: "element", tab: TAB, pick: "B" },
      ],
    }),
  );
  expect(owner.picks).toEqual([c]);
});

test("a retained pick cannot be reused before its original admission settles, while unreserved picks can send", async () => {
  const first = deferred<PageAssistAskResponseV1>();
  const second = deferred<PageAssistAskResponseV1>();
  native.ask
    .mockReturnValueOnce(first.promise)
    .mockImplementation((request: PageAssistAskV1) =>
      request.attachments.some(
        (attachment) => attachment.kind === "element" && attachment.pick === "A",
      )
        ? Promise.resolve({ reply: null, fault: { reason: "page_changed" } })
        : second.promise,
    );
  const owner = session();
  const a = picked("A");
  const b = picked("B");
  owner.picks = [a];
  const sendingFirst = owner.ask("Original send");
  native.open.mockResolvedValue(true);
  native.status.mockResolvedValue(
    status([], {
      conversation: { id: "history", title: null, messages: [], seq: 0 },
    }),
  );
  expect(await owner.openPast("history")).toBe(true);
  await vi.waitFor(() => expect(owner.conversation.id).toBe("history"));
  owner.picks = [...owner.picks, b];
  // A may already be consumed natively although its acknowledgement is delayed.
  expect(await owner.ask("Must not reuse A")).toBe(false);
  expect(native.ask).toHaveBeenCalledOnce();
  expect(owner.picks).toEqual([a, b]);
  // Explicitly removing A leaves a truthful request for the independent new pick.
  owner.picks = [b];
  const sendingSecond = owner.ask("Only B");
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({
      conversation: "history",
      attachments: [
        { kind: "page", tab: TAB },
        { kind: "element", tab: TAB, pick: "B" },
      ],
    }),
  );
  first.resolve({ reply: 2, fault: null });
  expect(await sendingFirst).toBe(true);
  expect(owner.sending).toBe(true);
  expect(owner.picks).toEqual([b]);
  second.resolve({ reply: 4, fault: null });
  expect(await sendingSecond).toBe(true);
  expect(owner.picks).toEqual([]);
});

test.each(["fault", "error"] as const)(
  "an admission %s releases its retained pick reservation in a different conversation",
  async (outcome) => {
    const admission = deferred<PageAssistAskResponseV1>();
    native.ask
      .mockReturnValueOnce(admission.promise)
      .mockResolvedValueOnce({ reply: 4, fault: null });
    const owner = session();
    const a = picked("A");
    owner.picks = [a];
    const sending = owner.ask("Original send");
    native.open.mockResolvedValue(true);
    native.status.mockResolvedValue(
      status([], {
        conversation: { id: "history", title: null, messages: [], seq: 0 },
      }),
    );
    expect(await owner.openPast("history")).toBe(true);
    await vi.waitFor(() => expect(owner.conversation.id).toBe("history"));
    expect(await owner.ask("Still reserved")).toBe(false);
    expect(native.ask).toHaveBeenCalledOnce();
    if (outcome === "error") admission.reject(new Error("Admission unavailable"));
    else admission.resolve({ reply: null, fault: { reason: "busy" } });
    expect(await sending).toBe(false);
    expect(owner.picks).toEqual([a]);
    expect(await owner.ask("Retry unconsumed pick")).toBe(true);
    expect(native.ask).toHaveBeenLastCalledWith(
      expect.objectContaining({
        conversation: "history",
        attachments: [
          { kind: "page", tab: TAB },
          { kind: "element", tab: TAB, pick: "A" },
        ],
      }),
    );
  },
);

test("a pick excluded by the original attachment limit remains available to another conversation", async () => {
  const first = deferred<PageAssistAskResponseV1>();
  const second = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const owner = session(status([], { selection: "selected words" }));
  const a = picked("A");
  const b = picked("B");
  owner.others = ["other-1", "other-2", "other-3"];
  owner.selectionOn = true;
  owner.picks = [a, b];
  const sendingFirst = owner.ask("Only A fits");
  expect(native.ask).toHaveBeenCalledWith(
    expect.objectContaining({
      attachments: expect.arrayContaining([{ kind: "element", tab: TAB, pick: "A" }]),
    }),
  );
  native.open.mockResolvedValue(true);
  native.status.mockResolvedValue(
    status([], {
      selection: "selected words",
      conversation: { id: "history", title: null, messages: [], seq: 0 },
    }),
  );
  expect(await owner.openPast("history")).toBe(true);
  // A visible reserved chip must still hold Send when it moves outside the bound.
  owner.picks = [b, a];
  expect(await owner.ask("Both picks are still shown")).toBe(false);
  expect(native.ask).toHaveBeenCalledOnce();
  owner.picks = [b];
  const sendingSecond = owner.ask("B is unreserved");
  expect(native.ask).toHaveBeenCalledTimes(2);
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({
      attachments: expect.arrayContaining([{ kind: "element", tab: TAB, pick: "B" }]),
    }),
  );
  first.resolve({ reply: null, fault: { reason: "busy" } });
  expect(await sendingFirst).toBe(false);
  expect(owner.sending).toBe(true);
  expect(owner.picks).toEqual([b]);
  second.resolve({ reply: 4, fault: null });
  expect(await sendingSecond).toBe(true);
  expect(owner.picks).toEqual([]);
});

test("an old admission preserves a replacement pick object with the same tab and token", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValue(admission.promise);
  const owner = session();
  owner.picks = [picked("A")];
  const sending = owner.ask("First context");
  native.status.mockResolvedValue(status([], { space: "space-b" }));
  owner.follow("space-b", TAB);
  await vi.waitFor(() => expect(owner.status?.space).toBe("space-b"));
  const replacement = picked("A");
  owner.picks = [replacement];
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(owner.picks).toEqual([replacement]);
  expect(owner.picks[0]).toBe(replacement);
});

test("a consumed pick identity includes its tab and preserves unrelated attachment edits", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValue(admission.promise);
  const owner = session();
  owner.picks = [picked("A")];
  const sending = owner.ask("First tab");
  const otherPick = { ...picked("A"), tab: "another-tab" };
  owner.picks = [...owner.picks, otherPick];
  owner.attachTab("another-tab");
  owner.pageOff = true;
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(owner.picks).toEqual([otherPick]);
  expect(owner.others).toEqual(["another-tab"]);
  expect(owner.pageOff).toBe(true);
});

test.each(["fault", "error"] as const)(
  "an admission %s preserves submitted and newly added picks",
  async (outcome) => {
    const admission = deferred<PageAssistAskResponseV1>();
    native.ask.mockReturnValue(admission.promise);
    const owner = session();
    owner.picks = [picked("A")];
    const sending = owner.ask("Keep pending picks");
    owner.picks = [...owner.picks, picked("B")];
    if (outcome === "error") admission.reject(new Error("admission unavailable"));
    else admission.resolve({ reply: null, fault: { reason: "page_changed" } });
    expect(await sending).toBe(false);
    expect(owner.picks).toEqual([picked("A"), picked("B")]);
    expect(owner.sending).toBe(false);
  },
);

test("opening the current conversation is a no-op and its pending send consumes attachments once", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask
    .mockReturnValueOnce(admission.promise)
    .mockResolvedValueOnce({ reply: 4, fault: null });
  const view = status([], { selection: "selected words" });
  native.open.mockResolvedValue(true);
  native.status.mockResolvedValue(view);
  const owner = session(view);
  owner.picks = [picked("A")];
  owner.others = ["other"];
  owner.selectionOn = true;
  const sending = owner.ask("Original send");
  const generation = owner.conversationGeneration;
  const opened = await owner.openPast(CONVERSATION);
  const stillSending = owner.sending;
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(opened).toBe(true);
  expect(stillSending).toBe(true);
  expect(owner.conversationGeneration).toBe(generation);
  expect(owner.picks).toEqual([]);
  expect(owner.others).toEqual([]);
  expect(owner.selectionOn).toBe(false);
  expect(owner.sending).toBe(false);
  expect(native.open).not.toHaveBeenCalled();
  expect(native.status).not.toHaveBeenCalled();
  expect(await owner.ask("Next message")).toBe(true);
  expect(native.ask).toHaveBeenLastCalledWith(
    expect.objectContaining({ attachments: [{ kind: "page", tab: TAB }] }),
  );
});

test.each(["success", "fault", "error"] as const)(
  "a late %s from another Space cannot settle the current send or replace its composer state",
  async (outcome) => {
    const first = deferred<PageAssistAskResponseV1>();
    const second = deferred<PageAssistAskResponseV1>();
    native.ask.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const owner = session();
    const sendingA = owner.ask("Space A");
    const next = status([], {
      space: "space-b",
      conversation: { id: "conversation-b", title: null, messages: [], seq: 0 },
    });
    native.status.mockResolvedValue(next);
    owner.follow(next.space, TAB);
    await vi.waitFor(() => expect(owner.conversation.id).toBe("conversation-b"));
    owner.picks = [picked("next")];
    owner.others = ["other-b"];
    owner.pageOff = true;
    owner.selectionOn = true;
    const sendingB = owner.ask("Space B");
    expect(native.ask).toHaveBeenCalledTimes(2);
    owner.notice = { reason: "no_selection" };
    if (outcome === "error") first.reject(new Error("late transport failure"));
    else
      first.resolve({
        reply: outcome === "success" ? 2 : null,
        fault: outcome === "fault" ? { reason: "invalid" } : null,
      });
    await sendingA;
    expect(owner.sending).toBe(true);
    expect(owner.notice).toEqual({ reason: "no_selection" });
    expect(owner.picks.map((pick) => pick.pick)).toEqual(["next"]);
    expect(owner.others).toEqual(["other-b"]);
    expect(owner.pageOff).toBe(true);
    expect(owner.selectionOn).toBe(true);
    expect(native.status).toHaveBeenCalledTimes(1);
    second.resolve({ reply: null, fault: { reason: "busy" } });
    await sendingB;
    expect(owner.sending).toBe(false);
    expect(owner.notice).toEqual({ reason: "no_selection" });
  },
);

test.each(["fresh", "history"] as const)(
  "%s selection ends the old send's lifetime before its native acknowledgement",
  async (choice) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const switching = deferred<boolean>();
    native.ask.mockReturnValue(admission.promise);
    native.fresh.mockReturnValue(switching.promise);
    native.open.mockReturnValue(switching.promise);
    const owner = session(status([person(1, "Earlier message")]));
    const epoch = owner.conversationGeneration;
    const sending = owner.ask("Old conversation");
    const selected = choice === "fresh" ? owner.newConversation() : owner.openPast("history");
    expect(owner.conversationGeneration).toBeGreaterThan(epoch);
    expect(owner.sending).toBe(false);
    owner.picks = [picked("next")];
    owner.notice = { reason: "page_changed" };
    admission.resolve({ reply: 2, fault: null });
    await sending;
    expect(owner.picks.map((pick) => pick.pick)).toEqual(["next"]);
    expect(owner.notice).toEqual({ reason: "page_changed" });
    switching.resolve(false);
    await selected;
  },
);

test.each([
  ["fresh", "before"],
  ["fresh", "after"],
  ["history", "before"],
  ["history", "after"],
] as const)(
  "a refused %s switch reconciles admission arriving %s the refusal",
  async (choice, order) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const switching = deferred<boolean>();
    native.ask.mockReturnValue(admission.promise);
    native.fresh.mockReturnValue(switching.promise);
    native.open.mockReturnValue(switching.promise);
    const owner = session(status([person(1, "Earlier")], { selection: "selected words" }));
    owner.picks = [picked("A")];
    owner.others = ["other"];
    owner.pageOff = true;
    owner.selectionOn = true;
    const sending = owner.ask("Admitted once");
    const selected = choice === "fresh" ? owner.newConversation() : owner.openPast("history");
    if (order === "before") {
      admission.resolve({ reply: 2, fault: null });
      expect(await sending).toBe(true);
    }
    switching.resolve(false);
    await selected;
    if (order === "after") admission.resolve({ reply: 2, fault: null });
    expect(await sending).toBe(true);
    expect(owner.conversation.id).toBe(CONVERSATION);
    expect(owner.picks).toEqual([]);
    expect(owner.others).toEqual([]);
    expect(owner.pageOff).toBe(false);
    expect(owner.selectionOn).toBe(false);
    expect(owner.sending).toBe(false);
  },
);

test("an opening acknowledged after its projection does not orphan a send in the opened conversation", async () => {
  const oldAdmission = deferred<PageAssistAskResponseV1>();
  const nextAdmission = deferred<PageAssistAskResponseV1>();
  const switching = deferred<boolean>();
  native.ask.mockReturnValueOnce(oldAdmission.promise).mockReturnValueOnce(nextAdmission.promise);
  native.open.mockReturnValue(switching.promise);
  const owner = session();
  owner.picks = [picked("A")];
  const sending = owner.ask("Old conversation");
  const selected = owner.openPast("history");
  const next = status([], {
    selection: "selected words",
    conversation: { id: "history", title: null, messages: [], seq: 0 },
  });
  native.status.mockResolvedValue(next);
  owner.refresh();
  await vi.waitFor(() => expect(owner.conversation.id).toBe("history"));
  const replacement = picked("A");
  owner.picks = [replacement];
  owner.others = ["next-other"];
  owner.pageOff = true;
  owner.selectionOn = true;
  const sendingNext = owner.ask("Opened conversation");
  expect(native.ask).toHaveBeenCalledTimes(2);
  owner.notice = { reason: "no_selection" };
  switching.resolve(true);
  expect(await selected).toBe(true);
  oldAdmission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(owner.sending).toBe(true);
  expect(owner.picks).toEqual([replacement]);
  expect(owner.others).toEqual(["next-other"]);
  expect(owner.pageOff).toBe(true);
  expect(owner.selectionOn).toBe(true);
  expect(owner.notice).toEqual({ reason: "no_selection" });
  nextAdmission.resolve({ reply: null, fault: { reason: "busy" } });
  expect(await sendingNext).toBe(false);
  expect(owner.sending).toBe(false);
});

test.each(["before", "after"] as const)(
  "repeated refused opens reconcile the admission when the latest refusal arrives %s the older one",
  async (order) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const first = deferred<boolean>();
    const second = deferred<boolean>();
    native.ask.mockReturnValue(admission.promise);
    native.open.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const owner = session();
    owner.others = ["other"];
    const sending = owner.ask("Admitted once");
    const openedFirst = owner.openPast("history-1");
    const openedSecond = owner.openPast("history-2");
    admission.resolve({ reply: 2, fault: null });
    expect(await sending).toBe(true);
    expect(owner.others).toEqual(["other"]);
    if (order === "after") {
      first.resolve(false);
      expect(await openedFirst).toBe(false);
      expect(owner.others).toEqual(["other"]);
    }
    second.resolve(false);
    expect(await openedSecond).toBe(false);
    if (order === "before") {
      expect(owner.others).toEqual(["other"]);
      first.resolve(false);
      expect(await openedFirst).toBe(false);
    }
    expect(owner.others).toEqual([]);
    expect(owner.sending).toBe(false);
  },
);

test.each(["first", "last"] as const)(
  "a superseded successful open acknowledged %s reconciles native history after a newer refusal",
  async (order) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const x = deferred<boolean>();
    const y = deferred<boolean>();
    const snapshot = deferred<PageAssistStatusV1>();
    native.ask.mockReturnValue(admission.promise);
    native.open.mockReturnValueOnce(x.promise).mockReturnValueOnce(y.promise);
    native.status.mockReturnValue(snapshot.promise);
    const owner = session();
    owner.others = ["old-other"];
    const sending = owner.ask("Original draft");
    const openedX = owner.openPast("X");
    const openedY = owner.openPast("Y");
    if (order === "last") {
      y.resolve(false);
      expect(await openedY).toBe(false);
      admission.resolve({ reply: 2, fault: null });
      expect(await sending).toBe(true);
      expect(owner.others).toEqual(["old-other"]);
    }
    x.resolve(true);
    await vi.waitFor(() => expect(native.status).toHaveBeenCalledOnce());
    // The authoritative snapshot, rather than response order, chooses X.
    snapshot.resolve(
      status([], {
        conversation: { id: "X", title: null, messages: [], seq: 0 },
      }),
    );
    await openedX;
    await vi.waitFor(() => expect(owner.conversation.id).toBe("X"));
    owner.others = ["new-other"];
    if (order === "first") {
      y.resolve(false);
      expect(await openedY).toBe(false);
      admission.resolve({ reply: 2, fault: null });
      expect(await sending).toBe(true);
    }
    expect(owner.others).toEqual(["new-other"]);
    expect(owner.conversation.id).toBe("X");
  },
);

test.each(["first", "last"] as const)(
  "pending navigation outcomes cannot reconcile a new Space when the older success arrives %s",
  async (order) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const nextAdmission = deferred<PageAssistAskResponseV1>();
    const x = deferred<boolean>();
    const y = deferred<boolean>();
    native.ask.mockReturnValueOnce(admission.promise).mockReturnValueOnce(nextAdmission.promise);
    native.open.mockReturnValueOnce(x.promise).mockReturnValueOnce(y.promise);
    const owner = session();
    owner.picks = [picked("A")];
    const sending = owner.ask("Original Space");
    const openedX = owner.openPast("X");
    const openedY = owner.openPast("Y");
    native.status.mockResolvedValue(
      status([], {
        space: "space-b",
        conversation: { id: "B", title: null, messages: [], seq: 0 },
      }),
    );
    owner.follow("space-b", TAB);
    await vi.waitFor(() => expect(owner.conversation.id).toBe("B"));
    owner.others = ["new-other"];
    owner.picks = [picked("B")];
    const sendingNext = owner.ask("New Space");
    if (order === "first") {
      x.resolve(true);
      await openedX;
      y.resolve(false);
      await openedY;
    } else {
      y.resolve(false);
      await openedY;
      x.resolve(true);
      await openedX;
    }
    admission.resolve({ reply: 2, fault: null });
    expect(await sending).toBe(true);
    expect(owner.conversation.id).toBe("B");
    expect(owner.others).toEqual(["new-other"]);
    expect(owner.picks).toEqual([picked("B")]);
    expect(owner.sending).toBe(true);
    expect(native.status).toHaveBeenCalledOnce();
    nextAdmission.resolve({ reply: null, fault: { reason: "busy" } });
    expect(await sendingNext).toBe(false);
  },
);

test("late successful navigation acknowledgements read native state without replacing a newer pending turn", async () => {
  const oldAdmission = deferred<PageAssistAskResponseV1>();
  const nextAdmission = deferred<PageAssistAskResponseV1>();
  const x = deferred<boolean>();
  const y = deferred<boolean>();
  native.ask.mockReturnValueOnce(oldAdmission.promise).mockReturnValueOnce(nextAdmission.promise);
  native.open.mockReturnValueOnce(x.promise).mockReturnValueOnce(y.promise);
  const owner = session();
  const sendingOld = owner.ask("Original conversation");
  const openedX = owner.openPast("X");
  const openedY = owner.openPast("Y");
  // Native executed X then Y, although X's acknowledgement arrives last.
  native.status.mockResolvedValue(
    status([], {
      conversation: { id: "Y", title: null, messages: [], seq: 0 },
    }),
  );
  y.resolve(true);
  expect(await openedY).toBe(true);
  expect(owner.conversation.id).toBe("Y");
  owner.picks = [picked("new")];
  const sendingNext = owner.ask("Y's pending turn");
  expect(native.ask).toHaveBeenCalledTimes(2);
  x.resolve(true);
  await openedX;
  expect(native.status).toHaveBeenCalledTimes(2);
  expect(owner.conversation.id).toBe("Y");
  expect(owner.sending).toBe(true);
  oldAdmission.resolve({ reply: 2, fault: null });
  expect(await sendingOld).toBe(true);
  expect(owner.sending).toBe(true);
  expect(owner.picks).toEqual([picked("new")]);
  nextAdmission.resolve({ reply: 4, fault: null });
  expect(await sendingNext).toBe(true);
  expect(owner.sending).toBe(false);
});

test.each(["fresh", "history"] as const)(
  "a stale failed open cannot reconcile the old composer after a newer %s switch",
  async (choice) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const first = deferred<boolean>();
    native.ask.mockReturnValue(admission.promise);
    native.open.mockReturnValueOnce(first.promise).mockResolvedValueOnce(true);
    native.fresh.mockResolvedValue(true);
    const owner = session(status([person(1, "Earlier")]));
    owner.others = ["old-other"];
    const sending = owner.ask("Old conversation");
    const openedFirst = owner.openPast("history-1");
    admission.resolve({ reply: 2, fault: null });
    expect(await sending).toBe(true);
    const next = status([], {
      conversation: { id: "next", title: null, messages: [], seq: 0 },
    });
    native.status.mockResolvedValue(next);
    if (choice === "fresh") await owner.newConversation();
    else expect(await owner.openPast("next")).toBe(true);
    await vi.waitFor(() => expect(owner.conversation.id).toBe("next"));
    owner.others = ["next-other"];
    owner.picks = [picked("next")];
    owner.pageOff = true;
    owner.notice = { reason: "no_selection" };
    first.resolve(false);
    expect(await openedFirst).toBe(false);
    expect(owner.others).toEqual(["next-other"]);
    expect(owner.picks).toEqual([picked("next")]);
    expect(owner.pageOff).toBe(true);
    expect(owner.notice).toEqual({ reason: "no_selection" });
  },
);

test("a failed open after leaving and returning to the same Space cannot reclaim that composer", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  const switching = deferred<boolean>();
  native.ask.mockReturnValue(admission.promise);
  native.open.mockReturnValue(switching.promise);
  const owner = session();
  const sending = owner.ask("Original composer");
  const selected = owner.openPast("history");
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  const next = status([], { space: "space-b" });
  native.status.mockResolvedValueOnce(next).mockResolvedValueOnce(status());
  owner.follow("space-b", TAB);
  await vi.waitFor(() => expect(owner.status?.space).toBe("space-b"));
  owner.follow(SPACE, TAB);
  await vi.waitFor(() => expect(owner.status?.space).toBe(SPACE));
  owner.others = ["new-other"];
  owner.picks = [picked("new")];
  switching.resolve(false);
  expect(await selected).toBe(false);
  expect(owner.conversation.id).toBe(CONVERSATION);
  expect(owner.others).toEqual(["new-other"]);
  expect(owner.picks).toEqual([picked("new")]);
});

test.each(["fresh", "history"] as const)(
  "a refused %s switch keeps an unsettled admission single-flight across a snapshot refresh",
  async (choice) => {
    const admission = deferred<PageAssistAskResponseV1>();
    const switching = deferred<boolean>();
    native.ask.mockReturnValue(admission.promise);
    native.open.mockReturnValue(switching.promise);
    native.fresh.mockReturnValue(switching.promise);
    native.status.mockResolvedValue(status([person(1, "Earlier")]));
    const owner = session(status([person(1, "Earlier")]));
    const sending = owner.ask("Admitted once");
    const selected = choice === "fresh" ? owner.newConversation() : owner.openPast("history");
    expect(await owner.ask("Duplicate while navigating")).toBe(false);
    owner.refresh();
    await vi.waitFor(() => expect(native.status).toHaveBeenCalledOnce());
    switching.resolve(false);
    await selected;
    expect(owner.sending).toBe(true);
    expect(await owner.ask("Duplicate after refusal")).toBe(false);
    expect(native.ask).toHaveBeenCalledOnce();
    admission.resolve({ reply: 2, fault: null });
    expect(await sending).toBe(true);
    expect(owner.sending).toBe(false);
  },
);

test("snapshot refreshes of the same conversation do not orphan a legitimate send", async () => {
  const admission = deferred<PageAssistAskResponseV1>();
  native.ask.mockReturnValue(admission.promise);
  native.status.mockResolvedValue(status());
  const owner = session();
  const sending = owner.ask("Keep this lifetime");
  const epoch = owner.conversationGeneration;
  owner.refresh();
  await vi.waitFor(() => expect(native.status).toHaveBeenCalledOnce());
  expect(owner.conversationGeneration).toBe(epoch);
  admission.resolve({ reply: 2, fault: null });
  expect(await sending).toBe(true);
  expect(owner.sending).toBe(false);
});
