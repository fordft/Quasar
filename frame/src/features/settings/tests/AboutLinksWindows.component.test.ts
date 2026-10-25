import { expect, test, vi } from "vitest";
import { render } from "vitest-browser-svelte";
import "$styles/global.css";
import AboutLinks from "../components/about/AboutLinks.svelte";

const native = vi.hoisted(() => ({ open: vi.fn(async () => true) }));

vi.mock("$shared/platform", () => ({ IS_MAC: false, IS_WINDOWS: true }));
vi.mock("$shared/ipc/bindings", async () => {
  const { mockBindings } = await import("$shared/testing/bindings");
  return mockBindings({ browserOpenUrl: native.open as never });
});

test("with no menu bar, About names its documentation Help", async () => {
  const screen = await render(AboutLinks);
  await screen.getByRole("link", { name: "Help", exact: true }).click();
  expect(native.open).toHaveBeenLastCalledWith(
    "https://github.com/fordft/Quasar/tree/main/docs",
    true,
  );
  expect(screen.getByRole("link", { name: "Docs" }).query()).toBeNull();
});
