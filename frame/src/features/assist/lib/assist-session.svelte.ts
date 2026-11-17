import {
  commands,
  type PageAssistAskResponseV1,
  type PageAssistAttachV1,
  type PageAssistChangedV1,
  type PageAssistCiteV1,
  type PageAssistDecisionV1,
  type PageAssistFaultV1,
  type PageAssistPastV1,
  type PageAssistPickV1,
  type PageAssistStatusV1,
  type PageAssistSuggestionV1,
  type PageStylesV1,
} from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";
import { EMPTY, askedBy, fold, latestReply, open, type Conversation } from "./conversation";
import type { Arrival } from "./markdown";

/** Rust holds the same bound, in UTF-8 bytes; a character is at most four. */
export const MESSAGE_LIMIT = 2048;
/** The latest deltas of a streaming text that still ink in. */
const FRESH = 3;
const MAX_ATTACHMENTS = 6;
const MAX_OTHERS = 3;

/** A picked element, with the address its page had when it was picked. */
export type Pick = PageAssistPickV1 & { url: string | null };

type Navigation = { generation: number; settled: Promise<void>; finish: () => void };

const quietly = <T>(promise: Promise<T>, fallback: T) => promise.catch(() => fallback);

/**
 * The panel's view of a Space's conversation. Rust owns it; this follows the
 * Space and the tab in front, takes Rust's snapshot, and folds each event once,
 * in `seq` order. Anything it cannot fold in order sends it back to the
 * snapshot. The composer's own choices (the page chip, the selection, picked
 * elements) live here until they are sent.
 */
export class AssistSession {
  space = $state<string | null>(null);
  tab = $state<string | null>(null);
  status = $state.raw<PageAssistStatusV1 | null>(null);
  conversation = $state.raw<Conversation>(EMPTY);
  /** The first read of this Space has not come back yet. */
  loading = $state(false);
  /** Rust could not be reached. */
  unavailable = $state(false);
  /** The words each streaming text block received last, by `reply:block`. */
  arrivals = $state.raw<Readonly<Record<string, readonly Arrival[]>>>({});
  private picked = $state.raw<Pick[]>([]);
  private reservedPicks = $state.raw<readonly Pick[]>([]);
  /** The tab an element is being picked on. */
  picking = $state<string | null>(null);
  /** The person took the page chip off for the next message. */
  private pageExcluded = $state(false);
  /** Other tabs of the Space the person attached to the next message. */
  private otherTabs = $state.raw<string[]>([]);
  private selectionIncluded = $state(false);
  /** Why the last message or pick did not start. */
  private lastNotice = $state.raw<PageAssistFaultV1 | null>(null);
  sending = $state(false);
  styles = $state.raw<PageStylesV1 | null>(null);
  past = $state.raw<PageAssistPastV1[] | null>(null);

  private generation = 0;
  private conversationEpoch = 0;
  /** Older generations expire only when native or its projection confirms a switch. */
  private composerEpoch = $state(0);
  private navigations = $state.raw<Navigation[]>([]);
  private pendingSend: object | null = null;
  private revisions = { others: 0, page: 0, selection: 0, notice: 0 };
  private queue: PageAssistChangedV1[] | null = null;
  private stopListening: (() => void) | null = null;
  private active = false;

  constructor(readonly profile: string) {}

  /** A composer generation, independent of snapshot refreshes. */
  get conversationGeneration() {
    return this.conversationEpoch;
  }

  /** A refused navigation keeps ownership of the unchanged admitted composer. */
  async settleComposer(generation: number, settle: () => void) {
    while (this.navigations.length > 0 && generation >= this.composerEpoch)
      await Promise.all(this.navigations.map((navigation) => navigation.settled));
    if (generation >= this.composerEpoch) settle();
  }

  get navigating() {
    return this.navigations.some((navigation) => navigation.generation >= this.composerEpoch);
  }

  pickPending(pick: Pick) {
    return this.reservedPicks.includes(pick);
  }

  /** A visible reserved chip cannot promise its pick to another message. */
  get attachmentsPending() {
    return this.picks.some((pick) => this.pickPending(pick));
  }

