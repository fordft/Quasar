import "$styles/global.css";
import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import type { AccountV1 } from "$shared/ipc/bindings";
import { accountFixture } from "$shared/testing/account";
import AccountPageStage from "./AccountPageStage.svelte";
const state = vi.hoisted(() => ({ account: null as AccountV1 | null }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ accountGet: async () => ({ account: state.account!, fault: null }) });
});
test("ChatGPT connection states in both themes", async () => {
  await page.viewport(1000, 850);
  const states = [
    accountFixture(),
    accountFixture({ pending: true }),
    accountFixture({
      signed_in: true,
      sharing: true,
      name: "Ada Lovelace",
      email: "ada@example.com",
    }),
  ];
  for (const [index, account] of states.entries()) {
    state.account = account;
    const screen = await render(AccountPageStage);
    await expect.element(screen.getByText("ChatGPT", { exact: true }).first()).toBeVisible();
    for (const theme of ["light", "dark"]) {
      document.documentElement.dataset.theme = theme;
      await page.screenshot({
        path: `../../../../../target/look/account/chatgpt-${index}-${theme}.png`,
      });
    }
    await screen.unmount();
  }
});
