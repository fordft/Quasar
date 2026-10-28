# Frontend implementation rules

Read `docs/frontend.md` from the repository root. Do not infer completion from this tree alone; the Rust side owns durable state.

- Features expose a small `index.ts`. Put rendered UI and component composition in
  `components/`, supporting behavior/state/selectors in `lib/`, and verification in
  the module's `tests/`. No empty template directories. Component sections remain
  components. A controller does not import rendered components or its own barrel.
- Domain slices keep focused projection/admission/intent source files and `tests/`.
  Session owns transient state with an explicit document lifetime. Neither owns
  durable execution, permissions, secrets or a second copy of Rust entities.
- Across features/domain modules use public APIs and aliases. Inside a module use
  relative imports. Shared primitives have their own entry points; do not assemble
  an eager barrel of every component. Heavy views export loaders.
- Browser tests use `.component.test.ts`; other unit tests use `.test.ts`. Use
  `shared/testing` for native mocks and fixtures. Production entry graphs must never
  import tests, fixtures, or development scenario adapters.
- Work schemas, identities, execution and legal transitions belong to the runtime
  track. Do not invent WorkProjectionV1, WorkCommandV1, WorkSignalV1 or artifact
  wire types. Display-only component types are distinct and must say so.
- Preserve synchronous tab-presentation sentinels, fixed-raster favicons, native
  chrome/page separation, CSP and caller scope. Update `desktop/src/frame_sources.rs`
  whenever a source anchor moves. Never hand-edit generated IPC bindings.
- Use semantic tokens and component-owned styling for new UI. Maintain keyboard
  access, logical CSS, error/retry states and reduced motion. Error boundaries catch
  render/effect failures; asynchronous commands still need explicit settlement.
- `bundle-budgets.json` limits complete emitted static graphs. New lazy destinations
  require measured budgets; do not auto-raise limits to hide a regression. A reviewed
  increase records its reason with the feature change.
- Run `pnpm -C frame check`, relevant browser-component tests, `pnpm -C frame build`,
  and `cargo xtask check-frame-styles`. Run native checks when desktop contracts
  change. A known unrelated full-CI failure does not authorize bypassing its gate or
  stopping independent frontend work. Report it separately.
- Identify a native app by this checkout's executable/process before interacting;
  other Quasar worktrees may be running. Static checks are not native visual,
  interaction, resource or cross-platform qualification.
- Respect the user's current commit/push scope. Preserve unrelated existing changes
  and do not use a shared stash or bulk checkpoint commit to reorganize dirty work.
