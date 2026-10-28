# Tasks redesign

Status: foundation, scheduling and core design slice implemented; organization
refinements, capture-everywhere and release qualification remain.
Date: 2026-09-22.

## Product direction

Tasks is a shared product surface for people and agents. The primary experience
serves the person capturing, organizing, and completing work. Agents can later
participate in the same tasks through the authoritative resource boundary.
Agent participation is part of the design now, while Work execution and
agentic browsing continue in their separate implementation stream.

This plan follows the user's current scope: improve Tasks as a complete product,
including its foundation and both browser and full-page experiences. Build
natural resource-level capabilities here. Integrate runtime capabilities when
their actual contracts arrive from the other development stream.

References:

- [Product system](../product-system.md): Browse stands alone; agents are actors.
- [Frontend architecture](../frontend.md): ownership and native surface rules.
- [Work persistence](../agent-work-persistence.md): runtime ownership and durable state.

## Scope and ownership

| Build in the Tasks stream | Integrate with the runtime stream later |
| --- | --- |
| Capture, editing, scheduling, completion, restore, undo | Starting, pausing, cancelling, or resuming agent execution |
| Task status and existing responsibility/origin presentation | Agent identities, availability, capabilities and authorization |
| Browser context and resource relationships | Work execution, approvals, evidence and result contracts |
| Reliable projections when another authorized actor edits a task | Agent tool admission and authenticated mutation attribution |
| Navigation, search, organization, accessibility and performance | Binding a task to a real Work or execution |

A task remains useful with AI disabled. A task can be displayed in multiple
surfaces without being copied. Internal agent plan steps do not automatically
become user tasks. Task completion and execution completion are separate facts;
their eventual relationship must be specified by the runtime integration.

## Core concepts

Keep these questions separate:

- **Task:** what outcome needs to happen?
- **Status:** is it open, in progress, blocked, or complete?
- **Responsibility:** who is expected to take the next action?
- **Origin:** where did it come from, and was it created by a person or agent?
- **Context:** which page or resource helps accomplish it?
- **Execution:** is there an actual connected runtime attempt, and what happened?

The existing Rust task fields support the first five at a basic level. Preserve
them during redesign. `assignee` is descriptive responsibility, not permission;
`origin` is existing metadata, not authenticated change history. Do not invent
named-agent identities using the current `user`/`agent` enum. The reserved `work`
field is not proof of a valid or executable Work relationship.

Use human labels such as “In progress” while retaining the canonical Rust values.
An agent-created task may still be the user's responsibility. An agent-assigned
task must not show a running indicator merely because its status is `active`.
Existing agent metadata should be understandable in detail; avoid repeated badges
on every ordinary user-owned task. Do not add an actionable delegation button
until its effect can be carried out and confirmed.

## Information architecture

Proposed navigation:

- **Inbox:** newly captured tasks awaiting organization. This is an explicit
  organization concept, not a synonym for every undated task.
- **Today:** active work due today, with overdue work in a separate section.
- **Upcoming:** future dated work, grouped by day. Undated work does not belong here.
- **All tasks:** all active tasks, with deliberate sort and filter controls.
- **Lists:** lightweight organization if included in the agreed feature scope.
- **Completed:** accessible history with a completion timestamp and pagination.
- **Trash:** recoverable deletion, separate from completion.

Inbox and lists need real Rust-owned semantics and migration rules before shipping.
Do not manufacture membership from a frontend filter. Existing undated tasks must
survive migration without silently being classified as new or unprocessed.

For the initial repair, retain the current due-date meaning and label it clearly.
A planned work date and a hard deadline are distinct product concepts; introducing
both is a separate decision with migration consequences. A due time does not
promise a reminder. Recurrence and notifications need their own reliable native
behavior, including missed occurrences and restart handling.

Search must have one stated scope and behave consistently in list and board views.
Navigation totals are independent of search results and loaded pages. Show “1
overdue” when there is one overdue task and no tasks due today; do not imply two.

## Browser panel

Purpose: capture and act without losing the webpage.

- Keep a compact Tasks heading, a view selector, a discoverable expand action,
  and secondary actions in overflow.
- Distinguish **New task** from **Add current page**. Both reach the same composer;
  page capture additionally supplies an editable title and visible context.
- Give the action title first priority. Allow a bounded second line for long
  titles; place source metadata below when a single line would obscure the title.
- Keep completion and scheduling accessible directly on rows. Hover disclosure
  must have equivalent keyboard access.
- Open a task's full detail within the panel. Back restores the view, query,
  selection, scroll position, and keyboard focus.
- Collapse completed tasks by default. Offer accessible completed history rather
  than permanently displaying a clipped pile of finished rows.
