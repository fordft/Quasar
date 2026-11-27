import * as accountSession from "./account.svelte";
import { initials } from "./account-model";
import { drawn, profiles } from "$domain/profiles";

/** One safe reading of ChatGPT identity and the browser profile's own picture. */
export function accountFoot(options: { private?: () => boolean } = {}) {
  const account = $derived(accountSession.account());
  const signedIn = $derived(account?.signed_in === true);
  const face = $derived.by((): ReturnType<typeof drawn> | null => {
    if (options.private?.()) return null;
    const current = profiles.current();
    return current && current.avatar.revision > 0 ? drawn(current) : null;
  });
  return {
    get available() {
      return account?.available === true && !options.private?.();
    },
    get signedIn() {
      return signedIn;
    },
    get sharing() {
      return account?.sharing === true;
    },
    get email() {
      return account?.email ?? account?.name ?? null;
    },
    /** The first name the account itself carries, never one guessed from the address. */
    get name() {
      const first = signedIn ? (account?.name ?? "").trim().split(/\s+/u)[0] : "";
      return first || null;
    },
    get initials() {
      return signedIn ? initials(account?.email, account?.name) : "";
    },
    get face() {
      return face;
    },
  };
}
