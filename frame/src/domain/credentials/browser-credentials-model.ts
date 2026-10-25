import type { BrowserPasskeyAuthorizationView } from "$shared/ipc/bindings";

/** Closed user copy for one browser-native passkey capability state. */
export function browserPasskeyStatus(
  state: BrowserPasskeyAuthorizationView | null | undefined,
): string {
  switch (state) {
    case "authorized":
      return "Passkeys are enabled for Quasar.";
    case "denied":
      return "Passkey access is disabled in System Settings.";
    case "not_determined":
      return "Enable passkeys to use system and third-party credential providers.";
    case "entitlement_required":
      return "Passkeys require an approved macOS browser build.";
    case "unknown":
    case "unavailable":
      return "Passkey status is unavailable on this system.";
    default:
      return "Passkeys are not provided by this platform integration.";
  }
}
