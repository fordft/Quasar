import type { WorkMemoryV1, WorkSiteRowV1, WorkSkillRowV1 } from "$shared/ipc/bindings";

const DAY = 86_400_000;
const now = Date.now();
const at = (days: number) => String(now - days * DAY);

export const sites: WorkSiteRowV1[] = [
  { site: "slack.com", name: "Slack", access: "always", sensitive: false },
  { site: "notion.so", name: "Notion", access: "always", sensitive: false },
  { site: "linkedin.com", name: "LinkedIn", access: "ask", sensitive: false },
  { site: "reddit.com", name: "Reddit", access: "never", sensitive: false },
  { site: "chase.com", name: "chase.com", access: "ask", sensitive: true },
];

export const memories: WorkMemoryV1[] = [
  {
    id: "01M3F2YJTVB6S1M73MERQER7NA",
    text: "Prefers aisle seats on long flights",
    kind: "preference",
    work: "01M3CV1H7HRABAGVH1T8HXH2DD",
    source: "Plan my YC batch trip from Warsaw",
    created_ms: at(2),
    used_ms: at(0),
  },
  {
    id: "01M3F2YJTVB6S1M73MERQER7NB",
    text: "Anna Kowalska leads design and reviews every release",
    kind: "person",
    work: "01M3CV1H7HRABAGVH1T8HXH2DE",
    source: "What do I need to do today",
    created_ms: at(5),
  },
  {
    id: "01M3F2YJTVB6S1M73MERQER7NC",
    text: "Quasar ships from the work-mode-integration branch",
    kind: "project",
    created_ms: at(12),
    used_ms: at(1),
  },
  {
    id: "01M3F2YJTVB6S1M73MERQER7ND",
    text: "Budget for the stay is about $4,000 a month",
    kind: "preference",
    work: "01M3CV1H7HRABAGVH1T8HXH2DD",
    source: "Plan my YC batch trip from Warsaw",
    created_ms: at(2),
  },
  {
    id: "01M3F2YJTVB6S1M73MERQER7NE",
    text: "Writes in British English",
    kind: "preference",
    created_ms: at(40),
  },
];

export const skills: WorkSkillRowV1[] = [
  {
    name: "compare-and-choose",
    description: "Compare a few options on what matters and recommend one.",
    builtin: true,
    customized: false,
    enabled: true,
    tools: [],
    role: null,
  },
  {
    name: "fix-a-bug",
    description:
      "Fix a bug from an issue: reproduce it, change the code, run the tests, draft the PR.",
    builtin: true,
    customized: false,
    enabled: true,
    tools: [],
    role: null,
  },
  {
    name: "job-search",
    description: "Find open roles that fit, with the company, pay and why each fits.",
    builtin: false,
    customized: true,
    enabled: true,
    tools: [],
    role: null,
  },
  {
    name: "today-from-my-messages",
    description: "What to do today, from Slack and Gmail, each with where it came from.",
    builtin: true,
    customized: false,
    enabled: false,
    tools: [],
    role: null,
  },
  {
    name: "trip-planning",
    description:
      "Plan a trip end to end: where to stay, how to get there and back, entry rules, and a day-by-day plan with the total cost.",
    builtin: true,
    customized: false,
    enabled: true,
    tools: ["ask", "start_part", "create", "finish"],
    role: null,
  },
  {
    name: "weekly-review",
    description: "Review my week from my calendar and notes, and plan the next one.",
    builtin: false,
    customized: false,
    enabled: true,
    tools: [],
    role: "light",
  },
];

export function skillText(name: string): string {
  if (name === "weekly-review")
    return `---
name: weekly-review
description: Review my week from my calendar and notes, and plan the next one.
role: light
---

# Weekly review

## First
- Read my notes from this week (search_notes "week") and my open tabs.

## Result
- A plan for next week: three priorities, then the days.
- Keep it short; no document unless I ask.
`;
  const skill = skills.find((row) => row.name === name)!;
  return `---
name: ${skill.name}
description: ${skill.description}
tools: [${skill.tools.join(", ")}]
---

# Trip planning

## First
- Know the dates (or length and month), where from, who travels and the budget.
- A known event fixes the dates and the place.

## Parts (start them in one turn, they run in parallel)
- **Stay**: browser part on airbnb.com. It returns three homes as picks.
- **Flights**: browser part on google.com/travel/flights.
- **Entry**: research part: visa or ESTA, passport validity.

## Result
- One **plan** from the flight out to the flight back, with a total.
`;
}
