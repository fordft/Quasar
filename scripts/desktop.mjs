import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";

const require = createRequire(new URL("../desktop/package.json", import.meta.url));
const cli = require.resolve("@tauri-apps/cli/tauri.js");
const args = process.argv.slice(2);
if (process.platform === "linux" && !args.includes("--no-default-features")) {
  if (!args.includes("--")) args.push("--");
  args.push("--no-default-features");
}
const child = spawnSync(process.execPath, [cli, ...args], { stdio: "inherit" });
if (child.error) throw child.error;
process.exit(child.status ?? 1);
