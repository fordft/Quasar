# Zephium product system

Status: inherited upstream product and target-architecture reference

Quasar uses [Sign in with ChatGPT](chatgpt.md) as its sole product AI
connection. The local-model, BYOK and Zephium-hosted account paths described
below are upstream design history and are retired in this fork.
Last reviewed: 2026-09-13

This document defines what Zephium is, what the first public release must be,
and the architectural decisions that let the product grow without being
rewritten. Product direction is fixed at this level; individual interaction
details and replaceable implementation choices can evolve when evidence says
they should.

This is not a claim that every described capability exists in the current
tree. `architecture.md` remains the source of truth for the implemented browser
foundation, `security-model.md` for enforced security guarantees, and
the `agent-work-*.md` documents for Work runtime subsystems. `agentic-browsing.md` describes the browser execution substrate. This
revision supersedes older product assumptions; changing this
document does not change implemented permissions or contracts.

## 0. Program context

Zephium is not beginning as an AI mockup around a generic browser shell. Its
WebView-based browser foundation has already validated the hardest architectural
risk: native page lifecycle, low application overhead, native Rust blocking,
and substantial Chrome-extension compatibility. Active extension work has
validated difficult extensions including Vimium, Dark Reader, and 1Password on
macOS and is bringing Windows to parity. This is engineering-program context,
not a shipping claim; the current tree and `security-model.md` determine what
is enabled and guaranteed at any exact revision.

Extension work may continue concurrently with Work foundations. Changes to
profile construction, native extension controllers, page principals, view
lifecycle, or platform forks must therefore be coordinated rather than
silently rebased over either subsystem's invariants.

The project also starts with meaningful distribution: the creator's Terax
open-source audience and launch experience have produced nearly 10,000 GitHub
stars and substantial product interest. Zephium should be engineered for
thousands of real users from its first release, not as a disposable prototype
that will be rebuilt if a launch video succeeds.

## 1. Product thesis

Zephium has two native ways of using the web:

**Browse** is a premium, lightweight, private browser. It must be worth using
when every AI capability is disabled.

**Work** is an environment for accomplishing complex work through the web and
the rest of the computer with humans, agents, tools, knowledge, and persistent
state.

The central insight is:

> Complex work should not disappear behind a chat interface. It should have
> visible, persistent, directly manipulable state.

Language is often the fastest way to express intent. The environment is a
better way to represent complex state. The agent is an actor in that
environment, not the interface itself.

Zephium is therefore not positioned as a canvas browser, graph browser,
productivity browser, research browser, AI browser, agentic browser, task
manager, or browser with built-in tools. Those are components or properties.
The product identity is **Browse + Work**.

## 2. Product principles

The following are release requirements, not aspirations:

1. **No novelty tax for browsing.** Opening GitHub, watching a video, reading,
   searching, and managing tabs remain ordinary, excellent browser actions.
2. **Browse stands alone.** AI and Work can be disabled completely without
   degrading the browser. AI can also be disabled while manual Work remains
   available.
3. **Work state is visible.** Objectives, plans, evidence, execution,
   responsibility, questions, approvals, artifacts, and unfinished work are
   inspectable while they evolve.
4. **State survives the transcript.** Useful resources, tasks, decisions,
   knowledge, and artifacts persist after a conversation ends.
5. **Human control is structural.** A user can inspect, redirect, pause, take
   over, approve, reject, or manually complete work. Control is not a panic
   button added after autonomous execution. Pause and continuation are offered
   only where the runtime supports them; cancellation is not advertised as pause.
6. **Native components before generated webpages.** Agents populate bounded,
   accessible Zephium components with structured data. They do not inject
   arbitrary HTML into a privileged surface.
7. **Local-first and privacy-conscious.** Local state remains local by default.
   Only the minimum relevant context crosses a model or service boundary.
8. **One product architecture.** Local models, BYOK, and Zephium-hosted models
   use the same work, agent, tool, and result contracts. Provider choice must
   not create different products.
9. **No silent degradation.** If a model, platform, or integration cannot meet
   a capability contract, Zephium explains the incompatibility and asks for a
   compatible choice. It does not quietly produce an inferior workflow.
10. **Best-in-class engineering is part of the product.** Correctness,
    responsiveness, CPU, RAM, GPU, battery, security, recoverability, and
    debuggability are acceptance criteria for every subsystem.

### 2.1 One hierarchy across Browse and Work

The product hierarchy is **Profile → Space → Work → Area**. Tabs and reusable
resources also live within the profile's Space organization; they are not
forced to belong to a Work.

| Concept | Meaning and boundary |
| --- | --- |
| Profile | Browser identity and isolation root for sessions, credentials, and private data. |
| Space | Shared organizational home for tabs, Works, and related resources. |
| Work | Persistent working environment, normally presented as a canvas, containing resources and zero or more objectives and executions. |
| Area | Optional named spatial group inside a Work, with no automatic execution or permission boundary. |

Do not add a parallel Projects hierarchy. A Space can organize a product,
travel, learning, or personal work without requiring users to call it a
project. Start with one default profile and Space; hierarchy creation is not
onboarding homework. Additional Spaces are available when useful.

Changing modes preserves the current profile and Space. Changing Spaces changes
the organizational context. A Space is not a separate cookie jar or a promise
of account isolation. Works have a home Space and retain profile ownership.
Same-profile references can reuse a note, artifact, or browser resource across
Works without duplicating its identity. Cross-Space discovery follows context
access settings; cross-profile access or copying must always be explicit.

Browser profiles, a Zephium service account, connected website accounts, and
model-provider credentials are distinct concepts even when reached through one
menu. Moving or grouping an object never grants a capability or changes which
account an operation uses.

## 3. Browse

Browse owns the familiar web experience:

- windows, profiles, spaces/workspaces, tabs, splits, and navigation;
- horizontal, vertical, compact, and power-user browsing arrangements;
- platform-native presentation and integration;
- native Rust ad and tracker blocking;
- practical compatibility with major Chrome extensions;
- privacy-first defaults, zero application telemetry, and open-source code;
- customization through validated product concepts rather than injected code;
- optional notes, tasks, history, analytics, focus tools, command/search, and
  contextual AI.

Platform WebViews provide rendering. Zephium owns browser state, lifecycle,
resource policy, interface, product features, and the integration model around
them. Being WebView-based is an engineering constraint, not the product
identity.

