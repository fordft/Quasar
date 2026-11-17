import type { ShortcutRecordingRequest, ShortcutRecordingView } from "$shared/ipc/bindings";

/** Mock native authority; requests and acknowledgements can be delayed separately. */
export function recordingNative() {
  let revision = 0n;
  let owner: string | null = null;
  let granted: string | null = null;
  const view = (): ShortcutRecordingView => ({ revision: String(revision), owner });
  const apply = (request: ShortcutRecordingRequest): ShortcutRecordingView => {
    if (request.kind === "inspect") return view();
    const base = BigInt(request.token.revision);
    const owns = owner === request.token.owner && granted === request.token.revision;
    if (request.kind === "start" && owns) return view();
    const admitted = request.kind === "start" ? revision === base : owns || revision === base;
    if (admitted) {
      revision++;
      if (request.kind === "start") {
        owner = request.token.owner;
        granted = request.token.revision;
      } else if (owns) {
        owner = null;
        granted = null;
      }
    }
    return view();
  };
  return { view, apply };
}

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}
