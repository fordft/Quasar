import { afterEach, expect, test, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import type { AccountV1 } from "$shared/ipc/bindings";
import { accountFixture } from "$shared/testing/account";
import NewTabLook from "./NewTabLook.svelte";

const native = vi.hoisted(() => ({
  account: null as unknown as AccountV1,
  kind: "default" as "default" | "incognito",
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    accountGet: async () => ({ account: native.account, fault: null, retry_after: null }),
    settingGet: async () => null,
    newtabSearchContext: async () => ({
      window_id: "window",
      profile_id: "00000000000000000000000501",
      space_id: "space",
      session_id: "newtab:tab:1",
      request_id: "",
    }),
    newtabSearch: async () => true,
    newtabCancel: async () => true,
    newtabRun: async () => ({ accepted: true, operation_id: null }),
    blockerStats: async () => ({ status: "error" as const, error: null }),
    timeCall: async () => ({ kind: "focus_days" as const, days: [] }),
    resourceCall: async () => {
      throw new Error("no tasks here");
    },
  });
});
vi.mock("$domain/tabs", () => ({
  tabs: {
    profile: () => ({ id: "00000000000000000000000501", name: "Ada", kind: native.kind }),
  },
}));

const onaccount = vi.fn();
const shots = "../../../../../target/look/newtab-account";
const settled = () =>
  Promise.all(document.getAnimations().map((animation) => animation.finished.catch(() => null)));

const signedIn = (email: string) => accountFixture({ signed_in: true, sharing: true, email });

afterEach(() => {
  native.kind = "default";
  delete document.documentElement.dataset.theme;
});

async function stage(account: AccountV1) {
  native.account = account;
  onaccount.mockClear();
  await page.viewport(860, 560);
  const screen = await render(NewTabLook, { onaccount });
  screen.container.style.cssText = "inline-size:820px;block-size:520px;padding:6px";
  await new Promise((done) => setTimeout(done, 300));
  return screen;
}

async function shoot(screen: Awaited<ReturnType<typeof stage>>, name: string) {
  for (const theme of ["dark", "light"]) {
    document.documentElement.dataset.theme = theme;
    screen.container.style.background = "var(--color-canvas)";
    await settled();
    await page
      .elementLocator(screen.container)
      .screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
  document.documentElement.dataset.theme = "dark";
}

test("signed out, the corner offers ChatGPT", async () => {
  const screen = await stage(accountFixture());
  await expect
    .element(screen.getByRole("button", { name: "Continue with ChatGPT", exact: true }))
    .toBeVisible();
  await shoot(screen, "signed-out");
});
test("the connected account has no credit balance or upgrade menu", async () => {
  const screen = await stage(signedIn("ada@example.com"));
  await screen.getByRole("button", { name: "Account", exact: true }).click();
  await expect.element(screen.getByText("Using ChatGPT plan", { exact: true })).toBeVisible();
  expect(page.getByRole("menuitem", { name: "Buy credits" }).query()).toBeNull();
  await shoot(screen, "connected");
});

test("a long address is cut short, never the panel widened", async () => {
  const screen = await stage(signedIn("ada.lovelace@analytical-engine.example"));
  await screen.getByRole("button", { name: "Account", exact: true }).click();
  const panel = document.querySelector<HTMLElement>(".ntp-account-panel")!;
  await expect.poll(() => panel.inert).toBe(false);
  expect(panel.offsetWidth).toBe(272);
  await shoot(screen, "long-open");
});

test("a private window and an install without the service show no account", async () => {
  native.kind = "incognito";
  let screen = await stage(signedIn("ada@example.com"));
  expect(screen.container.querySelectorAll(".control")).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Account", exact: true }).query()).toBeNull();
  await screen.unmount();

  native.kind = "default";
  screen = await stage(accountFixture({ available: false }));
  expect(screen.getByRole("button", { name: "Sign in", exact: true }).query()).toBeNull();
  expect(screen.container.querySelectorAll(".control")).toHaveLength(1);
});