### 3.1 Browser interface

The normal browsing experience is centered on a polished sidebar. It has a
full state for tabs, spaces, profiles, and browser controls, and a compact state
that leaves room for one active native utility such as Notes, Tasks, History,
Analytics, or contextual AI. Utilities should appear when useful and collapse
cleanly when not.

A global Raycast/Spotlight-style command surface can expose browser search,
commands, notes, tasks, actions, and later Work. It remains one native product
surface rather than a parallel application.

### 3.2 AI in Browse

Browse may offer page-scoped help such as summarization, translation,
extraction, questions about the current page, and small actions. It is compact
and contextual, not Zephium's defining AI interface. A conventional chat may
exist where it is the simplest representation, but chat is never allowed to
become the primary product shell.

AI and Work have independent enablement. Disabling AI removes the AI composer
and model-dependent actions while preserving manual Work, its objects, and its
toolbar. Disabling Work leaves ordinary Browse available. Notes, tasks, history,
analytics, and other non-AI utilities remain independently usable and
independently disableable. Neither local/BYOK use nor manual Work requires a
Zephium service account.

## 4. Work

Work is a distinct full-window environment for substantial goals: development,
research, product work, operations, management, recruiting, design, content,
learning, purchasing, decisions, administration, and other multi-step work.
The web is its natural foundation, not its boundary. Work may also use files,
commands, APIs, MCP servers, connected services, and specialist agents such as
Codex or Claude Code when the user authorizes them.

Agents are expected to do much of the active work, but humans and agents use
the same environment and durable objects. Fully manual, fully delegated, and
mixed work are all valid. Users do not need to construct automation nodes,
choose tools in a prompt, or know an execution route before starting.

Work is not a larger AI sidebar, a dashboard, a roadmap, or a transcript split
into cards. The canvas can hold an organized investigation, an evolving plan,
or simply unrelated notes and media. It has no mandatory start/end graph.

### 4.1 Core loop

Substantial work follows this conceptual loop:

1. **Understand.** Start with the user's instruction, selection, and relevant
   Work state. Retrieve other permitted context when useful.
2. **Clarify when necessary.** Ask for missing choices that materially affect
   the work. Questions with options appear above the still-available composer;
   do not ask for facts already established by relevant context.
3. **Establish an approach.** Present concise responsibilities, decisions,
   dependencies, and expected outputs through tasks, areas, and resources.
   Detailed operational planning remains inspectable but is not required reading.
4. **Approve.** The user approves an explicit scoped plan rather than a vague
   autonomous request.
5. **Execute visibly.** Agents and tools operate while their purpose, inputs,
   progress, questions, and outputs remain inspectable.
6. **Persist the result.** Resources, artifacts, decisions, tasks, and useful
   knowledge survive. A transcript is supporting evidence, not the product
   result.

Understanding and execution are separable. A user can ask Zephium to build a
complete model and recommendation, then execute all, some, or none of it.
This loop is not a wizard that every interaction must traverse. Manual work
requires no objective or approval ceremony, and a small contextual question
does not need an elaborate plan. Model calls and context retrieval still obey
their applicable authorization and disclosure rules.

Routine directed work does not require a visible planning stage. For example,
after clarifying keyboard size and budget, public product discovery should
produce a few useful image-backed product comparisons on the canvas. The user
chooses a result there; payment is a separate explicit decision at the effect
boundary. Complex work, such as coordinating travel dates, flights and lodging,
benefits from a visual approach showing responsibilities and intended resources
before execution. Neither case should open a separate plan window or populate
the canvas with paragraphs merely because a model produced them. Detailed text
is secondary and shown where it helps a decision. Compact activity states such
as searching, comparing, or waiting for the user belong near the relevant work
or the task/composer controls, not in a tool transcript.

### 4.2 Entry, continuity, and controls

Entering Work makes the canvas the primary surface immediately. Restore the
current Space's last Work and its presentation checkpoint, or offer a usable
new Work when none exists. Do not force a dashboard or a create-objective form
before the user can work. Recent/active Works, search, creation, reopening,
archive, and deletion are available through the Work switcher.

The regular Browse sidebar is fully hidden. The initial arrangement is:

| Location | Control |
| --- | --- |
| Top left | Compact Space/Work name and switcher. Keep a discoverable Return to Browse action in this control; a separate minimal icon is an interaction detail to qualify, not a second large header control. |
| Top center | Compact toolbar of vertical icon-over-label tools, with Tabs visually distinguished at its center: Notes, New (Table, Chart, Comparison, Document, Checklist), Tabs, Media & Files, Area. Each opens a focused popover that combines finding existing objects with creation and attachment. Connections joins the toolbar only when a connection capability exists. |
| Top right | One profile/avatar control opening the Work-specific account, usage, provider, and settings menu. No separate settings or usage cluster. |
| Right edge | Compact task/activity control that expands into a useful task panel. |
| Bottom center | AI input panel anchored to the window's lower edge, with selected-context chips, workflow access, and questions/approvals above it. |

The header band (window controls, identity, toolbar, profile) sits directly on the
native window material with no background of its own; the same applies to a
horizontal tab layout in Browse. The canvas is the rounded main content region
below it, like Browse's content area. The composer is docked flush to the window's
bottom edge with square bottom corners and no bottom margin, and reserves a voice
input control. Inside Work every transition is short, purposeful, and
compositor-friendly; the Browse sidebar and the Browse→Work transition are a
separate polish step.

Exact spacing and the minimal return affordance remain subject to real native
visual and accessibility testing. Do not fill all corners with permanent
panels. Settings remain accessible without leaving Work. The Work profile
popover is Zephium-owned UI; it does not replace Browse's native menu boundary.
Show measured usage and labeled estimates, not invented provider balances.

Toolbar popovers combine creation with finding and attaching existing objects.
Use human vocabulary such as Notes, Table, Chart, Comparison, Document, and
Area, rather than exposing "nodes" or "UI components" as required concepts.
Click and keyboard operation must work without hover. Capability availability
is visible; an unavailable connection or generator is not a working control.

Preserve profile, Space, real tab identities, window geometry, and the return
location in Browse. Consistent typography, materials, focus behavior, and
short purposeful transitions make Work feel like another environment in the
same browser. Respect reduced motion and never delay interaction for animation.

### 4.3 Work identity

