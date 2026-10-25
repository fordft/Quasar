import "$styles/global.css";
import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import { page } from "vitest/browser";
import type { AccountV1 } from "$shared/ipc/bindings";
import { accountFixture } from "$shared/testing/account";
import { emitNativeEvent } from "$shared/testing/native-events";
import AccountPage from "../components/sections/AccountPage.svelte";
const native = vi.hoisted(() => ({
  get: vi.fn(),
  signIn: vi.fn(),
  cancel: vi.fn(),
  signOut: vi.fn(),
  select: vi.fn(),
  usage: vi.fn(),
}));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({
    accountGet: native.get,
    accountSignIn: native.signIn,
    accountCancelSignIn: native.cancel,
    accountSignOut: native.signOut,
    accountSelect: native.select,
    accountManageUsage: native.usage,
  });
});
const reply = (account: AccountV1) => ({ account, fault: null });
const connected = () =>
  accountFixture({
    signed_in: true,
    sharing: true,
    active: "one",
    name: "Ada",
    email: "ada@example.com",
    profiles: [
      { id: "one", label: "Ada", email: "ada@example.com", signed_in: true, sharing: true },
    ],
  });

test("ChatGPT sign-in opens natively and only a native callback connects the account", async () => {
  await page.viewport(1000, 850);
  native.get.mockResolvedValue(reply(accountFixture()));
  native.signIn.mockResolvedValue(reply(accountFixture({ pending: true })));
  native.cancel.mockResolvedValue(reply(accountFixture()));
  const screen = await render(AccountPage);
  await screen.getByRole("button", { name: "Continue with ChatGPT", exact: true }).click();
  expect(native.signIn).toHaveBeenCalledWith(null, false, false);
  await expect
    .element(screen.getByText("Finish signing in in your browser", { exact: true }))
    .toBeVisible();
  await expect
    .element(screen.getByText("Using ChatGPT plan", { exact: true }))
    .not.toBeInTheDocument();
  await expect.element(screen.getByRole("textbox")).not.toBeInTheDocument();
  await screen.getByRole("button", { name: "Cancel sign-in" }).click();
  expect(native.cancel).toHaveBeenCalled();
  native.get.mockResolvedValue(reply(connected()));
  emitNativeEvent("accountChanged", { version: 1 });
  await expect.element(screen.getByText("Using ChatGPT plan", { exact: true })).toBeVisible();
  await expect
    .element(screen.getByRole("button", { name: "Manage usage", exact: true }))
    .toBeVisible();
});

test("identity-only sign-in requests plan consent without claiming AI access", async () => {
  native.get.mockResolvedValue(reply({ ...connected(), sharing: false }));
  native.signIn.mockResolvedValue(reply({ ...connected(), sharing: false, pending: true }));
  const screen = await render(AccountPage);
  await expect
    .element(screen.getByText("ChatGPT plan usage has not been granted.", { exact: true }))
    .toBeVisible();
  await expect
    .element(screen.getByText("Using ChatGPT plan", { exact: true }))
    .not.toBeInTheDocument();
  await screen.getByRole("button", { name: "Continue with ChatGPT", exact: true }).click();
  expect(native.signIn).toHaveBeenLastCalledWith("one", false, true);
});

test("account switching, usage and sign-out are native intents", async () => {
  const two = {
    ...connected(),
    profiles: [
      ...connected().profiles,
      { id: "two", label: "Grace", email: null, signed_in: true, sharing: true },
    ],
  };
  native.get.mockResolvedValue(reply(two));
  native.select.mockResolvedValue(reply({ ...two, active: "two", name: "Grace", email: null }));
  native.signOut.mockResolvedValue(reply(accountFixture()));
  native.usage.mockResolvedValue(true);
  const screen = await render(AccountPage);
  await screen.getByRole("button", { name: "Manage usage", exact: true }).click();
  expect(native.usage).toHaveBeenCalled();
  await screen.getByRole("button", { name: "Use this account" }).click();
  expect(native.select).toHaveBeenCalledWith("two");
  await screen.getByRole("button", { name: "Sign out", exact: true }).click();
  expect(native.signOut).toHaveBeenCalled();
  await expect
    .element(screen.getByRole("button", { name: "Continue with ChatGPT", exact: true }))
    .toBeVisible();
});

test("a failed native request cannot claim a connected account", async () => {
  native.get.mockResolvedValue(reply(accountFixture()));
  native.signIn.mockRejectedValue(new Error("native unavailable"));
  const screen = await render(AccountPage);
  await screen.getByRole("button", { name: "Continue with ChatGPT", exact: true }).click();
  await expect.element(screen.getByRole("alert")).toBeVisible();
  await expect
    .element(screen.getByText("Using ChatGPT plan", { exact: true }))
    .not.toBeInTheDocument();
});
