import { commands } from "$shared/ipc/bindings";
import type { WorkDecisionChoiceV1, WorkDecisionPreferenceV1 } from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";
import { observe } from "$shared/lib/observe";

const TIMEOUT = 9000;

/**
 * The profile's typed-decision preference. Rust owns both the stored choice and
 * what actually runs, so this only ever mirrors the last answer it gave: a
 * refused read never becomes a local guess at the effective mode.
 */
export class WorkDecisionSession {
  preference = $state.raw<WorkDecisionPreferenceV1 | null>(null);
  unavailable = $state(false);
  busy = $state(false);
  private readSequence = 0;
  private active = false;
  private generation = 0;
  private stop: (() => void) | null = null;
  private lifetime = new AbortController();
  constructor(readonly profile: string) {}

  /** Subscribe before the first read: an invalidation in between is never lost. */
  async start() {
    if (this.active) return;
    this.active = true;
    const generation = ++this.generation;
    this.lifetime = new AbortController();
    const stop = await events.workDecisionPreferenceChanged.listen(({ payload }) => {
      if (payload.profile === this.profile) void this.read();
    });
    if (!this.active || generation !== this.generation) {
      stop();
      return;
    }
    this.stop = stop;
    await this.read();
  }

  private async read() {
    const generation = this.generation;
    const sequence = ++this.readSequence;
    const response = await observe(
      Promise.resolve().then(() => commands.workDecisionPreference(this.profile)),
      TIMEOUT,
      this.lifetime.signal,
    );
    if (response.state !== "received") {
      if (generation === this.generation && sequence === this.readSequence) this.unavailable = true;
      return;
    }
    if (sequence !== this.readSequence) return;
    this.adopt(generation, response.value);
  }

  /** Applied to the next read; a running read keeps the configuration it has. */
  async choose(choice: WorkDecisionChoiceV1) {
    if (this.busy) return;
    this.busy = true;
    ++this.readSequence;
    const generation = this.generation;
    const response = await observe(
      Promise.resolve().then(() => commands.workSetDecisionPreference(this.profile, choice)),
      TIMEOUT,
      this.lifetime.signal,
    );
    if (generation === this.generation) this.busy = false;
    ++this.readSequence;
    if (response.state !== "received") {
      if (generation === this.generation) this.unavailable = true;
      return;
    }
    this.adopt(generation, response.value);
  }

  private adopt(generation: number, response: WorkDecisionPreferenceV1) {
    if (!this.active || generation !== this.generation) return;
    if (response.version !== 1 || response.profile !== this.profile || response.error) {
      this.unavailable = true;
      return;
    }
    this.unavailable = false;
    this.preference = response;
  }

  dispose() {
    this.active = false;
    this.generation++;
    this.stop?.();
    this.stop = null;
    this.lifetime.abort();
    this.preference = null;
    this.busy = false;
    ++this.readSequence;
  }
}
