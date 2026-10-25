import type { RuntimeSecurityAdvisory } from "$shared/ipc/bindings";

const RELEASES = "https://github.com/fordft/Quasar/releases/tag/v";
const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/u;

export const releaseNotesUrl = (version: string) => `${RELEASES}${encodeURIComponent(version)}`;

function comparePrerelease(a: string | undefined, b: string | undefined): number {
  if (a === b) return 0;
  // A release outranks every prerelease of the same version.
  if (a === undefined) return 1;
  if (b === undefined) return -1;
  const left = a.split(".");
  const right = b.split(".");
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const x = left[index];
    const y = right[index];
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    const xNumeric = /^\d+$/u.test(x);
    const yNumeric = /^\d+$/u.test(y);
    if (xNumeric && yNumeric) {
      const difference = Number(x) - Number(y);
      if (difference !== 0) return Math.sign(difference);
    } else if (xNumeric !== yNumeric) {
      return xNumeric ? -1 : 1;
    } else if (x !== y) {
      return x < y ? -1 : 1;
    }
  }
  return 0;
}

/** Semantic-version order; null when either side is not a version. */
export function compareVersions(a: string, b: string): number | null {
  const left = SEMVER.exec(a);
  const right = SEMVER.exec(b);
  if (!left || !right) return null;
  for (let part = 1; part <= 3; part += 1) {
    const difference = Number(left[part]) - Number(right[part]);
    if (difference !== 0) return Math.sign(difference);
  }
  return comparePrerelease(left[4], right[4]);
}

export type SystemUpdateTarget = "operating_system" | "browser_runtime";

/** The platform update the runtime recommends, if any. Review notices stay
 *  internal: they ask nothing a person can act on. */
export function systemUpdateTarget(
  advisories: readonly RuntimeSecurityAdvisory[],
): SystemUpdateTarget | null {
  for (const advisory of advisories.slice(0, 5)) {
    if (advisory.kind !== "update_recommended") continue;
    if (advisory.update_target === "operating_system") return "operating_system";
    if (advisory.update_target === "browser_runtime") return "browser_runtime";
  }
  return null;
}
