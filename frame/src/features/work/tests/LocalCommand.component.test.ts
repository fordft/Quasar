import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import type { WorkCommandRecordV1, WorkStepFact } from "$shared/ipc/bindings";
import CommandReview from "../components/local/CommandReview.svelte";
import CommandRecord from "../components/local/CommandRecord.svelte";
import WorkFileInspector from "../components/WorkFileInspector.svelte";
import { WorkSession } from "$domain/work";
import { projection } from "./environment-fixtures";

const proposal: WorkStepFact = {
  id: "command",
  turn: 1,
  kind: {
    kind: "run_command",
    cwd: "/Users/person/project",
    command: "npm test && printf done",
    timeout_secs: null,
    decision: null,
  },
  status: "running",
  usage: null,
  artifacts: [],
  evidence: null,
  note: null,
  measurements: null,
  local: {
    policy: {
      class: "write",
      reason: "project_execution",
      scope: "folder",
      root: "/Users/person/project",
    },
  },
};

test("the inspector loads command review and sends its decision to the execution", async () => {
  const session = new WorkSession("profile");
  const execution = structuredClone(projection.executions[0]!);
  execution.status = "running";
  execution.steps = [structuredClone(proposal)];
  session.projection = { ...structuredClone(projection), executions: [execution] };
  const approve = vi.spyOn(session, "approveStep").mockResolvedValue(true);
  const ondecided = vi.fn();
  const screen = await render(WorkFileInspector, {
    proposal: { session, step: proposal.id },
    ondecided,
  });
  await screen.getByRole("button", { name: "Approve", exact: true }).click();
  expect(approve).toHaveBeenCalledExactlyOnceWith(proposal.id, true);
  await expect.poll(() => ondecided.mock.calls.length).toBe(1);
  await screen.unmount();
  session.dispose();
});

test("command review shows the exact line and folder boundary and preserves an unsuccessful decision", async () => {
  const ondecision = vi
    .fn()
    .mockRejectedValueOnce(new Error("unavailable"))
    .mockResolvedValue(undefined);
  const screen = await render(CommandReview, { step: proposal, ondecision });
  await expect.element(screen.getByText("npm test && printf done", { exact: true })).toBeVisible();
  await expect.element(screen.getByText("~/project", { exact: true })).toBeVisible();
  await expect
    .element(
      screen.getByText(
        "Commands can reach anything your account can; this only decides when Quasar asks.",
      ),
    )
    .toBeVisible();
  await screen.getByRole("button", { name: "Approve", exact: true }).click();
  await expect.element(screen.getByRole("alert")).toBeVisible();
  await screen.getByRole("button", { name: "Decline", exact: true }).click();
  expect(ondecision.mock.calls.map(([approve]) => approve)).toEqual([true, false]);
});

test("command record shows failure and copies only the recorded output", async () => {
  const copy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  const record: WorkCommandRecordV1 = {
    id: "record",
    node: "node",
    attempt: "attempt",
    command: {
      cwd: "/Users/person/project",
      command: "npm test",
      exit: 1,
      signal: null,
      elapsed_ms: 1250,
      bytes: 99999,
      digest: "a".repeat(64),
      text: "beginning\nlast failure",
      truncated: true,
    },
  };
  const screen = await render(CommandRecord, { record });
  await expect.element(screen.getByText("Exit 1 · 1.3 s", { exact: true })).toBeVisible();
  await expect
    .element(screen.getByText("Showing the beginning and end of 99999 output bytes."))
    .toBeVisible();
  await screen.getByRole("button", { name: "Copy output" }).click();
  expect(copy).toHaveBeenCalledExactlyOnceWith(record.command.text);
  await expect.element(screen.getByRole("status")).toHaveTextContent("Copied");
});