- Replace the all-time completion fraction with useful contextual feedback or a
  temporary undo action. Counts must describe the current view accurately.

Example: capturing a GitHub page permits “Review the storage changes” as the
task title while retaining the original page title and URL as context. A captured
page title alone is not always a useful description of the intended action.

## Full page

Purpose: organize, review, and work through tasks.

- Compose navigation, task list, and optional detail inspector as a connected
  workspace. Avoid a centered fragment of UI inside a much larger empty canvas.
- Keep the reading column comfortable; opening the inspector uses the available
  width without squeezing task titles below a useful width.
- Put the view heading and relevant actions in a compact toolbar. Search should
  be accessible without visually dominating the page.
- Selecting a task opens an attached inspector. Closing it restores list focus.
  On narrow layouts, detail replaces the list and offers an explicit return.
- Preserve task identity, selection, and drafts when changing presentation. A
  board card opens that task, rather than only switching to the list.
- List is the primary presentation. Keep board as a secondary status-oriented
  view if it meets the same editing, error, pagination, and accessibility contract.

Task detail uses a multiline title, a quiet property area, an expanding description,
and readable linked context. Origin and responsibility appear where meaningful.
Empty notes should be an invitation to add detail, not a large raised rectangle.
Deletion belongs among secondary actions and remains recoverable.

## Interaction contract

| Action | Required behavior |
| --- | --- |
| Capture | Immediate local feedback; retained draft until confirmed; clear recovery on failure |
| Edit | Preserve text while saving; edits during a write survive its response |
| Complete | Brief stable feedback, predictable next focus, available undo; pending is distinct from saved |
| Schedule | Explicit date, clear removal, preview of interpreted text, no accidental date changes |
| Delete / restore | Confirm the native outcome, keep recovery available, preserve nearby focus |
| Undo | Reverse one successful action without adding its inverse back to the undo stack |
| Text undo | Text inputs retain normal editing history; task undo applies outside editing contexts |
| External update | Refresh the authoritative task without losing local input or interrupting typing |
| Conflicting edit | Preserve user text and explain a same-field conflict; do not silently overwrite it |
| Change host / close | Drain or retain edits with an explicit outcome; no silent loss on teardown |

Bulk changes need either an atomic native operation or explicit per-item outcomes
and matching undo semantics. A success announcement must describe confirmed
results, not merely the number of attempted commands.

## Foundation findings to address

These findings come from the source inspected on 2026-09-22. Reproduce the relevant
paths with focused tests before repairing them; the dirty working tree can evolve.

1. `TaskComposer.svelte` clears input before creation succeeds and does not
   restore it when `create()` returns `null`.
2. `TaskSession` fetches 100 generic resources before frontend scope filtering.
   Older overdue tasks can be missing from Today; navigation counts reflect the
   loaded search results. The 1,000-row loaded cap also needs explicit behavior.
3. Cached task descriptions are not invalidated against resource revisions when
   change events refresh the listing.
4. `schedule()` and `move()` register history when invoked by their undo closures.
5. `Tasks.svelte` consumes Cmd/Ctrl+Z without excluding text editing targets.
6. `TasksPage.svelte` discards the selected board card's ID when opening list view.
7. The populated list lacks a visible persistent write-error treatment; successful
   reloads can clear the session error without recovering the user's failed edit.
8. Per-task serialization is local to a session. The read-then-replace sequence
   remains subject to concurrent revision conflicts; “rebasing” is not a guarantee
   against an intervening write by another actor.
9. A write settlement clears the entire optimistic overlay for a task. Verify that
   earlier write responses cannot erase the display of newer queued edits.
10. Unknown outcomes need retained request identity and reconciliation. A reload
    alone does not establish whether a particular creation or edit succeeded.

Keep Rust authoritative for identity, validation, persistence, revisions, ordering,
query membership and operation settlement. Introduce task-focused query and count
contracts where the generic resource query cannot represent the required view.
Page in the same stable order that the user sees. Bound rendering independently
from query correctness; loading every task is not the pagination fix.

Use revision-aware cached bodies, field-aware pending edits, and recoverable
operation records. Reconcile independent field changes where possible; make the
same-field policy explicit. Regenerate IPC bindings for changed Rust contracts.
Do not duplicate domain records or execution authority in Svelte.

## Visual direction

Use Quasar's shared type and semantic tokens. Establish hierarchy through title
legibility, alignment, spacing, and restrained surface differences. Keep native
material in chrome; content remains stable and readable over varied backgrounds.
Avoid nested filled cards, a capsule around every property, decorative glass,
and perpetual status animation.