  get picks() {
    return this.picked;
  }
  set picks(value: Pick[]) {
    this.picked = value;
  }
  get others() {
    return this.otherTabs;
  }
  set others(value: string[]) {
    this.revisions.others++;
    this.otherTabs = value;
  }
  get pageOff() {
    return this.pageExcluded;
  }
  set pageOff(value: boolean) {
    this.revisions.page++;
    this.pageExcluded = value;
  }
  get selectionOn() {
    return this.selectionIncluded;
  }
  set selectionOn(value: boolean) {
    this.revisions.selection++;
    this.selectionIncluded = value;
  }
  get notice() {
    return this.lastNotice;
  }
  set notice(value: PageAssistFaultV1 | null) {
    this.revisions.notice++;
    this.lastNotice = value;
  }

  private leaveConversation() {
    this.composerEpoch = ++this.conversationEpoch;
    this.pendingSend = null;
    this.sending = false;
    this.notice = null;
  }

  private beginNavigation(): Navigation {
    let finish!: () => void;
    const settled = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const navigation = { generation: ++this.conversationEpoch, settled, finish };
    this.navigations = [...this.navigations, navigation];
    this.sending = false;
    this.notice = null;
    return navigation;
  }

  private async finishNavigation(navigation: Navigation, switched: boolean) {
    if (!this.navigations.includes(navigation)) return false;
    if (switched) {
      // Superseded successes still changed native state. Read what is current;
      // neither request order nor acknowledgement order determines that state.
      await this.read();
      if (!this.navigations.includes(navigation)) return false;
      // A projection may already have replaced the composer before the ack.
      if (navigation.generation >= this.composerEpoch) this.leaveConversation();
    }
    this.navigations = this.navigations.filter((pending) => pending !== navigation);
    navigation.finish();
    this.sending = this.pendingSend !== null;
    return true;
  }

  private cancelNavigation() {
    const navigations = this.navigations;
    this.navigations = [];
    for (const navigation of navigations) navigation.finish();
  }

  get latest() {
    return latestReply(this.conversation);
  }

  /** A reply is working or waiting on the person: Send becomes Stop. */
  get busy(): boolean {
    const latest = this.latest;
    return !!latest && open(latest);
  }

  /** The page in front can go along with a message. */
  get attachable(): boolean {
    return this.status?.availability === "ready" && !!this.status.page;
  }

  get pageAttached(): boolean {
    return this.attachable && !this.pageOff;
  }

  /** A message can be sent at all: a conversation exists and the panel can talk. */
  get talks(): boolean {
    const availability = this.status?.availability;
    return (
      !!this.conversation.id &&
      availability !== "ai_off" &&
      availability !== "private" &&
      availability !== "no_model"
    );
  }

  /** Listening starts before anything is read, so no event falls between. */
  async start() {
    if (this.active) return;
    this.active = true;
    const stop = await events.pageAssistChanged.listen(({ payload }) => this.receive(payload));
    if (!this.active) {
      stop();
      return;
    }
    this.stopListening = stop;
  }

  /** The Space and the tab in front. A new Space starts from its own snapshot. */
  follow(space: string | null, tab: string | null) {
    if (space !== this.space) {
      if (this.space) void quietly(commands.pageAssistClose(this.space), false);
      this.generation++;
      this.cancelNavigation();
      this.leaveConversation();
      this.space = space;
      this.tab = tab;
      this.status = null;
      this.conversation = EMPTY;
      this.arrivals = {};
      this.picks = [];
      this.others = [];
      this.picking = null;
      this.styles = null;
      this.past = null;
      this.reset();
      this.queue = null;
      this.unavailable = false;
      this.loading = space !== null;
      if (space !== null) void this.read();
      return;
    }
    if (tab === this.tab) return;
    this.tab = tab;
    this.pageOff = false;
    this.selectionOn = false;
    this.styles = null;
    if (tab && this.others.includes(tab))
      this.others = this.others.filter((other) => other !== tab);
    void this.read();
  }

