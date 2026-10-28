# The sidebar assistant (page assist): design as built

Status as of 2026-10-23, branch `feat/work-production`. The assistant is named **Quasar** in the product ("Ask Quasar…"); the code keeps the subsystem name `page_assist`. It replaced the one-shot Ask panel of 2026-10-22.

## Shape

- **One conversation per Space** that follows the person across tabs. The page in front is an attachment (a chip), not the conversation's owner. Conversations persist per profile in the profile store (`Store::{chats, chat, save_chat, delete_chat, clear_chats}`, 200 per profile, cleared with browsing data, never in private windows) and reopen from history.
- **One streamed model loop per message** (`zephium-work-composition/src/page_assist/agent.rs`) on `LeadClient` through `resolve_entry_for(…, Page, WorkCallSource::Page{conversation, reply})`, at the person's global effort. Up to 8 calls and 16 tool calls per reply, 90 s per call. Text streams as markdown, coalesced to one delivery per 50 ms; `[a12]` references become cite marks `⟦n⟧` as they stream (`core::stream`).
- **Tools** (`tools.rs`): `list_tabs`, `read_page`, `show_items` (cards + numbered marks), `open_tabs`, `act` (light steps), `inspect_style`, `restyle`, `hand_off`. Every argument is the model's words; the service admits each against the reply's own reads.
- **Contract**: `crates/zephium-ipc/src/page_assist.rs` (status, ask, blocks, events with `seq`), commands in `desktop/src/page_assist.rs`.

## Security rules

- Page, selection, element and outline text, and every tool result, are fenced untrusted data (`core::prompt::fence`); page text cannot forge cite marks (`text::clean` drops them).
- `open_tabs` opens only addresses that a read in this conversation produced (link `href` from the reader, `http(s)` only) or that the person wrote; at most 8, in the background, never focused.
- Item URLs and marks come from read passages only, never from model text. Other tabs are read only by handle, never sensitive sites, never private windows.
- Steps keep their rules: protected fields and cross-site links refused, Commit steps wait for the person's confirm, human input pauses them (lease).
- Styles are CSS only, validated (`core::css`) and guarded (`core::guard`), applied only after Apply. Layout properties are allowed (flex, grid tracks, order, table parts as blocks); `content` holds only `""`, counters and a fixed separator set — free text, `attr()`, `url()` and `:has()` are refused.
- `hand_off` is a preview card: nothing runs until "Continue in Work". Work gets the title, summary, the conversation (bounded) and the tabs as context.

## Page layer (engine)

Reads carry link targets; marks are numbered badges and item outlines in one closed, top-layer overlay positioned against the initial containing block (no script per document scroll); the picker is shared with the blocker (`host/picker_kit.js`) and runs in the reader's private world for the assistant. Probe: `macos-page-reader-probe --assist`. Reads use the bounded extraction budget (256 items, 48 KiB) so a whole listing page fits.

## Not done

1. Windows reader, marks, picks and steps on a real machine (compiled, not run).
2. A live run of the launch-video script on the real app.

## Reader: chosen design

Founder bar: lighter than protection-off on CPU and RAM; no user activation from automation; Rust is the only source of program bytes.

### macOS — evaluate first: bootstrap-only