Task rows have clear hover, selected, focused, pending, and failed states. Keyboard
focus must remain visible. Color supplements labels rather than carrying meaning
alone. Support light and dark appearance, increased contrast, reduced motion,
forced colors, long titles, localized text and narrow hosts.

If multiline rows require new virtualization behavior, use measured or otherwise
correct bounded heights. Do not retain fixed one-line assumptions while changing
the rendered geometry.

## Delivery sequence and acceptance

1. **Foundation repairs:** reproduce failure, concurrency, pagination and undo
   issues; repair contracts and settlement before depending on them in new UI.
2. **Core design slice:** capture, list, detail, scheduling, completion and undo in
   both native hosts. Review empty, populated, long-content and failure states.
3. **Organization:** finalize Inbox/list semantics and feature scope; implement
   the corresponding Rust migration, query and editing behavior.
4. **Secondary presentation and integration handoff:** qualify board parity and
   document resource operations for the future agent adapter. Bind runtime UI only
   to capabilities verified in the integrated checkout.
5. **Release qualification:** run repository checks and inspect native behavior.

Acceptance must cover:

- Zero tasks; an empty Today with tasks elsewhere; one task; many pages of tasks;
  overdue tasks outside the newest resource page; substantial completed history.
- Create failure, unknown outcome, safe reconciliation without duplicates, edits
  during saves, same-field and independent-field concurrent edits, close/reopen.
- Accurate search/counts, page-to-panel continuity, board-to-detail identity,
  completion/delete focus, bulk outcomes, repeated undo and normal text undo.
- Long titles, long descriptions, page context, different viewport sizes, keyboard
  operation, screen-reader labels and announcements, all supported appearance modes.
- Rust migration and profile boundaries; required frontend and native checks;
  realistic rendering and resource-use checks at the supported collection limit.

The earlier focused unit baseline was 33 passing tests across five files. It does
not cover these acceptance cases or establish native quality. Record actual new
evidence with each implementation stage; do not mark this plan complete from
static checks or screenshots alone.

## Decisions and contracts (2026-09-22)

- **Status** keeps all four values. Completion stays one click on the check;
  In progress and Blocked are a secondary property, shown as a state glyph and a
  quiet label on the row, never as a running indicator.
- **Scheduling** separates the planned day from the deadline, TickTick-style:
  `due_date`/`due_time` remain the planned day and optional time; the new
  `details.deadline` is a date-only hard deadline and `details.duration` an
  estimate in minutes (1–10,080). Views, counts and sections place a task by
  whichever of its planned day and deadline comes first, in Rust and in Svelte.
  Profile migration 19 adds the `task_deadline`/`task_duration` projections.
- **Reminders and recurrence** are deferred; nothing implies a notification.
- **Board** is offered for All tasks and custom lists only. Date views hold open
  tasks, so a Done column there could never fill.

Resource contracts added for every actor, including the future agent adapter:

| Contract | Behavior |
| --- | --- |
| `update_task { id, set: TaskField[], expect: TaskField[] }` | Writes individual properties onto the current revision. `expect` states same-field preconditions; a mismatch is `conflict`. Independent fields never conflict. Duplicate fields are invalid. |
| `task_overview { today }` | Counts and lists without rows, for refreshing totals after a settled write. |
| `resource-changed.kind` | `note`, `task` or `task_list`, so a task surface ignores note edits and fetches only the changed task. |

`TaskField` covers title, description, status, schedule, deadline, duration,
organization, priority, steps, pinned and position. Assignee and origin are not
writable through it until the runtime contract defines who may change them.

Foundation findings 1, 2, 4, 5, 6 and 7 are resolved; 8 and 9 are addressed by
field-level writes with explicit expectations. A failed save no longer blocks
capture, and the session discards or retries retained edits explicitly.

Bundle budgets for the task graphs were raised with this work (measured, plus
about 2.5% headroom): the list body carries the token parser, deadline and
estimate rendering, the undo notice and the failure banner; the page now
includes its own navigation, whose separate lazy chunk and budget are gone; the
detail and board draw the new properties and priority glyphs.
The launcher's capture is its own lazy chunk (`features/tasks/lib/capture.ts`,
budgeted at its measured size plus headroom): it loads only when a line is kept
as a task, and is almost entirely the shared resource domain.
The board, detail and page CSS limits rose by about 1.5 KB each for the list's
drag marker and carried-row styles and the date picker's time slots, which
share their chunks.

## Remaining product choices

The user has established the shared human/agent direction and the separate runtime
ownership. The following remain proposals, not approved feature commitments:

- Whether lists gain colours or icons.
- When recurrence and native reminders enter scope, and their missed-occurrence rules.

Foundation repair and the core interaction design can proceed independently of
these feature-expansion decisions.