  /** Reads what is true now, without losing the conversation. */
  refresh() {
    if (this.space) void this.read();
  }

  private reset() {
    this.pageOff = false;
    this.selectionOn = false;
    this.notice = null;
  }

  private async read() {
    const space = this.space;
    const tab = this.tab;
    if (!space) return;
    const generation = ++this.generation;
    this.queue ??= [];
    let status: PageAssistStatusV1;
    try {
      status = await commands.pageAssistStatus(space, tab);
    } catch {
      if (generation !== this.generation) return;
      this.unavailable = true;
      this.loading = false;
      this.flush();
      return;
    }
    if (generation !== this.generation) return;
    this.unavailable = !status.conversation.id && status.availability === "unavailable";
    this.status = status;
    if (status.conversation.id !== this.conversation.id) {
      this.leaveConversation();
      this.arrivals = {};
      this.picking = null;
    }
    this.conversation = status.conversation;
    if (!status.selection) this.selectionOn = false;
    this.loading = false;
    this.flush();
    if (status.styled && tab) void this.readStyles();
  }

  private flush() {
    const queued = this.queue ?? [];
    this.queue = null;
    for (const event of queued) this.receive(event);
  }

  private receive(event: PageAssistChangedV1) {
    if (event.space !== this.space) return;
    if (this.queue) {
      this.queue.push(event);
      return;
    }
    const before = this.conversation;
    const next = fold(before, event);
    if (next === "reload") {
      this.refresh();
      return;
    }
    if (!next) return;
    this.conversation = next;
    this.track(before, event);
    const kind = event.kind;
    if (
      kind.type === "block" &&
      kind.value.kind === "style" &&
      kind.value.style.state === "applied"
    )
      void this.readStyles();
  }

  /** Keeps the last few deltas of each streaming block, so their words can ink in. */
  private track(before: Conversation, event: PageAssistChangedV1) {
    const kind = event.kind;
    if (kind.type === "text") {
      const reply = before.messages.find((message) => message.id === event.reply);
      const block = reply?.kind === "reply" ? reply.blocks[kind.block] : undefined;
      const at = block?.kind === "text" ? block.text.length : 0;
      const key = `${event.reply}:${kind.block}`;
      const known = this.arrivals[key] ?? [];
      this.arrivals = {
        ...this.arrivals,
        [key]: [...known, { at, length: kind.delta.length }].slice(-FRESH),
      };
    } else if (kind.type === "done" || kind.type === "failed" || kind.type === "cancelled") {
      const prefix = `${event.reply}:`;
      if (Object.keys(this.arrivals).some((key) => key.startsWith(prefix)))
        this.arrivals = Object.fromEntries(
          Object.entries(this.arrivals).filter(([key]) => !key.startsWith(prefix)),
        );
    }
  }

  private attachments(): PageAssistAttachV1[] {
    const tab = this.tab;
    const out: PageAssistAttachV1[] = [];
    if (tab && this.pageAttached) out.push({ kind: "page", tab });
    for (const other of this.others) if (other !== tab) out.push({ kind: "page", tab: other });
    if (tab && this.selectionOn && this.status?.selection) out.push({ kind: "selection", tab });
    for (const pick of this.picks) out.push({ kind: "element", tab: pick.tab, pick: pick.pick });
    return out.slice(0, MAX_ATTACHMENTS);
  }

  private submittedPicks(attachments: PageAssistAttachV1[]) {
    return this.picks.filter((pick) =>
      attachments.some(
        (attachment) =>
          attachment.kind === "element" &&
          attachment.tab === pick.tab &&
          attachment.pick === pick.pick,
      ),
    );
  }

  /** True when Rust admitted the message; what it says arrives as events. */
  async ask(text: string, suggestion: PageAssistSuggestionV1 | null = null): Promise<boolean> {
    const words = text.trim().slice(0, MESSAGE_LIMIT);
    const space = this.space;
    if (
      !space ||
      (!words && !suggestion) ||
      this.busy ||
      !this.talks ||
      this.sending ||
      this.navigating ||
      this.attachmentsPending
    )
      return false;
    return this.send(space, words, suggestion, this.attachments(), true);
  }

