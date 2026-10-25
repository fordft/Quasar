import presetData from "$shared/ipc/theme-presets.json";

export type Seeds = { bg: string; fg: string; accent: string };
export type Mode = "light" | "dark";
/** A theme's seeds as stored in `ui.theme-custom`. Mirrors
 *  `zephium_core::theme`, which validates every write. Contrast is its own
 *  setting, `ui.theme-contrast`, and travels with a theme only when shared. */
export type ThemeSpec = { light: Seeds; dark: Seeds; from?: string };
export type SharedTheme = { spec: ThemeSpec; contrast: number };
export type Preset = {
  id: string;
  name: string;
  light: Seeds;
  dark: Seeds;
  ground: Record<Mode, string>;
};

export const DEFAULT_THEME = "zephium";
export const CUSTOM_THEME = "custom";
export const DEFAULT_CONTRAST = 50;
const MAX_CUSTOM_BYTES = 320;
const SHARE_PREFIX = "zephium-theme:v1:";
const HEX = /^#[0-9a-f]{6}$/;

export const presets: readonly Preset[] = presetData;

export const preset = (id: string) => presets.find((entry) => entry.id === id);

const specOf = (entry: Preset): ThemeSpec => ({
  light: { ...entry.light },
  dark: { ...entry.dark },
  from: entry.id,
});

/** `ui.theme-contrast` as a number; anything else is the default. */
export function contrastOf(text: string): number {
  return /^(?:100|[1-9]?[0-9])$/.test(text) ? Number(text) : DEFAULT_CONTRAST;
}

function seeds(value: unknown): Seeds | null {
  if (typeof value !== "object" || value === null) return null;
  const keys = Object.keys(value);
  if (keys.length !== 3) return null;
  const { bg, fg, accent } = value as Record<string, unknown>;
  for (const hex of [bg, fg, accent]) if (typeof hex !== "string" || !HEX.test(hex)) return null;
  return { bg: bg as string, fg: fg as string, accent: accent as string };
}

function record(value: unknown, allowed: string[]): Record<string, unknown> | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const entries = value as Record<string, unknown>;
  return Object.keys(entries).every((key) => allowed.includes(key)) ? entries : null;
}

function specFrom(entries: Record<string, unknown>): ThemeSpec | null {
  const light = seeds(entries.light);
  const dark = seeds(entries.dark);
  const { from } = entries;
  if (!light || !dark) return null;
  if (from !== undefined && (typeof from !== "string" || !preset(from))) return null;
  return from === undefined ? { light, dark } : { light, dark, from };
}

function json(text: string): unknown {
  if (text.length === 0 || text.length > MAX_CUSTOM_BYTES) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/** Strict, as core is: unknown keys, uppercase hex or an unknown origin fail. */
export function parseTheme(text: string): ThemeSpec | null {
  const entries = record(json(text), ["light", "dark", "from"]);
  return entries && specFrom(entries);
}

const pick = ({ bg, fg, accent }: Seeds) => ({ bg, fg, accent });

export function serializeTheme(spec: ThemeSpec): string {
  return JSON.stringify({
    light: pick(spec.light),
    dark: pick(spec.dark),
    ...(spec.from ? { from: spec.from } : {}),
  });
}

/** A theme as one pasteable string: its seeds and its contrast. */
export function encodeShare({ spec, contrast }: SharedTheme): string {
  const body = JSON.stringify({
    light: pick(spec.light),
    dark: pick(spec.dark),
    contrast,
    ...(spec.from ? { from: spec.from } : {}),
  });
  return SHARE_PREFIX + btoa(body).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function decodeShare(text: string): SharedTheme | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith(SHARE_PREFIX)) return null;
  const body = trimmed.slice(SHARE_PREFIX.length);
  if (!/^[A-Za-z0-9_-]+$/.test(body)) return null;
  let decoded: string;
  try {
    decoded = atob(body.replaceAll("-", "+").replaceAll("_", "/"));
  } catch {
    return null;
  }
  const entries = record(json(decoded), ["light", "dark", "contrast", "from"]);
  const spec = entries && specFrom(entries);
  const contrast = entries?.contrast;
  if (!spec || typeof contrast !== "number" || !Number.isInteger(contrast)) return null;
  return contrast >= 0 && contrast <= 100 ? { spec, contrast } : null;
}

export type Resolved = { id: string; spec: ThemeSpec; custom: boolean };

/** The theme the stored pair names. An unknown or broken one is Quasar. */
export function resolveTheme(id: string, custom: string): Resolved {
  if (id === CUSTOM_THEME) {
    const spec = parseTheme(custom);
    if (spec) return { id, spec, custom: true };
  }
  const entry = preset(id) ?? preset(DEFAULT_THEME)!;
  return { id: entry.id, spec: specOf(entry), custom: false };
}

export const forkOf = (entry: Preset): ThemeSpec => specOf(entry);

function channels(hex: string): [number, number, number] {
  const value = Number.parseInt(hex.slice(1), 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}

function luminance(hex: string): number {
  const [r, g, b] = channels(hex).map((channel) => {
    const c = channel / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const toHex = (rgb: number[]) =>
  `#${rgb.map((channel) => Math.round(channel).toString(16).padStart(2, "0")).join("")}`;

/** Ink to set on the accent: white, or the accent taken nearly to black,
 *  whichever reads better against it. */
export function onAccent(accent: string): string {
  const lum = luminance(accent);
  const dark = toHex(channels(accent).map((channel) => channel * 0.16));
  const contrast = (a: number, b: number) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  return contrast(lum, 1) >= contrast(lum, luminance(dark)) ? "#ffffff" : dark;
}

/** The custom properties a scope needs to derive its whole ladder. */
export function seedVars(spec: ThemeSpec, contrast: number): Record<string, string> {
  const vars: Record<string, string> = { "--seed-contrast": String(contrast) };
  for (const mode of ["light", "dark"] as const) {
    const { bg, fg, accent } = spec[mode];
    vars[`--seed-${mode}-bg`] = bg;
    vars[`--seed-${mode}-fg`] = fg;
    vars[`--seed-${mode}-accent`] = accent;
    vars[`--seed-${mode}-on-accent`] = onAccent(accent);
  }
  return vars;
}

export const sameSpec = (a: ThemeSpec, b: ThemeSpec) =>
  serializeTheme({ ...a, from: undefined }) === serializeTheme({ ...b, from: undefined });
