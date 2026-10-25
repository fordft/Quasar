import { afterEach, expect, test, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import primitives from "../../../styles/tokens/primitive.css?raw";
import tokens from "../../../styles/tokens.css?raw";
import browserCSS from "../../../styles/browser.css?raw";
import { resourceTestServer } from "$shared/testing/resources/server";
import { taskSession } from "$domain/resources";
import { dayKey } from "../lib/task-sections";
import { setPageView } from "../lib/page-view.svelte";
import PageHost from "./PageHost.svelte";

const native = vi.hoisted(() => ({
  call: vi.fn(),
  open: vi.fn(),
  profile: "00000000000000000000000071",
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ resourceCall: native.call, browserOpenUrl: native.open });
});
vi.mock("$domain/tabs", () => ({
  tabs: {
    profile: () => ({ id: native.profile }),
    activeTab: () => ({ url: "https://github.com/zephium/browser", title: "Quasar on GitHub" }),
  },
}));
vi.mock("$domain/surface", () => ({ surface: { open: vi.fn() } }));
const style = document.createElement("style");
style.textContent = primitives + tokens.replace("@theme static", ":root") + browserCSS;
document.head.append(style);
afterEach(() => {
  delete document.documentElement.dataset.theme;
});

async function setup(scope: "today" | "all" = "today") {
  native.profile = String(BigInt(native.profile) + 1n).padStart(26, "0");
  const server = resourceTestServer(native.profile);
  native.call.mockImplementation(server.call);
  setPageView({ scope, trashed: false, board: false, list: null }, native.profile);
  const session = taskSession(native.profile, "page");
  await session.create({
    title: "Review the browser task experience",
    dueDate: dayKey(new Date()),
    context: { url: "https://github.com/zephium/browser", title: "Browser design review" },
  });
  await session.create({ title: "Write the launch notes", dueDate: dayKey(new Date()) });
  const screen = await render(PageHost);
  await expect.element(screen.getByText("Review the browser task experience")).toBeVisible();
  return { screen, session, server };
}

test("full page opens the selected task from board and preserves editing", async () => {
  await page.viewport(1440, 900);
  document.documentElement.dataset.theme = "dark";
  const { screen, server } = await setup();
  await page.screenshot({ path: "../../../../../target/tasks-qa/page-list-dark.png" });
  // Date views hold open tasks only, so the board is offered where Done can fill.
  expect(screen.container.querySelector("[role=radiogroup]")).toBeNull();
  await screen.getByRole("button", { name: /All tasks/u }).click();
  await screen.getByRole("radio", { name: "Board", exact: true }).click();
  await screen
    .getByRole("button", { name: "Review the browser task experience", exact: true })
    .click();
  await expect
    .element(screen.getByRole("textbox", { name: "Rename", exact: true }))
    .toHaveValue("Review the browser task experience");
  await screen
    .getByRole("textbox", { name: "Description", exact: true })
    .fill("Check narrow panels, keyboard navigation, and recovery after failed saves.");
  await expect
    .poll(() =>
      [...server.records.values()].some(
        (r) =>
          r.draft.content.kind === "task" && r.draft.content.description.startsWith("Check narrow"),
      ),
    )
    .toBe(true);
  await page.screenshot({ path: "../../../../../target/tasks-qa/page-dark.png" });
  await screen.getByRole("button", { name: "Close task details" }).click();
  await expect
    .element(screen.getByRole("textbox", { name: "Rename", exact: true }))
    .not.toBeInTheDocument();
});

test("page capture opens an editable composer and keeps the source separate", async () => {
  await page.viewport(1440, 900);
  const { screen, server } = await setup();
  await screen.getByRole("textbox", { name: "New task", exact: true }).click();
  await screen.getByRole("button", { name: "Attach this page", exact: true }).click();
  await page.screenshot({ path: "../../../../../target/tasks-qa/page-capture.png" });
  await expect
    .element(screen.getByRole("button", { name: "Remove linked page", exact: true }))
    .toBeVisible();
  const field = screen.getByRole("textbox", { name: "New task", exact: true });
  await field.fill("Review the pull request");
  await field.click();
  await expect.element(field).toHaveValue("Review the pull request");
  await userEvent.keyboard("{Enter}");
  await expect
    .poll(() =>
      [...server.records.values()].map((r) => ({
        title: r.draft.title,
        context: r.draft.content.kind === "task" ? r.draft.content.context?.url : null,
      })),
    )
    .toContainEqual({
      title: "Review the pull request",
      context: "https://github.com/zephium/browser",
    });
});