  /** Sends the message a reply answered again, with the pages it had. */
  async retry(reply: number): Promise<boolean> {
    const space = this.space;
    const asked = askedBy(this.conversation, reply);
    if (!space || !asked || this.busy || this.sending || this.navigating) return false;
    const attachments = asked.attachments.flatMap<PageAssistAttachV1>((attachment) =>
      attachment.kind === "page"
        ? [{ kind: "page", tab: attachment.tab }]
        : attachment.kind === "selection"
          ? [{ kind: "selection", tab: attachment.tab }]
          : [],
    );
    return this.send(space, asked.text, asked.suggestion, attachments, false);
  }

  private async send(
    space: string,
    text: string,
    suggestion: PageAssistSuggestionV1 | null,
    attachments: PageAssistAttachV1[],
    composed: boolean,
  ): Promise<boolean> {
    const conversation = this.conversation.id;
    const generation = this.conversationEpoch;
    const turn = {};
    this.pendingSend = turn;
    this.sending = true;
    this.notice = null;
    const submitted = { ...this.revisions };
    const submittedPicks = this.submittedPicks(attachments);
    this.reservedPicks = [...this.reservedPicks, ...submittedPicks];
    const current = () =>
      this.pendingSend === turn &&
      this.space === space &&
      this.conversation.id === conversation &&
      generation >= this.composerEpoch;
    let response: PageAssistAskResponseV1 | null = null;
    try {
      response = await commands.pageAssistAsk({
        space,
        conversation,
        text,
        suggestion,
        attachments,
      });
      // Native consumes these exact submitted objects even across navigation.
      // A later pick may reuse the same tab/token; it has a different lifetime.
      if (composed && response.reply !== null)
        this.picks = this.picks.filter((pick) => !submittedPicks.includes(pick));
      return response.reply !== null;
    } catch {
      return false;
    } finally {
      this.reservedPicks = this.reservedPicks.filter((pick) => !submittedPicks.includes(pick));
      void this.settleComposer(generation, () => {
        if (!current()) return;
        if (!response || response.reply === null) {
          if (this.revisions.notice === submitted.notice)
            this.notice = response?.fault ?? { reason: "unavailable" };
          if (response?.fault?.reason === "invalid") this.refresh();
        } else if (composed) {
          if (this.revisions.others === submitted.others) this.others = [];
          if (this.revisions.page === submitted.page) this.pageOff = false;
          if (this.revisions.selection === submitted.selection) this.selectionOn = false;
        }
        this.pendingSend = null;
        this.sending = false;
      });
    }
  }

  stop() {
    if (this.space) void quietly(commands.pageAssistStop(this.space), false);
  }

  /** Brings the cite's page forward when it is behind, then its words into view. */
  async cite(reply: number, cite: PageAssistCiteV1, active: string | null): Promise<boolean> {
    const id = this.conversation.id;
    if (!id) return false;
    if (cite.tab !== active) await quietly(commands.tabsActivate(cite.tab), null);
    return quietly(commands.pageAssistCite(id, reply, cite.n), false);
  }

  focus(reply: number, block: number, n: number | null) {
    const id = this.conversation.id;
    if (id) void quietly(commands.pageAssistFocus(id, reply, block, n), false);
  }

  decide(reply: number, block: number, decision: PageAssistDecisionV1) {
    const id = this.conversation.id;
    if (id) void quietly(commands.pageAssistDecide(id, reply, block, decision), false);
  }

  resume(reply: number) {
    const id = this.conversation.id;
    if (id) void quietly(commands.pageAssistResume(id, reply), false);
  }

  /** The new Work's id once it started; the block itself follows by events. */
  async continueWork(reply: number, block: number): Promise<string | null> {
    const id = this.conversation.id;
    if (!id) return null;
    const result = await quietly(commands.pageAssistContinue(id, reply, block), {
      work: null,
      fault: { reason: "unavailable" } as const,
    });
    return result.work;
  }

