/** Display-only initials; identity and permissions come from verified native state. */
export function initials(email?: string | null, name?: string | null): string {
  const label = name?.trim() || email?.split("@")[0] || "";
  return label
    .split(/\s+/u)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => Array.from(word)[0])
    .join("")
    .toLocaleUpperCase();
}