test("search leaves navigation counts intact and narrow detail returns to the list", async () => {
  await page.viewport(900, 900);
  document.documentElement.dataset.theme = "light";
  const { screen, session } = await setup();
  await screen.getByRole("searchbox", { name: "Search tasks", exact: true }).fill("browser");
  await expect.poll(() => session.rows.length).toBe(1);
  expect(session.counts.today).toBe(2);
  await screen
    .getByRole("button", { name: "Review the browser task experience", exact: true })
    .click();
  await expect.element(screen.getByRole("textbox", { name: "Rename", exact: true })).toBeVisible();
  await page.screenshot({ path: "../../../../../target/tasks-qa/page-narrow.png" });
  await screen.getByRole("button", { name: "Back to tasks" }).click();
  await expect
    .element(
      screen.getByRole("button", { name: "Review the browser task experience", exact: true }),
    )
    .toBeVisible();
});

test("lists, priorities and subtasks form a persistent task workflow", async () => {
  await page.viewport(1500, 940);
  document.documentElement.dataset.theme = "dark";
  const { screen, session, server } = await setup("all");
  await screen.getByRole("button", { name: "New list", exact: true }).first().click();
  const name = screen.getByRole("textbox", { name: "List name", exact: true });
  await name.fill("Quasar");
  await name.click();
  await userEvent.keyboard("{Enter}");
  await expect.poll(() => session.lists.length).toBe(1);
  const list = session.lists[0]!;
  await session.saveList("Personal");
  await session.saveList("Reading");
  const id = (await session.create({
    title: "Bring Tasks to the quality of the browser",
    list: list.id,
    dueDate: dayKey(new Date()),
    priority: "high",
    context: { url: "https://github.com/zephium/browser", title: "Quasar · task experience" },
  }))!;
  await session.create({
    title: "Refine the new tab experience",
    list: list.id,
    dueDate: dayKey(new Date()),
  });
  await session.create({ title: "Review keyboard navigation", list: list.id });
  await session.create({ title: "Write the release notes", list: list.id });
  await session.reload();
  await screen
    .getByRole("button", { name: "Bring Tasks to the quality of the browser", exact: true })
    .click();
  const detail = screen.getByRole("complementary", { name: "Task detail" });
  await detail
    .getByRole("textbox", { name: "Description", exact: true })
    .fill(
      "A quiet place to plan your day and keep the context that matters.\n\nReview the full page, the compact panel, and the transitions between them.",
    );
  const subtask = detail.getByRole("textbox", { name: "Add a subtask…", exact: true });
  await subtask.fill("Review the full-page layout");
  await subtask.click();
  await userEvent.keyboard("{Enter}");
  await expect
    .poll(() => {
      const task = server.records.get(id)?.draft.content;
      return task?.kind === "task" ? task.details.steps?.length : null;
    })
    .toBe(1);
  await subtask.fill("Check the narrow browser panel");
  await subtask.click();
  await userEvent.keyboard("{Enter}");
  await expect
    .element(detail.getByRole("checkbox", { name: "Review the full-page layout", exact: true }))
    .toBeVisible();
  await detail.getByRole("checkbox", { name: "Review the full-page layout", exact: true }).click();
  await expect
    .poll(() => {
      const task = server.records.get(id)?.draft.content;
      return task?.kind === "task" ? task.details.steps?.[0]?.completed : null;
    })
    .toBe(true);
  await page.screenshot({ path: "../../../../../target/tasks-qa/organized-dark.png" });
  document.documentElement.dataset.theme = "light";
  await page.screenshot({ path: "../../../../../target/tasks-qa/organized-light.png" });
  await session.flush();
  await session.reload();
  expect(session.rows.find((row) => row.id === id)?.priority).toBe("high");
  expect(session.rows.find((row) => row.id === id)?.stepDone).toBe(1);
});