- One **~1 KB document-start `WKUserScript`** per Browse view's user content controller, main frame only, in a **private `WKContentWorld`** created for the view. It holds exactly one pending reply pull on a dedicated reply handler (for example `zephiumPageAssistBootV1`) and does nothing else.
- When the person asks, Rust answers that pull with the fixed runtime program (`SEMANTIC_RUNTIME_PROGRAM.source()`, the pinned constant, never page-derived) and the bootstrap instantiates it inside the private world (for example with `Function`). The runtime then speaks the existing closed channel (`P1`/`R1:`/`A1`/`S1`) on its own reply handler in the same world.
- **Verify before building on it**, with loopback fixtures in a probe (never on the founder's screen):
  - a page served with `Content-Security-Policy: script-src 'self'` (no `'unsafe-eval'`): instantiation in the private world still works;
  - a page that calls `window.open()` right after our pull answer and polls `navigator.userActivation.isActive`: stays blocked and inactive;
  - the page world cannot observe the private world's globals or the handler.
- **If WebKit refuses** instantiation under page CSP: fall back to installing the unmodified runtime wrapped in a never-called function at document start (lazy pre-parse only), and measure first: A/B over about ten real pages for load time and WebContent footprint. Accept only at ≤ ~1 ms main-thread and ≤ ~250 KB per page; otherwise report the numbers.
- Either way: never install on sensitive sites (`zephium_app::work_sites::is_sensitive`), in private profiles, or with AI off; remove the script and handlers from the controller when AI is switched off. Tabs whose document predates installation report `loading` until their next navigation.
- The owned-view registration (`AgentSemanticRuntimeRegistration`) cannot be reused: it removes all user scripts and attests exactly one. Build a sibling, for example `platform/macos/browse_reader.rs`, reusing `AgentSemanticRuntimeController`'s state machine through a narrow constructor export from `semantic_runtime.rs` (pinned file: keep the edit minimal, and keep `cargo xtask check-agentic-probe-boundary` green).
- Epoch binding: begin/commit document loads from the Browse navigation observer (`host/navigation.rs`, `NavigationEpochTracker`), so a read or step answered for an older document is discarded; the desktop `TabFacts.epoch` must come from the same committed epoch rather than the URL-change counter it uses today.
- Teardown: navigation (begin load invalidates), tab close (retire), idle (stop the runtime's pull after a few minutes without asks; the bootstrap pull remains), AI off (remove).

#### As built (macOS)

- The fixtures held: under `script-src 'self'`, Trusted Types and `default-src 'self'`, `Function` instantiates the program in the private world; activation and `hasBeenActive` stay false and no popup reaches native while the page polls; the page sees no private global or handler. No fallback was needed.
- **One private world per view**, not one per document: rotating the world on every navigation attempt cost 2.3–4 ms page-process CPU per load and kept memory. Instead every instantiation gets a fresh channel and handler registration bound to the committed epoch, and a commit removes the old document's handler. The script is added or removed (WebKit's `_removeAllUserScriptsAssociatedWithContentWorld:`, probed) only when the next document's admission changes: sensitive site, AI switch.
- The program is `installation_source()` + `page_reader_recipes.js` + `page_reader_serve.js`. Recipes are `Q1:selection`, `Q1:outline`, `Q1:mark:<json string>`, carried by one closed pending kind in the pinned runtime (`PendingInvocation::Recipe`) plus `browse_channel`, its constructor export.
- A pull that arrives while a navigation is under way stays unbound until its commit, so a read can never open the incoming document under the outgoing epoch.
- The mark is a CSS Custom Highlight under the quote (no element, no reflow; amber tuned for light or dark grounds; eases in, holds 2.6 s, fades). A page can see that a highlight with a random name and one adopted sheet exist while it shows; nothing it can see names the panel.
- Measured (release build, headless): per page load +0.3–0.5 ms page-process CPU, no measurable load time or memory difference (alternating blocks in one process). First ask 15–23 ms on a short article, 62–70 ms on a 3,000-paragraph page, 86 ms on a live Wikipedia article; later reads 2.5 ms, 28 ms, 36 ms; selection 2 ms; outline 5–30 ms; mark 2–25 ms. Asking adds ~1.5 MB on a short page and 6–20 MB on very long ones while that page stays open; leaving it returns the process to its earlier footprint.

### Windows — on demand, no document-start piece

Through the closed CDP protocol (`platform/agent_semantic_cdp_protocol.rs`): on first ask, create a fresh, unpredictable isolated world for the current document (`grantUniveralAccess: false`), join its execution-context event, install `cdp_source()` with `Runtime.callFunctionOn` (`userGesture: false`), and invoke reads through the same fixed functions. Coexist with extensions and the person's DevTools: one CDP request at a time, detach cleanly, and treat a lost context as `PageChanged`.

As built: the recipes are installed beside the runtime by one more fixed `callFunctionOn` and called through a fixed one-line function; the pinned Windows runtime only exposes its CDP call helper. Runtime reporting is on only between `Runtime.enable` and the world's context event.

### Reader output

`Readout { epoch, title, passages }` (`page_assist/port.rs`). Build it from the runtime's `SemanticSnapshot` via `read_semantic_observation` with `SemanticReadSensitivityLimit::PublicOnly` and `SemanticReadBudget::STANDARD`: one `Passage` per read fragment, `id` = the fragment's `rN`, `verbatim` only when `verbatim_text()` is `Some`, `value` never for protected fields (the service also withholds protected kinds and secret-shaped values). Selection and highlight need two more fixed recipes in the runtime (select-text read; scroll-to-and-mark a verbatim quote without the find bar), each a closed invocation like `H1:survey`.

## Light steps (`PagePort::resolve` / `act`)

- **Judging.** `page_assist::steps::plan` resolves a step against the exact read it was planned on: every readout carries a read id, each step carries it through `resolve` and `act`, and the engine finds that read only by its id (`page_reader::planned_read`, same epoch). Later reads, such as a status check, never stand in for it, and once a step takes over from the person's input the rest of the plan moves to the fresh read. Each tab keeps its last eight reads; a step whose read is gone ends as `PageChanged` and judges it with `site_work::classify` on that read's snapshot: Read, Draft or Commit with its confirm category. Password, card and one-time-code fields (`Password` role or secret sensitivity) and links to another registrable site are refused; a control the verb cannot use is unsuitable. The service holds every Commit for the person's confirm.
- **Running.** The engine re-reads the page, binds the planned proposal to that same read and prepares it against the fresh snapshot (the core's target revalidation, baseline and checkpoint guards), then hands the closed request to the runtime through `PersonDirectedAction` (`zephium-agentic`). That authority is built in one place, marked as the person's, and joins no agent run; it is not Work's policy permit. At the handoff the step is re-admitted only while the same document is committed; queued steps fail as `PageChanged` after a navigation.
- **Input path, both platforms: the runtime's fixed isolated-world recipe**, never native input. macOS uses the existing closed action path (`semantic_action::dispatch_closed`); Windows one fixed `Runtime.callFunctionOn` (`invoke_action_recipe_command`, `userGesture: false`), not the Work path's `Input.*` events. Effects: the page gains **no user activation**; a page handler that calls `window.open` from the click is **blocked** like any script popup; **focus** moves only inside the page (a field is focused by the recipe for typing; the window and the panel keep OS focus). Pages that insist on trusted events (`isTrusted`) or on a user gesture will not respond; the step then reports failed. `userGesture: true` and public `evaluateJavaScript` stay forbidden.
- **Lease.** A step holds the tab for 15 s after it last ran. Meanwhile the page is asked every 200 ms whether the person pressed, typed, scrolled or touched it (capture listeners in the private world, trusted events only); the shell then pauses the steps (`PageAssist::human_input`) and the service re-reads on Continue. A page's own capture listener registered earlier could hide input from these listeners; it can only make steps continue that the person asked for, and Stop always works. A commit ends the lease.

## Lifecycle as built

- A program that will not start ends that ask as unavailable at once; the next ask on the document tries once more, then the page reads as unavailable (never as loading) with no further attempts.
- The first answer to a document carries a fresh token it names when it pulls again. A back-forward cache restore binds the restored document to its new epoch only at its own address, and a document being left cannot displace the incoming one's pull. The script is added by the next admitted navigation, so a blank warm spare never gets it; adopted spares, restored discarded tabs and cache restores read under their own epoch (probe).
- Marks: the highlight and its sheet are removed when the fade ends (a timer backs the frames) and when a new mark replaces the old one. WebKit mirrors `CSS.highlights` into each world's wrapper when that wrapper is created and does not propagate later deletions, so a page that looked at `CSS.highlights` while a mark showed may keep seeing a stale name; the native registry and painting are clear (probe checks pixels), and the stale entry holds no ranges.

## Tests still needed

- The same step and lifecycle checks on a physical Windows machine.
- Windows: the same reads through CDP with an extension's content script and DevTools attached.
