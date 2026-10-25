# ChatGPT connection

Quasar uses Sign in with ChatGPT as its only product AI connection. On macOS
and Windows, open Settings → Account (also available under Settings → AI) and
choose **Continue with ChatGPT**. Complete sign-in in your system browser and
allow ChatGPT plan usage. An eligible ChatGPT Plus or Pro plan is required.
Identity sign-in alone does not enable AI. **Manage usage** opens ChatGPT's own
usage settings. Quasar does not receive the user's ChatGPT conversation history.

Linux remains a browsing-only preview: account authorization, page AI and Work
are unavailable there. Removing the old API-key UI does not enable a Linux AI
backend. Dictation uses the on-device recognizer; plan authorization does not
also grant the Audio Transcriptions API.

## Native boundary

`crates/quasar-chatgpt` owns OAuth, verified identity, profiles, refresh and
protected storage. `desktop/src/account.rs` owns the system-browser handoff and
privileged IPC. The Svelte frame sees display state and closed faults only:
never a bearer, refresh token, authorization code, ID token or authorization URL.

The native runtime binds `127.0.0.1` on an ephemeral port before opening the
browser. Every attempt has fresh state, nonce and S256 PKCE. The redirect is
`http://127.0.0.1:<port>/auth/callback`; its path, Host header and state are checked.
A newly issued client ID is durably saved before code exchange. A failed exchange
can reuse that registration. Returning identities must match issuer and subject,
with ID-token RS256 signature, audience, expiration and nonce validation.
Registrations are kept distinct even if they have the same email.

An installation host UUID persists across restarts and account switches. Session
records are AES-256-GCM encrypted with a random 32-byte key in the macOS login
Keychain or Windows Credential Manager. The record file and lock are private on
Unix. A file lease serializes refresh across cooperating processes; atomic
replacement commits rotated access token, refresh token, expiry and scope
metadata together. Inaccessible or corrupt credentials fail closed.

Model discovery uses authenticated `GET https://api.openai.com/v1/models` and
only visible account entries become selectable. Ask, Work, chosen page agents
and native model search use account-bound clients at
`https://api.openai.com/v1/responses`, with `store: false`, `stream: true` and a
required `response.completed` event. There is no development API-key fallback
in product composition. Unknown account model capabilities use bounded text and
tool defaults; unreviewed vision and native search are disabled.

Switching or signing out advances the native account generation, cancels old
model streams and prevents old clients from obtaining another account's bearer.
Sign-out clears local tokens and ID-token hints before attempting refresh-token
revocation, so local erasure survives cancellation during the network operation. In that case the UI says to remove
the connection in ChatGPT. The host ID, issued client and verified identity remain
for a later sign-in. Cancel also stops an in-flight authorization exchange.

## Retiring upstream credentials

The first startup retires only known Zephium account vault items (default and
saved custom origin), OpenAI/Anthropic/other provider keys, the old TypeSafe key,
and related AI account/endpoint cache settings. Both upstream and fork service
names are covered. The completion marker is written only after all operations
succeed. A refused vault operation remains visible with a retry action; retired
credentials are never used as a fallback. No upstream sign-in, billing, receipt,
avatar or account-deletion request is made.

Website passwords, cookies, browser profiles, MCP OAuth, notes and Work history
are preserved. Old serialized model/provider/cost identifiers remain readable;
they cannot authorize requests or become selectable. The assistant's existing
direct-call cost record shape is preserved, with provider `ChatGPT` for new plan
usage and no API dollar or Zephium-credit claim. Work's existing conservative
budget reservations remain local execution limits, not a ChatGPT invoice.

Rolling back code does not restore retired keys or account tokens. Existing
browser and Work records are retained; old releases may display new ChatGPT cost
records as an unknown estimate. Reconnecting to an old provider would require
entering its credentials again in that old release.

## Validation

Unit fixtures cover callback binding, PKCE and code exchange, JWT verification,
concurrent token rotation, invalid grants, loss of plan permission, account
switches, cancellation during exchange, discovery filtering, and local sign-out
when remote revocation fails. Fixture HTTP endpoints exist only under `cfg(test)`.
Frontend component tests exercise connection, consent, cancellation, switching,
sign-out and the ChatGPT-only model picker. Native tests regenerate IPC bindings.

Live account qualification still requires a person using an eligible ChatGPT
account on macOS or Windows: sign in, approve plan usage, ask about a real web
page, restart, try Work, and sign out during a running answer. Automated fixtures
and successful builds do not establish live OAuth or inference success.

## Official contract

- [OpenAI cookbook: Sign in with ChatGPT](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt)
- [Open-source ChatGPT plan usage](https://developers.openai.com/siwc/token-sharing-open-source)
- [Profiles and sessions](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)
- [Models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)