A `Work` is a profile-owned, Space-organized persistent aggregate. Its canvas
is a presentation of that aggregate, not its storage model. It can exist with
no objective, execution, agent, or connected objects. A user can add loose notes
and images today and start an objective using them tomorrow. A Work can contain
several objectives and executions over its lifetime; completing one does not
close or complete the workspace itself.

Work contains or references tasks, resources, actors, artifacts, decisions,
questions, approvals, knowledge, and views. Distinguish these from runtime
attempts and model conversations. Existing objective-bound Work contracts must
be evolved explicitly with preserved identities, historical joins, and forward
migrations, not hidden behind a second frontend-only aggregate.

Tabs and browser contexts can be attached to Work, but tab identity and Work
identity remain independent. Cross-profile copying is always explicit; a Work
never silently crosses a profile privacy boundary.

### 4.4 Spatial environment

A spatial surface is the strongest current representation for Work because it
can show structure, parallelism, dependencies, and heterogeneous artifacts at
once. It supports pan, zoom, selection, move, resize, grouping, meaningful
connections, focus, and collapse.

It is semantic, not a general drawing application. Freehand whiteboarding and
arbitrary decorative layout are not initial goals. Automatic layout is valid
only when it carries meaning and preserves user placement; the product must not
shuffle state merely to look dynamic.

The organizing unit is the **subject**: a framework, a listing, a flight, a
product, a concept. A subject is a hub; its sources, attached tabs, images,
findings, measurements, and comparisons connect to it, and Areas group hubs. The
canvas is never organized as objective → result, and it is not a dashboard of
unrelated charts and tables. Subjects and findings have stable identity so they
can be selected, revised, and related independently; later work adds to existing
hubs rather than creating a second pile. Objective and execution details live in
activity and the inspector, not as mandatory cards.

The primary agent is visible on the canvas as an avatar with a compact status
(thinking, searching, comparing, waiting). Expanding it shows admitted steps:
which source, subject, or responsibility, never raw reasoning or tool noise.
Sub-agents appear as smaller avatars attached to the subject or responsibility
they work on while active. Searching is visible work: sources appear as connected
source cards as each bounded unit settles, not behind a spinner and not as a
token stream. Text is shown where the material is inherently textual (a learning
note, a requested document); otherwise results are objects and connections.

Opening an object lifts the real card: it animates to a readable size near the
center as a transient transform, is edited in place, and returns to its exact
place on Escape or a click outside. Stored geometry never changes on focus.

Loose objects need no connectors. Areas can group flights, housing, and an
itinerary, or sources, experiments, and conclusions. Users and agents can create
and arrange areas; an area does not itself authorize tools or imply a DAG.
Relationships such as "uses source," "depends on," and "supports conclusion"
have distinct meanings. Show detail on focus when displaying every edge would
make the workspace unreadable. Search, fit/zoom-to-selection, area navigation,
and an accessible list presentation keep a large Work navigable.

Contextual actions act on real objects. Selecting a chart and asking "Explain
this" refers to its data and evidence. "Use Booking instead" updates the
relevant constraints and assignments, identifies affected results, and requests
new authority only if needed. It does not merely replace a logo. User placement
and edits are preserved; automatic organization applies deliberately, not on
every incoming result.

Information chooses the best native representation: cards, tables, maps,
timelines, documents, diagrams, source structures, code, tasks, images,
comparisons, and browser resources can coexist. A graph is one representation
inside the environment, not the product model.

### 4.5 Browser resources in Work

Tabs are one kind of resource, not the organizing unit of Work. Keeping many
interactive WebViews embedded across a zoomable canvas would create poor
interaction, focus, accessibility, compositing, and resource behavior.

The centered Tabs toolbar popover shows the current Space's open tabs, search,
selection, Add to Work, Open here, and a new-tab action. It opens automatically
on the first entry into Work so browser continuity is immediately visible.
Afterward, preserve the user's interaction preference rather than reopening it
on every transition. There is no permanent browser tray or automatic dump of
all tabs onto the canvas.

A new empty Work may also suggest the current tab with an explicit Add action;
suggestion is neither attachment nor permission to read. Existing Works restore
their real resources. Browse may offer Create Work with selected tabs. An
attachment references the same tab/context and never clones its URL into a new
session. Merely listing tabs does not expose all page contents to the model.

A browser-resource card shows title, icon, origin, purpose/state, and optionally
an existing bounded safe frame. Unloaded/restored tabs show title and icon;
never instantiate or wake a page for a preview. Initial previews have no
periodic capture or refresh loop. Cached frames are visibly descriptive, not
proof of current remote state. Opening a card or a source activates the exact
retained page in a transient native pane inside Work: it opens near the
originating card, can be moved and resized, dismisses on Escape or a click
outside, keeps its geometry only for the session, and never becomes canvas
state. Explicit actions open the page in Browse or add the tab to the Work.
Selecting a card need not activate its page.

Native geometry, observation, foreground, input, retirement, and ownership
fences remain authoritative. Closed, changed, unavailable, and agent-owned
resources have honest states. A URL in an artifact is a link, not a browser
context or a navigation grant.

Work-owned contexts, borrowed tabs, human takeover, and exact lifecycle rules
are specified in `agentic-browsing.md`.

### 4.6 Native visual primitives

Zephium owns a versioned registry of structured visual primitives. The target
vocabulary is expected to cover:

- Card and resource collection;
- Browser resource and source;
- Image and media;
- Map;
- Table and comparison;
- Chart;
- Timeline;
- Task and checklist;
- Note and document;
- Code and diff;
- Agent and tool activity;
- Artifact;
- Diagram and graph;
- Status, question, and approval.

The existing seven semantic artifact kinds are Document, Table, Comparison,
Chart, Checklist, Evidence collection, and Browser-resource preview. Reuse their
contracts and native renderers. Extend them when a real workflow needs an
additional representation; do not claim that the full target vocabulary is
already implemented. The next extensions are subjects, typed comparison cells,
findings, structured source entries, typed document blocks, and measurement
bases, each with evidence attached at the claim or cell level.

Numbers are honest. A measurement or rating rests on cited evidence with a
comparability basis (method, conditions, versions, observation time) or on
well-known general knowledge explicitly labeled as such; the interface shows the
two differently. Anything important, uncertain, or web-derived without citations
stays Unknown, and no chart is produced without a basis. Images, audio, local/video links including YouTube, PDFs,
diagrams, and code/diffs are useful product resources with explicit capability
and delivery status. Local files distinguish a scoped live reference from an
imported snapshot. Generated image/audio results become owned media resources
with provenance and accounting, not arbitrary executable UI.

