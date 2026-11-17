import {
  commands,
  type ShortcutRecordingRequest,
  type ShortcutRecordingToken,
  type ShortcutRecordingView,
} from "$shared/ipc/bindings";
import { keymap } from "$domain/keymap";
import { observe } from "$shared/lib/observe";
import { SvelteSet } from "svelte/reactivity";

export const RECORDING_ACK_MS = 5000;
type Phase = "idle" | "preparing" | "recording" | "stopping" | "failed";
type Turn = { token: ShortcutRecordingToken | null; abort: AbortController };
let active: { owner: object; stop: () => void } | null = null;

function valid(view: ShortcutRecordingView | null): view is ShortcutRecordingView {
  return (
    !!view &&
    typeof view.revision === "string" &&
    view.revision.length <= 20 &&
    /^(0|[1-9]\d*)$/u.test(view.revision) &&
    (view.owner === null || typeof view.owner === "string")
  );
}

/** One renderer owner at a time; native CAS admission remains authoritative. */
export class ShortcutRecorder {
  phase = $state<Phase>("idle");
  failed = $state(false);
  private turn: Turn | null = null;
  private readonly cancelled = new SvelteSet<ShortcutRecordingToken>();
  private generation = 0;
  private readonly identity = {};

  constructor(private readonly kind: "launcher" | "keymap") {}

  get engaged() {
    return (
      (this.phase === "preparing" || this.phase === "recording") &&
      active?.owner === this.identity &&
      this.turn !== null
    );
  }
  get ready() {
    return this.engaged && this.phase === "recording";
  }
  get pending() {
    return this.phase === "preparing" || this.phase === "stopping";
  }
  get token() {
    return this.turn?.token ?? null;
  }
  get cleanupPending() {
    return this.cancelled.size > 0;
  }

  private confirmCleanup(view: ShortcutRecordingView) {
    for (const token of this.cancelled) {
      if (view.owner !== token.owner && BigInt(view.revision) > BigInt(token.revision))
        this.cancelled.delete(token);
    }
  }

  private send(request: ShortcutRecordingRequest) {
    return Promise.resolve().then(() =>
      this.kind === "launcher" ? commands.launcherRecordShortcut(request) : keymap.record(request),
    );
  }

  watch<T>(request: Promise<T>) {
    return observe(request, RECORDING_ACK_MS, this.turn?.abort.signal);
  }

  async start(): Promise<boolean> {
    const previous = active;
    previous?.stop();
    if (previous?.owner !== this.identity && this.cancelled.size > 0) void this.stop();
    const turn: Turn = { token: null, abort: new AbortController() };
    const started = ++this.generation;
    this.turn = turn;
    active = {
      owner: this.identity,
      stop: () => {
        void this.stop();
      },
    };
    this.failed = false;
    this.phase = "preparing";
    const current = () =>
      this.turn === turn && active?.owner === this.identity && this.generation === started;
    const inspected = await this.watch(this.send({ kind: "inspect" }));
    if (!current()) return false;
    if (inspected.state !== "received" || !valid(inspected.value)) {
      this.failed = true;
      await this.stop();
      return false;
    }
    // Store the token before invoking Start. Cleanup can now cancel this exact
    // intent even when Start has not arrived or its acknowledgement is lost.
    const token = { owner: crypto.randomUUID(), revision: inspected.value.revision };
    turn.token = token;
    const admitted = await this.watch(this.send({ kind: "start", token }));
    if (!current()) return false;
    if (
      admitted.state === "received" &&
      valid(admitted.value) &&
      admitted.value.owner === token.owner &&
      BigInt(admitted.value.revision) > BigInt(token.revision)
    ) {
      this.confirmCleanup(admitted.value);
      this.phase = "recording";
      return true;
    }
    this.failed = true;
    await this.stop();
    return false;
  }

  async stop(): Promise<boolean> {
    const turn = this.turn;
    if (!turn && this.cancelled.size === 0) return this.phase === "idle";
    const retrying = turn === null;
    this.turn = null;
    if (active?.owner === this.identity) active = null;
    const stopped = ++this.generation;
    turn?.abort.abort();
    if (turn?.token) this.cancelled.add(turn.token);
    const tokens = [...this.cancelled];
    if (tokens.length === 0) {
      this.phase = "idle";
      return true;
    }
    this.phase = "stopping";
    // Never queue Stop behind Inspect, Start, an apply, or another Stop's ACK.
    // Native owner/revision validation makes late requests harmless.
    await Promise.all(
      tokens.map(async (token) => {
        const released = await observe(this.send({ kind: "stop", token }), RECORDING_ACK_MS);
        if (released.state === "received" && valid(released.value))
          this.confirmCleanup(released.value);
      }),
    );
    const confirmed = tokens.every((token) => !this.cancelled.has(token));
    if (this.generation === stopped) {
      this.phase = confirmed ? "idle" : "failed";
      if (!confirmed) this.failed = true;
      else if (retrying) this.failed = false;
    }
    return confirmed;
  }
}
