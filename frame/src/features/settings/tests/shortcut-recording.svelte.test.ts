import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { ShortcutRecordingRequest, ShortcutRecordingView } from "$shared/ipc/bindings";
import { deferred, recordingNative } from "./shortcut-recording-native";
import { RECORDING_ACK_MS, ShortcutRecorder } from "../lib/shortcut-recording.svelte";

const native = vi.hoisted(() => ({ command: vi.fn(), launcher: vi.fn() }));
vi.mock("$domain/keymap", () => ({ keymap: { record: native.command } }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ launcherRecordShortcut: native.launcher });
});

let authority = recordingNative();
const flush = async () => {
  for (let i = 0; i < 12; i++) await Promise.resolve();
};
beforeEach(() => {
  authority = recordingNative();
  for (const send of [native.command, native.launcher])
    send.mockReset().mockImplementation(async (request) => authority.apply(request));
});
afterEach(() => vi.useRealTimers());

test.each(["keymap", "launcher"] as const)(
  "%s retries a rejected Stop after transport recovers",
  async (kind) => {
    const send = kind === "keymap" ? native.command : native.launcher;
    const recorder = new ShortcutRecorder(kind);
    await recorder.start();
    const token = recorder.token;
    send.mockRejectedValueOnce(new Error("Stop was not delivered"));
    expect(await recorder.stop()).toBe(false);
    expect(authority.view().owner).toBe(token?.owner);
    const attempts = send.mock.calls.length;
    expect(await recorder.stop()).toBe(true);
    expect(send).toHaveBeenCalledTimes(attempts + 1);
    expect(send).toHaveBeenLastCalledWith({ kind: "stop", token });
    expect(authority.view().owner).toBeNull();
    expect(recorder.phase).toBe("idle");
  },
);

test("an old cleanup retry does not block or release a newer recorder", async () => {
  const retry = deferred<ShortcutRecordingView>();
  let delayed!: ShortcutRecordingRequest;
  const first = new ShortcutRecorder("keymap");
  const second = new ShortcutRecorder("launcher");
  await first.start();
  native.command.mockRejectedValueOnce(new Error("Stop was not delivered"));
  expect(await first.stop()).toBe(false);
  native.command.mockImplementationOnce(async (request) => {
    delayed = request;
    return retry.promise;
  });
  const retrying = first.stop();
  await flush();
  const delivered = delayed !== undefined;
  expect(await second.start()).toBe(true);
  const current = authority.view();
  if (delivered) retry.resolve(authority.apply(delayed));
  else retry.resolve(current);
  const confirmed = await retrying;
  expect(authority.view()).toEqual(current);
  expect(second.ready).toBe(true);
  await second.stop();
  expect(delivered).toBe(true);
  expect(confirmed).toBe(true);
});

test("a newer admission retires only obsolete cleanup and ignores its late retry acknowledgement", async () => {
  const retry = deferred<ShortcutRecordingView>();
  const recorder = new ShortcutRecorder("keymap");
  await recorder.start();
  const old = recorder.token!;
  native.command.mockRejectedValueOnce(new Error("Stop was not delivered"));
  expect(await recorder.stop()).toBe(false);
  let delayed!: ShortcutRecordingRequest;
  native.command.mockImplementation(async (request: ShortcutRecordingRequest) => {
    if (request.kind === "stop" && request.token.owner === old.owner) {
      delayed = request;
      return retry.promise;
    }
    return authority.apply(request);
  });
  expect(await recorder.start()).toBe(true);
  const current = authority.view();
  expect(recorder.cleanupPending).toBe(false);
  retry.resolve(authority.apply(delayed));
  await flush();
  expect(recorder.ready).toBe(true);
  expect(authority.view()).toEqual(current);
  await recorder.stop();
});

test.each(["keymap", "launcher"] as const)(
  "%s stays preparing until native suspension is acknowledged",
  async (kind) => {
    const ack = deferred<ShortcutRecordingView>();
    const send = kind === "keymap" ? native.command : native.launcher;
    send.mockImplementation((request: ShortcutRecordingRequest) => {
      const view = authority.apply(request);
      return request.kind === "start" ? ack.promise : Promise.resolve(view);
    });
    const recorder = new ShortcutRecorder(kind);
    const starting = recorder.start();
    await flush();
    expect(recorder.phase).toBe("preparing");
    expect(recorder.ready).toBe(false);
    expect(authority.view().owner).not.toBeNull();
    ack.resolve(authority.view());
    expect(await starting).toBe(true);
    expect(recorder.ready).toBe(true);
    expect(await recorder.stop()).toBe(true);
  },
);

test("Stop arriving before delayed Start invalidates its idle base revision", async () => {
  const ack = deferred<ShortcutRecordingView>();
  let delayed!: ShortcutRecordingRequest;
  native.launcher.mockImplementation(async (request: ShortcutRecordingRequest) => {
    if (request.kind === "start") {
      delayed = request;
      return ack.promise;
    }
    return authority.apply(request);
  });
  const recorder = new ShortcutRecorder("launcher");
  const starting = recorder.start();
  await flush();
  expect(authority.view()).toEqual({ revision: "0", owner: null });
  expect(await recorder.stop()).toBe(true);
  expect(authority.view()).toEqual({ revision: "1", owner: null });
  ack.resolve(authority.apply(delayed));
  expect(await starting).toBe(false);
  expect(authority.view().owner).toBeNull();
  expect(recorder.phase).toBe("idle");
});