Each primitive has a typed, bounded, versioned specification, stable identity,
accessibility behavior, interaction contract, renderer, persistence codec, and
migration path. Models can request a supported component and populate its
data. Rust validates admission. Unknown versions fail safely or render a
bounded fallback. The product's real-time generative UI is this validated
semantic composition: agents provide data and relationships to Zephium-owned
components. They never generate or execute arbitrary HTML, CSS, Svelte,
JavaScript, selectors, or privileged UI. Images and source logos use admitted
media loading with bounded decoding and privacy rules; a model-supplied image
URL is not trusted markup.

Artifacts are directly useful: sorting/filtering a table, inspecting chart
data, editing a document, accepting a finding, or preserving a result as a Note
or Task uses native operations. Preserve original artifacts, evidence, review
requirements, and subsequent user revisions. Presentation edits do not silently
alter evidence or claim a remote mutation.

### 4.7 Actors and delegation

A Work may contain the user, a primary Zephium agent, bounded sub-agents,
specialist external agents, deterministic workflows, local tools, connected
services, and local models. Responsibility must be visible at the part of the
work it affects.

The primary agent coordinates intent, relevant context, decisions, assignments,
and synthesis. Delegate substantial or noisy branches to specialists with the
tools their responsibility requires. Do not create a child for every tab or
give every worker the complete tool inventory. A browser researcher does not
need shell authority; a coding worker needs an explicitly scoped workspace and
process capability.

Children receive compact typed assignments: relevant objective, approved scope,
decisions, resources, constraints, budgets, and expected outputs. They return
structured outcomes, artifacts, evidence references, uncertainty, and accounting,
not full transcripts. Preserve original publication receipts when parents consume
child results. Rust admits responsibilities, enforces dependencies, schedules
workers, and propagates cancellation.

Use bounded direct-child topology rather than arbitrary recursion. Represent
in-flight children as a bounded keyed collection in scheduling, persistence,
IPC, and projections. One active child and one native browser slot may be the
initial admission policy; neither is a structural schema restriction. The UI
emphasizes responsibility and outcomes, revealing actors when ownership,
handoffs, questions, failures, or waiting matters.

External agents complement Zephium. A coding branch can be handed to Codex or
Claude Code while its objective, context, progress, review, and result remain
part of the parent Work. Integration is through capability-scoped adapters,
not screen scraping when a structured CLI, API, or MCP route exists.

### 4.8 Plans, effects, and approvals

Distinguish the user-visible approach, evolving internal execution assignments,
and exact authorization. An approach explains responsibilities and outputs;
operational details are available on inspection. Approval binds a precise
versioned grant and execution specification, including actor, accounts,
capabilities, disclosures, effects, destinations, budgets, and lifetime. It is
never approval of an animation, model message, or unversioned text summary.

Unknown-route public research uses a bounded read-only discovery capability.
Models can choose search terms and admissible public sources without users
predeclaring every origin/path or approving every link. Native browser discovery
validates navigation, redirects, destination safety, grounding, disclosure, and
resource bounds. It must not inherit authenticated cookies or attached private
content silently. New public sources inside that grant do not require amendment.

Authenticated account/resource access, sensitive disclosure, writes, and
consequential effects retain exact approval. Crossing those grants, increasing
approved budgets, or changing the permitted effect requires renewed approval.
Purchases, publishing, messages, account creation, and destructive operations
must be visible decisions with proven settlement boundaries. Signed-in reading
and approved reversible writes are part of the product; do not blanket-exclude
them because more consequential effects are not yet qualified.

Account-scoped work starts from an attached tab the user names. The approval
shows the profile, origin, exact page, effect class (read only, or one field
transition plus its restoration), and budget; the user attests that the tab is
signed in as the intended account, and the product states that it cannot verify
the account itself. Execution uses a Work-owned page sharing that profile's
session; the tab is never driven. Sign-in walls, challenges, and unsupported
interactions stop the run with a persisted reason and offer the page in the
pane; a takeover revokes and drains automation first. Continuing is a fresh
approval of the same specification, never a resumed run.

Routine public work runs as a Rust-supervised loop under one grant: sending
the objective authorizes anonymous public search, reading, discovery, and
placing objects on the canvas within a shown budget, and nothing else. Each
turn the model proposes operations from a closed vocabulary; Rust admits them,
runs them, and commits every step, source, and object as it settles, so the
canvas fills while the agent works. Questions are steps of the run, answered
above the composer while it waits. Search text that repeats private context is
refused. Accounts and effects never enter this grant; they keep exact approval.

Adaptation inside an approved capability must still create Rust-admitted,
durable assignments under a bounded delegation policy. Never mutate historical
approvals/specifications or allow a child to expand its own authority. Changes
that invalidate the admitted specification require replacement admission and,
where the grant changes, renewed user approval. A frontend graph edit alone
cannot authorize new work.

Two-factor prompts, CAPTCHAs, security challenges, and other actions that
cannot be delegated safely become `NeedsHuman` state. They are not disguised
as ordinary approval prompts. Human takeover revokes and drains automation
before granting input. Persist the reason for intervention. Continuation uses
fresh observation and admission, with approval where required; do not promise
resumable pause or automatic restart continuation without runtime support.

### 4.9 Human editing during execution

Manual actions and agents use the same validated Rust operations. Layout,
selection, and grouping remain available during execution. An object actively
being modified shows its owner. Conflicting edit/remove actions offer to stop
the affected operation first, and wait for the real ownership fence to settle.
Unrelated work remains available. Durable updates check the expected revision;
do not overwrite a user's newer edit or require collaborative text machinery
for the initial product.

Removing a canvas representation, deleting its resource, and closing its native
tab are distinct operations. Local undo uses revision history; reversing a
remote effect requires a verified compensating operation. Stopping a worker
does not prove that a remote effect did not happen. Preserve pending/conflict
drafts and honest unknown outcomes through close and reopen.

### 4.10 Workflows, not professional modes

The composer exposes a workflow picker and slash commands. Categories such as
development, research, marketing, travel, and learning help discovery; they do
not create different application modes. Review PRs, investigate a bug, compare
products, test a web application, and plan a trip are reusable starting points,
not hardcoded execution routes.

