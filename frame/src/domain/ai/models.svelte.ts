import { commands } from "$shared/ipc/bindings";
import type {
  WorkModelReasoning,
  WorkModelRole,
  WorkModelsFaultV1,
  WorkModelsV1,
} from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";
import { observe } from "$shared/lib/observe";

const TIMEOUT = 9000;

/** Native account-authorized models and profile-scoped choices. */
export class ModelsSession {
  models = $state.raw<WorkModelsV1 | null>(null);
  unavailable = $state(false);
  /** The action in flight, such as `key:anthropic` or `choose:lead`. */
  busy = $state<string | null>(null);
  /** What the last action could not do, with the action it belongs to. */
  fault = $state.raw<{ action: string; fault: WorkModelsFaultV1 } | null>(null);
  private readSequence = 0;
  private readPending = false;
  private reading = false;
  private active = false;
  private generation = 0;
  private stop: (() => void) | null = null;
  private lifetime = new AbortController();
  constructor(readonly profile: string) {}

  /** Subscribe before the first read: a change in between is never lost. */
  async start() {
    if (this.active) return;
    this.active = true;
    const generation = ++this.generation;
    this.lifetime = new AbortController();
    const stop = await events.workModelsChanged.listen(() => void this.read());
    if (!this.active || generation !== this.generation) {
      stop();
      return;
    }
    this.stop = stop;
    await this.read();
  }

  private async read() {
    if (this.busy || this.reading) {
      this.readPending = true;
      return;
    }
    this.reading = true;
    const sequence = ++this.readSequence;
    const generation = this.generation;
    const response = await observe(
      Promise.resolve().then(() => commands.workModels(this.profile)),
      TIMEOUT,
      this.lifetime.signal,
    );
    if (generation !== this.generation) return;
    this.reading = false;
    if (sequence === this.readSequence) {
      if (response.state !== "received") this.unavailable = true;
      else this.adopt(response.value);
    }
    // Changes that arrived during the read are answered by one more, not one each.
    if (this.readPending && !this.busy) {
      this.readPending = false;
      void this.read();
    }
  }

  private adopt(response: WorkModelsV1) {
    if (response.version !== 1 || response.profile !== this.profile) {
      this.unavailable = true;
      return;
    }
    if (response.fault === "unavailable") {
      this.unavailable = true;
      return;
    }
    this.unavailable = false;
    // An identical answer is not a change: dependants would ask again for nothing.
    if (this.models && JSON.stringify(this.models) === JSON.stringify(response)) return;
    this.models = response;
  }

  private async act(action: string, request: () => Promise<WorkModelsV1>, timeout = TIMEOUT) {
    if (this.busy) return false;
    this.busy = action;
    this.readSequence++;
    this.fault = null;
    const generation = this.generation;
    const response = await observe(Promise.resolve().then(request), timeout, this.lifetime.signal);
    if (generation !== this.generation) return false;
    this.busy = null;
    if (response.state !== "received") {
      this.fault = { action, fault: "unreachable" };
      if (this.readPending) {
        this.readPending = false;
        void this.read();
      }
      return false;
    }
    this.adopt(response.value);
    const fault = response.value.fault;
    if (fault && fault !== "unavailable") this.fault = { action, fault };
    if (this.readPending) {
      this.readPending = false;
      void this.read();
    }
    return !fault;
  }

  choose(role: WorkModelRole, id: string | null) {
    return this.act(`choose:${role}`, () => commands.workChooseModel(this.profile, role, id));
  }

  /** The lead's effort for this profile; null returns it to each model's default. */
  setEffort(effort: WorkModelReasoning | null) {
    return this.act("effort", () => commands.workSetModelEffort(this.profile, effort));
  }

  refresh() {
    return this.act(
      "refresh",
      async () => {
        await commands.workMoreModels(this.profile, "open_ai");
        return commands.workModels(this.profile);
      },
      30000,
    );
  }

  dispose() {
    this.active = false;
    this.generation++;
    this.stop?.();
    this.stop = null;
    this.lifetime.abort();
    this.models = null;
    this.busy = null;
    this.fault = null;
    this.readPending = false;
    this.reading = false;
    this.readSequence++;
  }
}