  /** The person points at a part of the page; it joins the next message. */
  async pick(url: string | null): Promise<void> {
    const space = this.space;
    const tab = this.tab;
    if (!space || !tab || this.picking || !this.status?.can_pick) return;
    this.picking = tab;
    this.notice = null;
    const result = await quietly(commands.pageAssistPick(space, tab), {
      pick: null,
      fault: { reason: "unavailable" } as const,
    });
    if (this.picking !== tab || this.space !== space) return;
    this.picking = null;
    if (result.pick) {
      const pick = { ...result.pick, url };
      this.picks = [...this.picks.filter((known) => known.pick !== pick.pick), pick].slice(
        -(MAX_ATTACHMENTS - 2),
      );
    } else if (result.fault) this.notice = result.fault;
  }

  cancelPick() {
    const tab = this.picking;
    if (!tab) return;
    this.picking = null;
    void quietly(commands.pageAssistPickCancel(tab), false);
  }

  detach(pick: Pick) {
    this.picks = this.picks.filter((known) => known !== pick);
    void quietly(commands.pageAssistDetach(pick.tab, pick.pick), false);
  }

  /** Another of the Space's tabs goes along with the next message; the one in front is the page chip. */
  attachTab(tab: string) {
    if (tab === this.tab) this.pageOff = false;
    else if (!this.others.includes(tab)) this.others = [...this.others, tab].slice(-MAX_OTHERS);
  }

  detachTab(tab: string) {
    this.others = this.others.filter((other) => other !== tab);
  }

  /** Picks on a page that navigated or closed went with it, as did closed tabs. */
  forget(urlOf: (tab: string) => string | null | undefined) {
    const kept = this.picks.filter((pick) => urlOf(pick.tab) === pick.url);
    if (kept.length !== this.picks.length) this.picks = kept;
    const open = this.others.filter((tab) => urlOf(tab) !== undefined);
    if (open.length !== this.others.length) this.others = open;
  }

  async newConversation() {
    const space = this.space;
    if (!space || this.conversation.messages.length === 0) return;
    const navigation = this.beginNavigation();
    const created = await quietly(commands.pageAssistNew(space), false);
    await this.finishNavigation(navigation, created);
  }

  async history() {
    if (!this.space) return;
    const view = await quietly(commands.pageAssistHistory(this.space), null);
    this.past = view?.conversations ?? [];
  }

  async openPast(id: string) {
    const space = this.space;
    if (!space) return false;
    if (id === this.conversation.id) return true;
    const navigation = this.beginNavigation();
    const opened = await quietly(commands.pageAssistOpen(space, id), false);
    if (!(await this.finishNavigation(navigation, opened))) return false;
    return opened;
  }

  async deletePast(id: string) {
    if (!this.space) return;
    const deleted = await quietly(commands.pageAssistDelete(this.space, id), false);
    if (!deleted) return;
    if (this.past) this.past = this.past.filter((entry) => entry.id !== id);
    if (id === this.conversation.id) this.refresh();
  }

  deleteCurrent() {
    if (this.conversation.messages.length) return this.deletePast(this.conversation.id);
  }

  async readStyles() {
    const tab = this.tab;
    if (!tab) return;
    const view = await quietly(commands.pageStyles(tab), null);
    if (view && tab === this.tab) this.styles = view;
  }

  async toggleStyle(id: string, enabled: boolean) {
    const tab = this.tab;
    if (!tab) return;
    const view = await quietly(commands.pageStyleToggle(tab, id, enabled), null);
    if (view && tab === this.tab) this.styles = view;
  }

  async removeStyle(id: string) {
    const tab = this.tab;
    if (!tab) return;
    const view = await quietly(commands.pageStyleRemove(tab, id), null);
    if (view && tab === this.tab) this.styles = view;
  }

  dispose() {
    this.cancelPick();
    if (this.space) void quietly(commands.pageAssistClose(this.space), false);
    this.active = false;
    this.generation++;
    this.cancelNavigation();
    this.leaveConversation();
    this.stopListening?.();
    this.stopListening = null;
    this.queue = null;
    this.space = null;
    this.tab = null;
    this.conversation = EMPTY;
    this.status = null;
  }
}