Users can inspect, edit, create, and save workflows containing instructions,
questions, expected outputs, suggested organization, and capability requirements.
The agent adapts each run to its actual context. A saved workflow contains no
credentials and carries no approval from an earlier execution. Workflow access
is useful guidance, not a prerequisite to describing an objective naturally.

## 5. Tasks, notes, knowledge, memory, history, and analytics

These are native resources and views shared by Browse and Work. Their useful
identities are independent of execution plans, attempts, and transcripts.

### 5.1 Tasks

Tasks represent unfinished work. A task can survive its Work session, appear
in Browse, reopen exact context, retain related resources, be completed by a
human, and—within an approved capability—be delegated. A checkbox rendered in
an AI answer is not a task unless it has a durable task identity.

Tasks organize actionable work with understandable responsibility, dependencies,
and decisions. Keep runtime plan nodes distinct: materialize or link a durable
Task when it is a useful user action, not for each tool call. Explicit Work
relationships support user-directed preservation without duplicating resources.

In Work, Tasks are the main unit of unfinished work the way tabs are for Browse:
the user's durable Tasks are pinned in the right-edge control, and agents may
create or complete them through the same validated resource commands with
visible attribution. The control is a compact vertical capsule: a count or state
with a thin bar above, a label below, and a restrained warm accent only while
work is active. Hover can preview; click or keyboard activation opens a stable
panel that stays open until dismissed. Do not depend on hover or animation to reach
tasks. Show counts, activity, or Needs your decision for open-ended work. Use a
percentage only with a meaningful basis, and reflect scope changes honestly.
Neither task checkmarks nor estimated progress prove an execution succeeded.

### 5.2 Notes and documents

Notes are lightweight user artifacts and context, kept as Markdown files the
person owns rather than records in a Zephium database. Rich editing uses Tiptap
directly over a Markdown bridge that writes unchanged text back exactly as it
was. Only the active document mounts an editor; read-only and spatial
projections use lightweight renderers. Collaboration packages and heavy extensions are
not loaded until a real product requirement exists. Notes are a well-built
unit in the system, not a second product or a reason to build an editor engine.

### 5.3 Memory and knowledge

Memory has two conceptual layers:

- **System memory** is mostly invisible, structured context such as history,
  relationships, previous decisions, tasks, site context, and interaction
  patterns used for retrieval.
- **User knowledge** is information deliberately worth revisiting: decisions,
  findings, explanations, references, notes, documents, and other artifacts.

Agent output does not automatically become permanent user knowledge. Zephium
can suggest preserving useful material. Retrieval filters locally first and
sends only the relevant bounded context to an external model. The user can
inspect a context manifest showing what will leave the device and why.

The current Work is the agent's first-class context. Supply a compact semantic
inventory, selection, relevant objectives, decisions, task states, relationships,
and resource revisions. Retrieve specific document sections, table ranges, or
page observations on demand. Large inventories are themselves searchable;
neither full canvas screenshots nor all object contents belong in every turn.
The composer and contextual actions show their selected objects consistently.

Other Works, unattached notes, browsing history, and open tabs are secondary
sources accessed under explicit context settings. Local discovery is distinct
from disclosure to a model or service; merely displaying a resource does not
grant disclosure. Retrieval is bounded, attributable, and profile-filtered.
An area or explicit relationship supplies context; accidental spatial proximity
does not create a dependency or permission. Source changes mark affected results
stale or needing review rather than rewriting earlier evidence.

### 5.4 History and analytics

History can grow beyond URL, title, and timestamp into contextual browsing and
Work history. Analytics can explain what was worked on, which resources
contributed, what remains unfinished, and where time went. Processing is local
where practical, opt-in where sensitive, and never a surveillance or
advertising channel.

## 6. AI and agent architecture

Zephium has one agent architecture with replaceable model transports:

```text
Intent / Work state
        |
Rust agent supervisor and policy engine
        |
Typed model, browser, tool, memory, artifact, and approval ports
        |
Local runtime | BYOK provider | Zephium hosted gateway
```

The Rust process owns run state, planning contracts, tool scheduling,
capability enforcement, cancellation, budgets, browser execution, durable
semantic commits, and recovery. A model proposes structured operations but is
not security authority. Page content, model output, extension output, MCP
results, and remote service output remain untrusted input.

### 6.1 Model modes

The target model surface includes:

- OpenAI, Anthropic, and Gemini BYOK adapters;
- OpenAI-compatible custom endpoints;
- local runtimes such as Ollama, LM Studio, and llama.cpp endpoints without
  bundling an inference runtime initially;
- a Zephium-hosted model gateway.

All implement the same typed streaming and tool-use contracts. Capability
descriptors express structured output, tool calling, images, context limits,
and other requirements. Work chooses only eligible models and explains why a
requested model cannot run a workflow. Provider changes never bypass an
approval or capability boundary.

Distinguish recommended/qualified models, compatible models with stated
limitations, and unsupported capabilities. API compatibility alone is not
qualification. Do not promise support for every model a provider lists, or
exclude a capable local model merely because it lacks native search. A local
model can use admitted browser or connected tools if it reliably meets their
contracts. Local inference does not make website interactions offline. Shipping
availability follows measured integration evidence, not this target list.

Secrets live in the operating-system credential vault and are zeroized from
temporary memory where practical. API keys, refresh tokens, and desktop
service credentials never live in ordinary SQLite fields or frontend state.

### 6.2 Hosted gateway

Hosted AI is part of the first product, but hosted browser execution,
multiplayer, mobile continuation, and the larger cloud ecosystem are not.
Initially the gateway authenticates, validates entitlements and budgets,
selects an eligible provider/model, relays bounded requests, streams outputs,
and accounts for provider-reported usage.

The hosted gateway is a separate private Python service using FastAPI. Its
authentication, entitlement, billing, persistence, cache and deployment
adapters remain behind explicit service boundaries; their concrete products
must be selected from operational evidence rather than becoming browser-domain
dependencies. The public browser does not require an account when hosted
features are unused, and it must not initialize hosted connections in that
state.

The gateway stores no prompts, page contents, model outputs, screenshots, or
tool payloads for product analytics. Operational and billing records are the
minimum content-free metadata required for abuse controls, reliability, and
accounting. Downstream model providers retain their own disclosed policies;
Zephium must not claim that BYOK or hosted relay changes those policies.

