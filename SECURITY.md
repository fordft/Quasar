# Security Policy

Quasar treats security as the top priority. A browser is a hostile environment by nature, and we'd rather hear about a problem privately than read about it publicly.

## Supported versions

Only the latest release receives security fixes.

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Anything older | No |

## Local secrets and browser data

The ChatGPT OAuth session is AES-256-GCM encrypted with a random 32-byte key
held in the macOS login Keychain or Windows Credential Manager, and MCP
connection secrets (including OAuth tokens and environment values marked
secret) live in the same OS-managed vaults, not plaintext settings files.
Retired provider API keys appear only as leftover entries the first-run
cleanup removes; Quasar never stores new ones. Credential Manager is scoped to
the Windows user, not to the Quasar executable; it does not exclude malicious
processes running as that user. Quasar does not maintain its own
website-password vault.

On Windows, WebView2 owns cookies and site storage beneath
`%APPDATA%\app.quasar\web-content\profiles\<id>\EBWebView` (the development
identity and Work stores use separate directories). Quasar restricts its profile
root and individual profile-directory DACLs to the current user and SYSTEM before
admitting them, refusing access if restriction fails. The inheritable permissions also
apply to new children and Windows propagates them to existing inheriting children;
explicit or protected child ACLs are not reset. This is filesystem access control,
not encryption or protection against other processes running as the same user.

WebView2 password autosave and general autofill are disabled and read back before
use. This prevents new saves, but does not erase older saved passwords or prevent
their autofill, as documented by
[Microsoft's settings API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings4).
WebView2 owns its cookie encryption and keys; Quasar adds no app-bound layer.
The pinned SDK exposes no supported app-bound encryption or custom storage-key
hook. DPAPI-wrapped cookie keys do not protect against another process running as
the user. Site storage (including localStorage and IndexedDB) and extension
storage are not Quasar-encrypted vaults and may contain tokens saved by sites or
extensions. Profile separation does not make them separate OS security principals.
See Microsoft's [user data folder](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder)
and [profile API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2profile)
documentation. Chromium's [app-bound protection](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/browser/os_crypt/app_bound_encryption_win.cc)
depends on its privileged service and supported installation, not a WebView2
setting that Quasar can enable.

On macOS, WKWebView and WebKit own website cookies, site data and native extension
storage; system password autofill and passkeys remain system-owned. App-owned
engine directories use owner-only Unix permissions. Quasar does not add a
separate encryption layer to WebKit data. On either platform, general local data
such as history, notes, conversations, downloads and connection settings is not
a secret vault: sensitive text entered there, or connection values not marked
secret, can be persisted as ordinary data. OS-account isolation and disk
encryption remain relevant on both platforms; neither is a promise of resistance
to malware already running as the user.

## Reporting a vulnerability

**Do not open a public issue for security problems.**

Report privately with GitHub's [private vulnerability reporting](https://github.com/fordft/Quasar/security/advisories/new).

Include:

- A description of the issue and its impact.
- Steps to reproduce (a proof of concept helps).
- Affected version / commit, and your platform (macOS or Windows).

We'll keep you updated and credit you in the release notes unless you'd rather stay anonymous.

Please give us a reasonable window to fix the issue before any public disclosure. We practice coordinated disclosure and won't take legal action against good-faith research.
