# Quasar frame

Read [the frontend contract](../docs/frontend.md) before changing browser chrome.

- `app`: compose surfaces and independent feature snippets.
- `features`: own a product concept's presentation and local interaction.
- `session`: share temporary state within one WebView document.
- `domain`: mirror Rust projections and send typed intents.
- `shared`: primitives, transport and helpers with no product authority.

Imports flow down that list. Features do not import siblings. Import feature/domain
public APIs through `index.ts` and the `$features`/`$domain` aliases; use relative
imports within a module. Add a projection in Rust, regenerate bindings, then add its
mirror. Add a primitive with semantic tokens and focused interaction tests. Add a
surface at the app root, with a lazy loader and an explicit native hosting contract.
Never place privileged chrome over page-WebView pixels.

```sh
pnpm -C frame check              # types, boundaries, styles, formatting, dead code, unit tests
pnpm -C frame test:component     # WebKit on macOS; Chromium on Windows
pnpm -C frame build              # production graph, no fixtures
cargo xtask check-frame-styles   # emitted CSS, requires the build
cargo xtask ci                  # full repository gate
```

Tests live beside their sources. Use `shared/testing` for native mocks and fixtures;
these cannot enter production bundles. Native startup, presentation, focus, CSP,
resource use and cross-platform behavior require separate bundled-app evidence.