Hosted AI should offer an integrated experience; speed, quality, and efficiency
advantages must be measured. It must not impose dependencies on BYOK or local
Work merely to make the hosted option appear better.

### 6.3 Search and capability providers

Work searches the person's own search engine's results page in the agent
browser and reads it with no model call (see
[Work execution](agent-work-execution.md#web-search)); the chosen provider's
native web search is its capped fallback. Preserve search activity, citations, provenance,
usage, cancellation, and attribution requirements through typed adapters. Do
not require Tavily, Firecrawl, a Zephium account, or a hosted search subscription
for BYOK. A future hosted offering can add suitable search services.

Ordinary public search needs no query-by-query Rust approval. Provider-native
search executes remotely within an admitted model request; Rust controls the
disclosed input and enabled tools and records returned evidence, not each
provider-internal step. Prepare task-relevant context so private material is
not casually included in public queries. A separate search worker is useful
when context isolation or workload size warrants it, not for every question.
Provider citations remain distinguishable from native browser extraction
receipts; never fabricate one evidence type from the other.

Prefer explicitly connected typed APIs, MCP tools, and trusted CLI capabilities
when appropriate. Use the native browser for web-interface interaction,
authenticated sessions, or discovery when suitable search is unavailable. A
simple public fact lookup need not instantiate a WebView. Browser screenshots
are bounded optional tools for phases requiring visual evidence; semantic
observations remain the default.

One capability-provider boundary admits browser, API, MCP, CLI/process, local
file, media, and future hosted workers without changing Work semantics. Each
adapter declares inputs, outputs, ownership, permissions, effect classes,
budgets, cancellation, and settlement. MCP is a protocol, not trust: a newly
advertised tool is not automatically authorized. Coding and filesystem work
need explicit workspace/process scopes, not unrestricted model-controlled shell
or filesystem access. Credential handling remains outside model-visible tools.

### 6.4 Rig reuse and Zephium authority

Prefer [Rig](https://github.com/0xPlaygrounds/rig) where it demonstrably reduces
provider, streaming, tool-protocol, search, embedding, or multimodal integration
work. Evaluate its host-driven agent loop where useful. Start with a pinned
upstream dependency and narrow Zephium adapters; contribute general fixes
upstream. Maintain a fork only for a demonstrated essential gap that cannot be
reasonably addressed through those boundaries.

Adoption must prove admitted request serialization, bounded responses, streaming,
tool mapping, cancellation, usage, citation preservation, privacy-safe errors,
and dependency/resource cost against real calls. A provider list or an adjacent
canvas product does not prove native efficiency. Inspect a pinned version;
features on upstream main are not automatically available in a release.

Rig is not the durable Work store, capability authority, effect-settlement
system, native browser owner, or UI model. Its internal checkpoints must not
become the product persistence format or resurrect worker authority. Zephium
retains control of requests and tool dispatch, scheduling, immutable approvals,
bounded delegation, recovery, and publication. Reuse a component only when this
boundary remains intact; do not maintain parallel provider implementations
without a concrete reason.

## 7. Domain and persistence model

Authoritative product state remains in Rust under the existing functional-core,
imperative-shell architecture. Svelte is a projection that sends typed intent.
Side effects flow through explicit ports and adapters.

The target domain adds, without collapsing existing browser concepts:

- `WorkId`: durable profile-owned ULID;
- `ActorId`, `RunId`, `PlanId`, and `TaskId`: durable ULIDs where identity
  survives a process;
- `ResourceId` and `ArtifactId`: durable identities for attached state;
- `ContextId`: browser automation identity independent of tab identity;
- runtime-only native view handles: compact process-local integers;
- `WorkView`: replaceable presentation state separate from Work semantics.

Profile/Space ownership, objective-independent Work identity, area membership,
and shared-resource relationships are Rust semantics. Canvas coordinates,
viewport, selection, expansion, and gestures are presentation. Svelte emits
typed intents and uses an explicit generated-contract-to-presentation adapter;
it does not create an independent Work model. Manual and agent operations
converge on the same validated domain commands.

Versioned IPC validates the native caller and profile and carries precision-safe
revisions, exact command identities, and bounded resynchronization. Preserve
command replay, stale-write rejection, historical plan joins, and honest
pending/conflict/unknown/resynchronizing states. Transient activity is admitted
only when profile, Work, execution, attempt, owner, and durable basis revision
match. Handle missed, reordered, duplicated, and stale delivery. A callback or
frontend render never establishes durable completion or worker authority.

Rust owns admitted execution independently of UI observation. Persist Work
facts, results, unresolved drafts, and separate presentation checkpoints across
restart. Restore neither stale worker authority nor an invented live status;
continuation requires actual fresh runtime admission. Close handling preserves
drafts and exposes unresolved operations without conflating hiding Work with
cancelling execution.

Persistence uses normalized envelopes for stable identity, ownership, status,
timestamps, ordering, and relationships plus typed, versioned payloads for
component-specific data. Current state is accompanied by an append-only
semantic audit of meaningful execution and approval transitions. This is not
full event sourcing and must not become a dump of model reasoning, DOM data,
or high-frequency UI movement.

Large immutable artifacts use a content-addressed BLAKE3 blob vault with
reference accounting, bounded metadata, atomic writes, and garbage collection.
Search begins with SQLite FTS5, recency, explicit relationships, and graph
signals. An `EmbeddingPort` remains optional; prefiltered bounded exact cosine
search is sufficient before a measured need for a managed vector service or
immature embedded ANN database.

The current local database relies on operating-system file protection and does
not claim application-level encryption. Persistence contracts reserve an
encryption migration seam without inventing a false guarantee today.

## 8. Frontend and component stack

The fixed foundation remains Svelte 5, strict TypeScript, Vite, Tailwind CSS v4,
narrow Bits UI adoption, Hugeicons, shared typography tokens, generated typed
IPC, and native OS surfaces where they fit. `frontend.md` is the working
frontend contract.

The following additions are preferred because each remains replaceable behind
a Zephium-owned domain boundary:

- **Tiptap/ProseMirror:** limited note/document editing schema;
- **`@xyflow/svelte`:** the existing viewport/interaction adapter, subject to
  representative resource budgets; it never owns Work semantics, persistence,
  or layout decisions;
- **Pointer Events and Svelte actions:** spatial movement and resize; native
  drag-and-drop only at OS or WebView boundaries, with no generic DnD framework
  by default;
- **Our own SVG charts:** the semantic chart renderer in `shared/ui/data/Chart`,
  lazy-loaded behind bounded chart contracts, with no charting dependency. It
  replaced LayerChart and d3 on measured cost: about 43 KB instead of 360 KB;
- **TanStack Table/Virtual:** large tables and lists;
- **MapLibre:** lazy-loaded only when a map is present;
- **CodeMirror 6:** lazy-loaded only for editable code surfaces;
- **Paraglide:** compile-time typed localization.

Markdown is parsed and sanitized into typed blocks at a trusted boundary. Raw
`{@html}`, arbitrary CSS, model-produced JavaScript, and executable theme
payloads are prohibited.

### 8.1 Performance rules

The frontend pays for capabilities only when visible:

- Work, editor, map, chart, code, and spatial libraries are lazy-loaded;
- one active rich editor exists unless a measured use case proves otherwise;
- long lists and large canvases are spatially culled and virtualized;
- live native browser contexts are explicitly budgeted, not created per card;
- animation is limited to compositor-friendly properties and respects reduced
  motion;
- model streams are coalesced before projection rather than causing a render
  per token;
- spatial movement is transient UI state and persists at bounded semantic
  checkpoints, not every pointer event.

Browse with Work unused pays no Work bundle or worker cost. Manual Work loads
the presentation capabilities it uses, with no AI requests or workers. No
active execution means no execution polling or worker wakeups. A durable Work
existing on disk is not a reason to keep its runtime alive.

## 9. Design system and native quality

Zephium should feel like a premium operating-system application: quiet,
legible, responsive, and obvious despite the complexity underneath. The aim is
Apple-level quality, not a platform-neutral imitation of macOS. Windows follows
Windows interaction conventions with the same restraint and finish.

The design system owns semantic tokens, density, typography, motion,
accessibility, control states, layout primitives, visual components, platform
adaptation, and documented composition patterns. User customization changes
typed validated tokens and product options. It never injects arbitrary CSS or
script.

On macOS, the complete native Tao window uses the best supported native window
material: Liquid Glass on macOS 26 and later when the pinned native stack can
prove it, with vibrancy as the fallback. Restrained Zephium-owned blur and
translucency are allowed where they improve separation and remain within
measured contrast, CPU/GPU, and power budgets. There is no blanket CSS blur ban
and no requirement to rewrite an upstream component solely for an unused blur
rule. Avoid gratuitous effects, especially across large moving canvases.
On Windows, prefer proven Mica and native materials with a solid Windows 10
fallback; Acrylic is retained only
where resize, GPU, power, and contrast measurements justify it. Cross-WebView
surfaces retain their native boundaries. Browse's native menus remain native;
Work's toolbar and profile popovers are accessible Zephium-owned controls in
its presentation surface, not a generic privileged overlay across web pages.

A specialist design agent may later own tokens, components, interaction polish,
visual QA, and accessibility evidence. It does not change Rust authority, IPC,
security boundaries, domain semantics, native layout invariants, or persistence
without an explicit architecture review.

Localization is structured from the beginning, including pluralization,
formatting, pseudo-locale, text expansion, and RTL validation. English may be
the only launch translation. AI-assisted translations can be reviewed later;
model output never bypasses the localization catalog and QA process.

## 10. Security and privacy

The enforced browser trust model in `security-model.md` remains non-negotiable.
Work adds more untrusted data and therefore must narrow, not weaken, those
boundaries.

Core rules include:

- the Rust shell is the only product authority;
- privileged application WebViews are local, navigation-locked projections;
- raw pages, agent pages, extensions, models, remote services, and connected
  tools receive no generic native bridge;
- model output is a proposal decoded into bounded typed values;
- page and service content is always labelled hostile data, never instruction;
- source-to-sink policy checks data sensitivity, origin, destination, account,
  and effect before execution;
- secrets are never included in page snapshots, prompts, logs, artifacts, or
  model-visible fields;
- plan approval is revalidated at the actual effect boundary;
- cancellation and user takeover revoke active automation authority;
- diagnostic traces default to semantic metadata and require explicit local
  opt-in for sensitive payload capture;
- there is no application-owned product telemetry.

MCP, CLI, filesystem, terminal, and external-agent adapters default off. A user
enables them through profile-scoped capability leases. Hosted entitlements do
not silently expose local tools or data.

## 11. Reliability, performance, and observability

Every production subsystem defines before implementation:

- ownership and lifetime;
- bounded memory, concurrency, queues, payloads, retries, and timeouts;
- cancellation and shutdown behavior;
- crash and partial-failure recovery;
- durability boundary and migration plan;
- trust classification and authorization point;
- performance and resource measurements;
- deterministic tests and representative integration/evaluation evidence.

Performance budgets are derived from a committed baseline on supported
hardware, then enforced as reviewed deltas. Measures include startup and input
latency, wall-clock task latency, CPU time, resident memory, native view/process
count, GPU/compositor pressure, wakeups, energy impact, disk and network bytes,
and battery behavior. A feature that is fast in a synthetic microbenchmark but
causes idle wakeups or multiplies native WebViews is not optimized.

Runtime behavior uses bounded workers, backpressure, cancellation tokens,
generation-aware results, typed failures, idempotent or explicitly
non-idempotent effects, and no blind retry. User-visible success means the
effect was verified, not merely requested.

## 12. First public release

The first release is a complete product rather than a demo or deliberately
crippled MVP. It includes:

- a polished production Browse experience on macOS and Windows;
- native blocking and practical major-extension compatibility;
- Work with clarify/plan/approve/visible execution/persistent result;
- shared Space organization, persistent manual Work, and a continuous
  Browse/Work transition with existing tab identity;
- a high-quality useful set of native visual components;
- production agentic browsing;
- tasks, notes, user knowledge, and memory integrated with Work;
- local models, BYOK, and hosted AI through one architecture;
- Browse-side contextual AI;
- the security, recovery, resource, and quality evidence needed for real users.

Linux remains in the architecture and build discipline, with full product
parity expected after the first release. Major Chrome extensions are the
compatibility target; Zephium does not claim the entire Chrome extension API.

The following are deliberately post-release but must attach through existing
ports and identities rather than require a product rewrite:

- cloud browser and Work execution;
- mobile progress, approval, and continuation;
- real-time multiplayer collaboration;
- teams and organizations;
- domain capability packs;
- remote long-running workers and broader cloud synchronization.

## 13. Business and openness

The browser and capable local Work remain open source. Free/local value can
include Browse, blocking, extensions, customization, local Work, local models,
BYOK, notes/tasks/memory, local integrations, and external-agent adapters.

Paid value corresponds to real hosted value: Zephium-hosted AI, compute,
premium models, cloud browser execution, long-running and parallel hosted
agents, synchronization, continuation, collaboration, and organizational
capabilities. The browser is not artificially impaired to manufacture a
subscription.

The hosted service may remain private and live in a separate repository. Open
browser protocols and data formats should still make local use, export, and
practical self-hosting possible where doing so does not falsely promise a fully
supported hosted product.

## 14. Delivery architecture

Implementation proceeds through production vertical slices that leave evidence
behind:

1. Inspect and preserve the integrated runtime/frontend foundation. Establish
   Space-organized, objective-independent Work persistence and generated
   contracts without rewriting established execution history.
2. Deliver the real native transition, canvas controls, tab continuity, manual
   objects, contextual editing, and reopen behavior against those contracts.
3. Integrate model/provider reuse and contextual orchestration, then connect
   clarification, visible approach, exact approval, bounded execution, tasks,
   evidence, and persistent native results through the same environment.
4. Qualify bounded delegation and non-browser capabilities alongside browser
   resources, intervention, cancellation, and result review. Do not build a
   broad service/MCP marketplace in this pass.
5. Complete real public workflows, then signed-in reading and an explicitly
   approved reversible write with restoration. Complete the release matrix on
   macOS first and Windows afterward; shared contracts remain Windows-compatible.

The integrated branch is `work-mode-integration`, baseline `c5a7b08e`. Preserve
both integrated histories and recovery refs
`backup/runtime-before-work-ui-integration` and
`backup/frontend-main-before-work-ui-integration`. Do not reset, rebase,
separate, or replace the histories, and do not push without explicit user
instruction. Preserve unrelated work. Use Node 24.19.0, compatible with the
repository's `>=24.18.0 <25` requirement, not system Node 26.

Profile schema 18 is the integration baseline; migrate forward from it. Reject
incompatible frontend-only migration-14 resource databases. Reset only identified
disposable development data, never silently accept an incompatible schema or
reset ordinary user data. Resolve process-global test-admission collisions
through isolation or serial execution; do not raise production resource bounds.

Each slice must have explicit bounds, typed contracts, migrations where needed,
proportional tests, metrics, and reviewable commits. Use real vertical behaviors
to settle architecture rather than completing another isolated backend phase
or a fixture-only canvas. Documentation is not implementation evidence.

### 14.1 Qualification and completion

Compilation and green unit tests are necessary but insufficient. Through the
actual native Work UI, qualify:

- an unscripted single-agent dependency investigation with an unknown route
  and a result judged useful by a human;
- a primary agent delegating bounded research for a Svelte/XYFlow/charting
  comparison, consuming compact child outcomes, and publishing persistent
  native artifacts;
- realistic public issue trackers or another non-static web application,
  not only controlled fixtures or static reference pages;
- cancellation/interruption during genuine execution, abrupt restart, and
  completed-result reopening without revived worker authority;
- exact browser promotion and human takeover where integrated;
- signed-in reading of a designated Notion test page, then a concrete,
  explicitly approved reversible title change and restoration, with each remote
  outcome independently verified. Obtain the needed test account and user
  participation; uncertain settlement remains uncertain.

Use relevant Rust, Svelte/TypeScript, WebKit, native, security, accessibility,
bundle, and style gates. Fixtures remain useful for deterministic component
tests but must be unreachable from production import/execution graphs. Test
manual Work with AI off and Browse with Work off as real configurations.

Authorized OpenAI development qualification uses `store=true` where appropriate
with useful non-sensitive metadata for inspectable provider traces. Keep local
diagnostics privacy-safe for policy, native dispatch, resources, timing,
settlement, and recovery facts the provider cannot observe. This does not
change ordinary product retention defaults and never permits logging secrets.
Measure task success, human intervention, tokens per observation/turn, elapsed
time, tool failures, resource concurrency, and honest uncertainty. Human review
judges usefulness; a validator that already knows an answer is not that review.

Commits should represent coherent production changes: a contract and its
tests, a platform adapter and its gates, a vertical behavior and its evidence.
Avoid both line-by-line noise and one or two unreviewable commits containing an
entire subsystem. Generated files and migrations travel with the change that
requires them.

## 15. Decision discipline

Zephium does not need an ADR ceremony for ordinary implementation. This
document and the subsystem specifications hold accepted decisions. A short ADR
is justified only when a choice is expensive to reverse or changes one of:

- trust, secrets, permissions, or data boundaries;
- durable identity, persistence, sync, or migration shape;
- cross-platform native architecture;
- public extension, tool, model, or cloud protocols;
- ownership of authoritative state;
- a release guarantee or irreversible dependency.

An ADR records context, decision, alternatives, evidence, consequences, and a
revisit trigger. It is not a status report and does not substitute for code,
tests, benchmarks, or this product source of truth.

## 16. Non-goals and anti-patterns

Do not:

- force Work, AI, notes, tasks, or analytics into normal browsing;
- turn Work into chat with a decorative graph;
- treat tabs as the Work data model;
- place many full interactive WebViews across a canvas;
- expose model chain-of-thought, raw DOM, CDP logs, selectors, or tool noise as
  product state;
- generate arbitrary privileged HTML, CSS, or JavaScript;
- make notes/tasks/analytics independent application clones;
- send broad history or personal context to a model because it may be useful;
- let provider choice bypass capability or security requirements;
- promise seamlessness by silently degrading behavior;
- auto-arrange spatial state without semantic purpose;
- add polling, background workers, or heavyweight bundles to Browse when a
  capability is unused;
- confuse an API request with a verified real-world effect;
- use cloud dependence to weaken the open local product.

## 17. Success

Browse succeeds when people switch because Zephium is an exceptional browser,
even if they never enable AI.

Work succeeds when a person can express a substantial goal, understand the
resulting plan, watch humans and agents construct useful state, intervene
without losing continuity, and leave with real resources, decisions, tasks,
knowledge, and artifacts—not merely a transcript.

The combined product succeeds when that power feels calm and native while its
resource use, security, stability, and engineering quality remain worthy of a
browser used by thousands and eventually millions of people.