test("Stop after an applied Start does not wait for its lost acknowledgement", async () => {
  const ack = deferred<ShortcutRecordingView>();
  let applied!: ShortcutRecordingView;
  native.command.mockImplementation(async (request: ShortcutRecordingRequest) => {
    const view = authority.apply(request);
    if (request.kind === "start") {
      applied = view;
      return ack.promise;
    }
    return view;
  });
  const recorder = new ShortcutRecorder("keymap");
  const starting = recorder.start();
  await flush();
  expect(authority.view().owner).not.toBeNull();
  expect(await recorder.stop()).toBe(true);
  expect(authority.view().owner).toBeNull();
  ack.resolve(applied);
  expect(await starting).toBe(false);
  expect(recorder.ready).toBe(false);
});

test.each(["keymap", "launcher"] as const)(
  "a stale %s Stop cannot release the other recorder's newer lease",
  async (firstKind) => {
    const ack = deferred<ShortcutRecordingView>();
    let delayed!: ShortcutRecordingRequest;
    const firstSend = firstKind === "keymap" ? native.command : native.launcher;
    firstSend.mockImplementation(async (request: ShortcutRecordingRequest) => {
      if (request.kind === "stop") {
        delayed = request;
        return ack.promise;
      }
      return authority.apply(request);
    });
    const first = new ShortcutRecorder(firstKind);
    const second = new ShortcutRecorder(firstKind === "keymap" ? "launcher" : "keymap");
    expect(await first.start()).toBe(true);
    expect(await second.start()).toBe(true);
    const current = authority.view();
    expect(first.ready).toBe(false);
    expect(second.ready).toBe(true);
    ack.resolve(authority.apply(delayed));
    await flush();
    await first.stop();
    expect(authority.view()).toEqual(current);
    expect(second.ready).toBe(true);
    await second.stop();
  },
);

test("a native revision mismatch refuses arming and preserves the foreign owner", async () => {
  native.launcher.mockImplementation(async (request: ShortcutRecordingRequest) => {
    if (request.kind === "start")
      authority.apply({
        kind: "start",
        token: { owner: "newer-recorder", revision: request.token.revision },
      });
    return authority.apply(request);
  });
  const recorder = new ShortcutRecorder("launcher");
  expect(await recorder.start()).toBe(false);
  expect(recorder.failed).toBe(true);
  expect(recorder.ready).toBe(false);
  expect(authority.view().owner).toBe("newer-recorder");
});

test("cancellation while Inspect is pending never issues Start or a native Stop", async () => {
  const inspecting = deferred<ShortcutRecordingView>();
  native.command.mockReturnValue(inspecting.promise);
  const recorder = new ShortcutRecorder("keymap");
  const starting = recorder.start();
  await flush();
  await recorder.stop();
  inspecting.resolve(authority.view());
  expect(await starting).toBe(false);
  expect(native.command).toHaveBeenCalledTimes(1);
  expect(native.command).toHaveBeenCalledWith({ kind: "inspect" });
  expect(authority.view().owner).toBeNull();
});

test("a lost Start acknowledgement times out and sends owned cleanup independently", async () => {
  vi.useFakeTimers();
  const ack = deferred<ShortcutRecordingView>();
  let applied!: ShortcutRecordingView;
  native.launcher.mockImplementation(async (request: ShortcutRecordingRequest) => {
    const view = authority.apply(request);
    if (request.kind === "start") {
      applied = view;
      return ack.promise;
    }
    return view;
  });
  const recorder = new ShortcutRecorder("launcher");
  const starting = recorder.start();
  await flush();
  await vi.advanceTimersByTimeAsync(RECORDING_ACK_MS + 1);
  expect(await starting).toBe(false);
  expect(recorder.failed).toBe(true);
  expect(authority.view().owner).toBeNull();
  ack.resolve(applied);
  await flush();
  expect(recorder.ready).toBe(false);
  expect(vi.getTimerCount()).toBe(0);
});

test("a missing Stop acknowledgement is reported as unconfirmed and cannot block a new owner", async () => {
  vi.useFakeTimers();
  const ack = deferred<ShortcutRecordingView>();
  let released!: ShortcutRecordingView;
  native.command.mockImplementation(async (request: ShortcutRecordingRequest) => {
    const view = authority.apply(request);
    if (request.kind === "stop") {
      released = view;
      return ack.promise;
    }
    return view;
  });
  const first = new ShortcutRecorder("keymap");
  await first.start();
  const stopping = first.stop();
  await flush();
  expect(first.phase).toBe("stopping");
  await vi.advanceTimersByTimeAsync(RECORDING_ACK_MS + 1);
  expect(await stopping).toBe(false);
  expect(first.phase).toBe("failed");
  expect(first.failed).toBe(true);
  const second = new ShortcutRecorder("launcher");
  expect(await second.start()).toBe(true);
  ack.resolve(released);
  await flush();
  expect(second.ready).toBe(true);
  await second.stop();
  expect(vi.getTimerCount()).toBe(0);
});
