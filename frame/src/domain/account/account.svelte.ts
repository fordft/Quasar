/** Native ChatGPT session projection. Tokens and authorization URLs stay native. */
import {
  commands,
  type AccountFaultV1,
  type AccountReplyV1,
  type AccountV1,
} from "$shared/ipc/bindings";
import { events } from "$shared/ipc/native-events";
import { observe } from "$shared/lib/observe";

type Action = "sign_in" | "cancel" | "select" | "sign_out" | "cleanup";
let current = $state.raw<AccountV1 | null>(null);
let unreachable = $state(false);
let busy = $state<Action | null>(null);
let lastFault = $state.raw<{ action: Action; fault: AccountFaultV1 } | null>(null);
let users = 0;
let generation = 0;
let sequence = 0;
let stop: (() => void) | null = null;
let queued = false;
let reading = false;

export const account = () => current;
export const unavailable = () => unreachable || current?.available === false;
export const pending = () => busy;
export const fault = () => lastFault;

function adopt(reply: AccountReplyV1 | undefined) {
  if (reply?.account?.version !== 1 || !Array.isArray(reply.account.profiles)) {
    unreachable = true;
    return;
  }
  unreachable = false;
  current = reply.account;
}
async function read() {
  if (busy || reading) {
    queued = true;
    return;
  }
  reading = true;
  const started = generation;
  const turn = ++sequence;
  const result = await observe(
    Promise.resolve().then(() => commands.accountGet()),
    15000,
  );
  if (started !== generation) return;
  reading = false;
  if (turn === sequence) {
    if (result.state === "received") adopt(result.value);
    else unreachable = true;
  }
  if (queued && !busy) {
    queued = false;
    void read();
  }
}
export async function retain() {
  users++;
  if (users > 1) {
    if (!current || unreachable) void read();
    return;
  }
  const started = ++generation;
  const listener = await events.accountChanged.listen(() => void read());
  if (started !== generation || users === 0) {
    listener();
    return;
  }
  stop = listener;
  await read();
}
export function release() {
  users = Math.max(0, users - 1);
  if (users) return;
  generation++;
  sequence++;
  stop?.();
  stop = null;
  reading = false;
  queued = false;
  busy = null;
  lastFault = null;
  current = null;
  unreachable = false;
}
async function act(action: Action, request: () => Promise<AccountReplyV1>) {
  if (busy) return false;
  const started = generation;
  sequence++;
  busy = action;
  lastFault = null;
  const result = await observe(Promise.resolve().then(request), 30000);
  if (started !== generation) return false;
  busy = null;
  if (result.state === "received") {
    adopt(result.value);
    lastFault = result.value.fault ? { action, fault: result.value.fault } : null;
  } else {
    lastFault = { action, fault: "timed_out" };
    // The native action may have completed; read the durable result.
    queued = true;
  }
  if (queued) {
    queued = false;
    void read();
  }
  return result.state === "received" && !result.value.fault;
}
export function signIn(profileId: string | null = null, newProfile = false, consent = false) {
  return act("sign_in", () => commands.accountSignIn(profileId, newProfile, consent));
}
export const cancel = () => act("cancel", () => commands.accountCancelSignIn());
export const select = (id: string) => act("select", () => commands.accountSelect(id));
export const signOut = () => act("sign_out", () => commands.accountSignOut());
export const retryCleanup = () => act("cleanup", () => commands.accountRetryCleanup());
export async function manageUsage() {
  const result = await observe(
    Promise.resolve().then(() => commands.accountManageUsage()),
    10000,
  );
  return result.state === "received" && result.value;
}
