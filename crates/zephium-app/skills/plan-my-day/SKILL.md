---
name: plan-my-day
description: Plan the person's day from their calendar, mail, messages and Quasar tasks: a timed plan and a checklist, each item linked to its source.
tools: [day_sources, list_tasks, start_part, create, finish]
---
# Plan my day

## First
- Call day_sources. It returns where the person's day lives and how to read each place; the first time, it asks the person once and remembers the answer. Never ask about sources yourself, and never plan from generic advice.

## Read (in one turn, in parallel)
- One part per app day_sources returned, with the part it gives (title, helper, service) and a goal for today's date:
  - **Calendar**: today's events with start and end times, title, place or call link, and who.
  - **Gmail**: mail from the last two days that asks something of the person; skip newsletters and notifications.
  - **Slack**: unread direct messages, mentions and threads waiting on them.
  - **Linear**, **GitHub**, **Notion**: what is assigned to them and due or waiting on them.
- Meanwhile call list_tasks for their Quasar tasks when day_sources includes them: every open task, overdue and today's first, then upcoming and undated; plan today's and overdue ones, and list the rest only when they fit the day.
- These parts read the person's own accounts; the app asks them first. Never put their messages' text into a search.

## Result
- A **plan** for the rest of today, from now on: every step has when as a time range ("09:30–10:00"). The calendar's events at their times (kind event, place or call link when known), and focused blocks for the most important work between them (kind task, "11:00–12:30"), each naming the one task or reply it is for. What is due or blocks someone comes first. Every step comes from a source: an event, a message, a task. Nothing generic ("check email", "take a break"). The plan is the timeline, not the checklist: leave checkable off.
- A **list**, style todo, as the checklist: each thing to do today as a verb-first title ("Reply to Ana about the Q3 deck"), with from: the app (Slack, Gmail, Linear, Quasar), its host, who, when, a short quote of their words and the link back; a Quasar task keeps its own title and has from.app Quasar with no quote. due when a time is stated. priority high only for what blocks someone or has a fixed time today. At most fifteen items, most urgent first; merge duplicates across apps.
- The **reply**: a headline with the shape of the day ("4 meetings, 6 things to do; Ana's deck first") and at most two sentences; no restatement of the plan or list.
- A source a part could not read shows its fix on its row (sign in, allow, use the connection). The plan and list hold only what was read; never an item to check or retry a source. When nothing could be read, there is no plan or list and the reply says so in one sentence.
