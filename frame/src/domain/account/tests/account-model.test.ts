import { expect, test } from "vitest";
import { initials } from "../account-model";
test("ChatGPT identity labels use a name or the email prefix", () => {
  expect(initials("ada@example.com", "Ada Lovelace")).toBe("AL");
  expect(initials("ada@example.com")).toBe("A");
  expect(initials(null)).toBe("");
});
