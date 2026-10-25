import type {
  CodeView,
  DiagramView,
  DiffView,
  DocumentObjectView,
  DraftView,
  FileView,
  FolderView,
  ListView,
  MediaView,
  NoteView,
  ObjectView,
  PageObjectView,
  PicksView,
  PickView,
  PlanView,
  PlotView,
  ProjectView,
  ReplyView,
  SheetView,
  SubjectView,
} from "../lib/board/types";
import { centreSize } from "../components/objects/centre";

/**
 * Every object as the new runtime fills it, from the acceptance tasks (spec §9).
 * Pictures are the QA profile's admitted images, copied read-only into
 * node_modules/.work-look/objects; without them a picture simply does not load.
 */
const LOOK = "/node_modules/.work-look/objects";
const picture = (name: string, width: number, height: number) => ({
  src: `${LOOK}/${name}`,
  width,
  height,
});

const replySmall: ReplyView = {
  kind: "reply",
  id: "reply-small",
  headline: "30 EUR is about 128 PLN",
  text: "At today's rate of **4.27 PLN** per euro. Card payments usually add 1–3% on top of the mid-market rate.",
  figures: [],
  points: [],
};

const replyFigures: ReplyView = {
  kind: "reply",
  id: "reply-figures",
  headline: "A multi-tenant SaaS on AWS, with the AI work on a queue",
  text: "Users reach a **Next.js** app through Cloudflare. The API checks the tenant on every request and hands slow model calls to workers through **SQS**, so a 20-second completion never blocks a page. Postgres holds the data, with `pgvector` until search outgrows it.",
  figures: [
    { label: "MVP per month", value: "$550–9,500" },
    { label: "At growth", value: "$9,800–153K", note: "model usage is most of it" },
    { label: "Services to run", value: "7" },
  ],
  points: [
    "Tenant id on every row, enforced by row-level security",
    "Timeouts, bounded retries and a budget per tenant",
    "Model providers behind one adapter, so they can change",
  ],
};

const stays: PicksView = {
  kind: "picks",
  id: "stays",
  title: "Homes near YC for the batch",
  facet: "stay",
  items: [
    {
      name: "Lux 2br/2ba next to YC",
      subtitle: "Entire flat · Dogpatch · 2 blocks from YC",
      picture: picture("3a91a46d12.jpg", 1200, 900),
      logo: "airbnb.com",
      url: "https://www.airbnb.com/rooms/965072398758262802",
      price: { display: "$6,900 / month", amount: 6900, currency: "USD" },
      facts: [
        { label: "Bedrooms", value: "2", kind: "text" },
        { label: "Workspace", value: "Office nook", kind: "yes" },
        { label: "Washer", value: "In unit", kind: "yes" },
        { label: "Min. stay", value: "30 nights", kind: "text" },
      ],
      rating: { value: 4.92, max: 5, count: 61 },
      why: "Two blocks from YC, a real desk, and the host takes batch bookings.",
      tags: ["Walk to YC", "Superhost"],
      recommended: true,
    },
    {
      name: "Sunny studio in Mission Bay",
      subtitle: "Entire studio · Mission Bay · 12 min to YC",
      picture: picture("9fb9978c4c.webp", 720, 540),
      logo: "airbnb.com",
      url: "https://www.airbnb.com/rooms/47980567",
      price: { display: "$4,350 / month", amount: 4350, currency: "USD" },
      facts: [
        { label: "Bedrooms", value: "Studio", kind: "text" },
        { label: "Workspace", value: "Desk", kind: "yes" },
        { label: "Washer", value: "In building", kind: "partial" },
        { label: "Min. stay", value: "30 nights", kind: "text" },
      ],
      rating: { value: 4.81, max: 5, count: 138 },
      why: "The cheapest place with a desk inside a 15-minute walk.",
      tags: ["Monthly discount"],
      recommended: false,
    },
    {
      name: "Room in a founders' house",
      subtitle: "Private room · SoMa · 20 min to YC",
      picture: picture("ffa4fad393.webp", 1200, 600),
      logo: "airbnb.com",
      url: "https://www.airbnb.com/rooms/1181130",
      price: { display: "$2,900 / month", amount: 2900, currency: "USD" },
      facts: [
        { label: "Bedrooms", value: "Shared flat", kind: "text" },
        { label: "Workspace", value: "Shared", kind: "partial" },
        { label: "Washer", value: "In unit", kind: "yes" },
        { label: "Private bath", value: "No", kind: "no" },
      ],
      rating: { value: 4.7, max: 5, count: 24 },
      why: "Half the price, and five other founders under one roof.",
      tags: ["Community"],
      recommended: false,
    },
  ],
};

const flights: PicksView = {
  kind: "picks",
  id: "flights",
  title: "Flights to San Francisco",
  facet: "flight",
  items: [
    {
      name: "Lufthansa via Frankfurt",
      logo: "lufthansa.com",
      url: "https://www.lufthansa.com",
      price: { display: "$1,142", amount: 1142, currency: "USD" },
      facts: [
        { label: "Bag", value: "23 kg included", kind: "yes" },
        { label: "Change", value: "Fee applies", kind: "partial" },
      ],
      why: "The shortest trip that lands before the Monday kickoff.",
      tags: [],
      recommended: true,
      when: "Sun 4 Jan",
      route: {
        from: "WAW",
        to: "SFO",
        depart: "07:05",
        arrive: "13:05",
        duration: "15 h 00 m",
        stops: 1,
        via: ["FRA"],
        carrier: "Lufthansa",
        carrierHost: "lufthansa.com",
      },
    },
    {
      name: "KLM via Amsterdam",
      logo: "klm.com",
      url: "https://www.klm.com",
      price: { display: "$1,068", amount: 1068, currency: "USD" },
      facts: [
        { label: "Bag", value: "23 kg included", kind: "yes" },
        { label: "Change", value: "Free", kind: "yes" },
      ],
      tags: [],
      recommended: false,
      when: "Sun 4 Jan",
      route: {
        from: "WAW",
        to: "SFO",
        depart: "06:00",
        arrive: "12:40",
        duration: "15 h 40 m",
        stops: 1,
        via: ["AMS"],
        carrier: "KLM",
        carrierHost: "klm.com",
      },
    },
    {
      name: "LOT and United via Chicago",
      logo: "lot.com",
      url: "https://www.lot.com",
      price: { display: "$986", amount: 986, currency: "USD" },
      facts: [
        { label: "Bag", value: "Carry-on only", kind: "no" },
        { label: "Change", value: "Fee applies", kind: "partial" },
      ],
      tags: ["Cheapest"],
      recommended: false,
      when: "Sun 4 Jan",
      route: {
        from: "WAW",
        to: "SFO",
        depart: "10:25",
        arrive: "19:50",
        duration: "18 h 25 m",
        stops: 2,
        via: ["ORD", "DEN"],
        carrier: "LOT",
        carrierHost: "lot.com",
      },
    },
  ],
};

const products: PicksView = {
  kind: "picks",
  id: "products",
  title: "LEGO Architecture sets",
  facet: "product",
  items: [
    {
      name: "Neuschwanstein Castle",
      subtitle: "LEGO Architecture · 21063",
      picture: picture("c89b72e433.jpg", 1500, 1132),
      logo: "lego.com",
      url: "https://www.lego.com/en-us/product/neuschwanstein-castle-21063",
      price: { display: "$279.99", amount: 279.99, currency: "USD" },
      facts: [
        { label: "Pieces", value: "3,455", kind: "text" },
        { label: "In stock", value: "Yes", kind: "yes" },
        { label: "Build time", value: "about 12 h", kind: "text" },
      ],
      rating: { value: 4.8, max: 5, count: 412 },
      why: "The most model for the money: twelve times the pieces of London.",
      tags: ["18+"],
      recommended: true,
    },
    {
      name: "Sagrada Família",
      subtitle: "LEGO Architecture · 21065",
      picture: picture("b7b16b22a0.png", 703, 800),
      logo: "lego.com",
      url: "https://www.lego.com/en-us/product/sagrada-familia-21065",
      price: { display: "$799.99", amount: 799.99, currency: "USD" },
      facts: [
        { label: "Pieces", value: "12,060", kind: "text" },
        { label: "In stock", value: "Backorder", kind: "partial" },
        { label: "Build time", value: "about 40 h", kind: "text" },
      ],
      rating: { value: 4.9, max: 5, count: 96 },
      tags: ["18+", "Largest"],
      recommended: false,
    },
    {
      name: "London",
      subtitle: "LEGO Architecture · 21034",
      picture: picture("816667de11.jpg", 800, 800),
      logo: "lego.com",
      url: "https://www.lego.com/en-us/product/london-21034",
      price: { display: "$39.99", amount: 39.99, currency: "USD" },
      facts: [
        { label: "Pieces", value: "468", kind: "text" },
        { label: "In stock", value: "Yes", kind: "yes" },
        { label: "Build time", value: "about 2 h", kind: "text" },
      ],
      rating: { value: 4.6, max: 5, count: 1840 },
      tags: ["12+"],
      recommended: false,
    },
  ],
};

const videos: PicksView = {
  kind: "picks",
  id: "videos",
  title: "Lectures to start with",
  facet: "video",
  items: [
    {
      name: "Compilers: lexical analysis",
      subtitle: "Stanford CS143 · Alex Aiken",
      picture: picture("479469f204.jpg", 480, 360),
      logo: "youtube.com",
      url: "https://www.youtube.com/watch?v=FgzyLoSkL5k",
      duration: "17:42",
      facts: [],
      why: "The clearest first hour on scanners, and it matches week 1 of the plan.",
      tags: ["Week 1"],
      recommended: true,
    },
    {
      name: "Crafting Interpreters, chapter 4",
      subtitle: "Robert Nystrom · book",
      logo: "craftinginterpreters.com",
      url: "https://craftinginterpreters.com/scanning.html",
      duration: "45 min read",
      facts: [{ label: "Free online", value: "Yes", kind: "yes" }],
      tags: ["Week 1"],
      recommended: false,
    },
    {
      name: "MIT 6.035 lecture notes",
      subtitle: "MIT OpenCourseWare",
      logo: "ocw.mit.edu",
      url: "https://ocw.mit.edu/courses/6-035-computer-language-engineering-spring-2010/",
      duration: "10 weeks",
      facts: [{ label: "Assignments", value: "With solutions", kind: "yes" }],
      tags: ["Weeks 2–4"],
      recommended: false,
    },
  ],
};

const trip: PlanView = {
  kind: "plan",
  id: "trip",
  title: "Your YC batch trip",
  checkable: true,
  steps: [
    {
      when: "Sun 4 Jan",
      title: "Fly Warsaw → San Francisco",
      detail: "Lufthansa via Frankfurt, lands 13:05. BART from SFO is about $10 and 30 min.",
      kind: "travel",
      cost: "$1,142",
      place: "WAW → SFO",
      pick: { name: "Lufthansa via Frankfurt", logo: "lufthansa.com" },
      done: true,
    },
    {
      when: "Sun 4 Jan",
      title: "Check in near YC",
      detail: "Code arrives by message the day before; the host is on site until 18:00.",
      kind: "stay",
      cost: "$6,900 / mo",
      place: "Dogpatch",
      pick: { name: "Lux 2br/2ba next to YC", picture: picture("3a91a46d12.jpg", 1200, 900) },
    },
    {
      when: "Mon 5 Jan",
      title: "Batch kickoff",
      detail: "Bring your passport and the signed SAFE; doors at 09:30.",
      kind: "event",
      place: "YC, Mission Bay",
    },
    {
      when: "Tue 6 Jan",
      title: "Open a US bank account",
      detail: "Mercury works remotely with the Delaware C-corp documents.",
      kind: "task",
    },
    {
      when: "Wed 7 Jan",
      title: "Get a US SIM",
      detail: "An eSIM from Mint Mobile is $15 for the first month.",
      kind: "task",
      cost: "$15",
    },
    {
      when: "Thu 12 Mar",
      title: "Demo Day",
      kind: "milestone",
      place: "Pier 48",
    },
    {
      when: "Sat 14 Mar",
      title: "Fly home",
      detail: "Return leg of the same Lufthansa booking.",
      kind: "travel",
      place: "SFO → WAW",
    },
  ],
  total: { label: "Trip total", value: "$22,390" },
};

const today: ListView = {
  kind: "list",
  id: "today",
  title: "Today",
  style: "todo",
  items: [
    {
      title: "Send Maya the pricing deck before her 3 pm call",
      due: "Before 15:00",
      priority: "high",
      from: {
        host: "slack.com",
        app: "Slack",
        who: "Maya Chen",
        when: "09:12",
        quote:
          "Could you send me the latest pricing deck before my 3pm with Acme? Their CFO will be on.",
        url: "https://acme.slack.com/archives/C024/p1727",
      },
    },
    {
      title: "Review the onboarding PR",
      detail: "Two comments are still open on the invite flow.",
      from: {
        host: "github.com",
        app: "GitHub",
        who: "jonas-k",
        when: "Yesterday",
        url: "https://github.com/acme/app/pull/482",
      },
    },
    {
      title: "Confirm the dinner with the Accel partners",
      due: "Today",
      from: {
        host: "mail.google.com",
        app: "Gmail",
        who: "Sarah Lin",
        when: "08:40",
        quote: "Are we still on for 7:30 at Nopa? Happy to move it if the batch runs late.",
        url: "https://mail.google.com/mail/u/0/#inbox/FMfcgz",
      },
    },
    {
      title: "Reply to the Stripe Atlas question about your EIN",
      from: {
        host: "mail.google.com",
        app: "Gmail",
        who: "Stripe Atlas",
        when: "Mon",
        url: "https://mail.google.com/mail/u/0/#inbox/QgrcJHs",
      },
    },
  ],
};

const providers: SheetView = {
  kind: "sheet",
  id: "providers",
  title: "Providers for this architecture",
  columns: [
    { label: "Provider", kind: "entity" },
    { label: "From", kind: "money", currency: "USD", best: "min" },
    { label: "Managed Postgres", kind: "yes_no" },
    { label: "Queues", kind: "yes_no" },
    { label: "Edge CDN", kind: "yes_no" },
    { label: "Ops effort", kind: "rating", best: "min" },
    { label: "Regions", kind: "number", best: "max" },
    { label: "Fits here", kind: "tag" },
  ],
  rows: [
    {
      cells: ["AWS", "180", "yes", "yes", "partial", "4/5", "34", "Backend"],
      entity: { logo: "aws.amazon.com" },
    },
    {
      cells: ["Vercel", "20", "partial", "no", "yes", "1/5", "18", "Web app"],
      entity: { logo: "vercel.com" },
    },
    {
      cells: ["Hetzner", "5", "no", "no", "no", "5/5", "6", "Cheap compute"],
      entity: { logo: "hetzner.com" },
    },
    {
      cells: ["Cloudflare", "5", "partial", "yes", "yes", "2/5", "330", "Edge"],
      entity: { logo: "cloudflare.com" },
    },
  ],
  note: "Starting prices for the smallest production tier, checked 28 Sep.",
};

const costs: PlotView = {
  kind: "plot",
  id: "costs",
  title: "Monthly cost at launch",
  spec: {
    kind: "donut",
    x: { label: "Category" },
    y: { label: "Cost", format: "money", currency: "USD" },
    headline: { label: "A month at launch", value: "$1,840" },
    basis: "Midpoints of the MVP estimates; model usage assumed at 2M tokens a day.",
    series: [
      {
        name: "Cost",
        points: [
          { x: "Model API", y: 820 },
          { x: "Database", y: 380 },
          { x: "Hosting and edge", y: 260 },
          { x: "Workers", y: 210 },
          { x: "Monitoring", y: 110 },
          { x: "Email and auth", y: 60 },
        ],
      },
    ],
  },
};

const growth: PlotView = {
  kind: "plot",
  id: "growth",
  title: "Model spend as usage grows",
  spec: {
    kind: "area",
    x: { label: "Month", kind: "category" },
    y: { label: "Spend", format: "money", currency: "USD" },
    headline: { label: "By month 12", value: "$18.4K" },
    basis: "At $3 per million input tokens and 40% cache hits.",
    series: [
      {
        name: "Model API",
        points: [
          "Jan",
          "Feb",
          "Mar",
          "Apr",
          "May",
          "Jun",
          "Jul",
          "Aug",
          "Sep",
          "Oct",
          "Nov",
          "Dec",
        ].map((x, index) => ({ x, y: Math.round(820 * 1.32 ** index) })),
      },
    ],
  },
};

const architecture: DiagramView = {
  kind: "diagram",
  id: "architecture",
  title: "Reference architecture",
  diagram: {
    kind: "diagram",
    nodes: [
      {
        id: "user",
        name: "Browser client",
        kind: "client",
        note: "Web UI, no provider secrets",
        layer: "client",
      },
      {
        id: "edge",
        name: "Cloudflare CDN",
        kind: "edge",
        vendor: "cloudflare.com",
        note: "TLS, caching, bot and rate controls",
        layer: "delivery",
      },
      {
        id: "web",
        name: "Next.js app",
        kind: "client",
        vendor: "vercel.com",
        note: "Server-rendered UI on Vercel",
        layer: "delivery",
      },
      {
        id: "api",
        name: "API service",
        kind: "gateway",
        note: "Tenant checks and quotas",
        layer: "app",
      },
      {
        id: "auth",
        name: "Amazon Cognito",
        kind: "service",
        vendor: "aws.amazon.com",
        note: "OIDC identity per tenant",
        layer: "app",
      },
      {
        id: "queue",
        name: "Amazon SQS",
        kind: "queue",
        vendor: "aws.amazon.com",
        note: "Jobs with a dead-letter queue",
        layer: "async",
      },
      {
        id: "worker",
        name: "AI workers",
        kind: "worker",
        note: "Retries, provider adapter",
        layer: "async",
      },
      {
        id: "model",
        name: "OpenAI API",
        kind: "model",
        vendor: "openai.com",
        note: "Model per task and budget",
        layer: "async",
      },
      {
        id: "db",
        name: "PostgreSQL on RDS",
        kind: "store",
        vendor: "aws.amazon.com",
        note: "Tenant data, jobs, usage",
        layer: "data",
      },
      {
        id: "obs",
        name: "OpenTelemetry",
        kind: "service",
        note: "Logs, traces, alerts",
        layer: "data",
      },
    ],
    edges: [
      { from: "user", to: "edge", label: "HTTPS" },
      { from: "edge", to: "web", label: "serve app" },
      { from: "web", to: "api", label: "signed requests" },
      { from: "api", to: "auth", label: "tenant claims" },
      { from: "api", to: "db", label: "reads, writes" },
      { from: "api", to: "queue", label: "enqueue job" },
      { from: "queue", to: "worker", label: "consume" },
      { from: "worker", to: "model", label: "inference" },
      { from: "worker", to: "db", label: "store result" },
      { from: "api", to: "obs", label: "telemetry" },
      { from: "worker", to: "obs", label: "telemetry" },
    ],
    layers: [
      { id: "client", name: "Client" },
      { id: "delivery", name: "Delivery" },
      { id: "app", name: "Application" },
      { id: "async", name: "Async AI" },
      { id: "data", name: "Data and operations" },
    ],
  },
};

const excerpt: CodeView = {
  kind: "code",
  id: "excerpt",
  title: "Where the invite expires",
  language: "rust",
  path: "crates/invites/src/token.rs",
  start: 41,
  text: `pub fn verify(token: &str, now: OffsetDateTime) -> Result<Invite, InviteError> {
    let claims = decode(token)?;
    if claims.expires_at < now {
        return Err(InviteError::Expired);
    }
    Ok(Invite::from(claims))
}`,
  notes: [{ from: 3, to: 5, text: "Compares against the server clock, not the invite's timezone" }],
};

const fix: DiffView = {
  kind: "diff",
  id: "fix",
  title: "Fix invite expiry across timezones",
  path: "crates/invites/src/token.rs",
  language: "rust",
  summary: "Compare expiry in UTC so invites no longer lapse early east of London.",
  hunks: [
    {
      oldStart: 41,
      newStart: 41,
      lines: [
        {
          op: "ctx",
          text: "pub fn verify(token: &str, now: OffsetDateTime) -> Result<Invite, InviteError> {",
        },
        { op: "ctx", text: "    let claims = decode(token)?;" },
        { op: "del", text: "    if claims.expires_at < now {" },
        {
          op: "add",
          text: "    if claims.expires_at.to_offset(UtcOffset::UTC) < now.to_offset(UtcOffset::UTC) {",
        },
        { op: "ctx", text: "        return Err(InviteError::Expired);" },
        { op: "ctx", text: "    }" },
      ],
    },
    {
      oldStart: 88,
      newStart: 88,
      lines: [
        { op: "ctx", text: "#[test]" },
        { op: "del", text: "fn expires_after_a_week() {" },
        { op: "add", text: "fn expires_after_a_week_in_any_timezone() {" },
        { op: "add", text: "    let warsaw = UtcOffset::from_hms(2, 0, 0).unwrap();" },
        { op: "ctx", text: "    let issued = datetime!(2026-01-04 23:30 UTC);" },
      ],
    },
  ],
};

const guide: DocumentObjectView = {
  kind: "document",
  id: "guide",
  title: "Arriving in San Francisco",
  content: {
    kind: "document",
    paragraphs: [
      "Getting in from SFO",
      "BART runs from the international terminal to Civic Center in about 30 minutes for roughly $10. A ride-share to Dogpatch costs $45–70 and takes as long in traffic.",
      "Your first week",
      "Open a US bank account with Mercury on Tuesday, pick up an eSIM before you land, and keep your passport with you on Monday: YC checks ID at the kickoff.",
    ],
    formatted: {
      version: 1,
      document: {
        type: "doc",
        content: [
          {
            type: "heading",
            attrs: { level: 2 },
            content: [{ type: "text", text: "Getting in from SFO" }],
          },
          {
            type: "paragraph",
            content: [
              {
                type: "text",
                text: "BART runs from the international terminal to Civic Center in about 30 minutes for roughly ",
              },
              { type: "text", text: "$10", marks: [{ type: "bold" }] },
              {
                type: "text",
                text: ". A ride-share to Dogpatch costs $45–70 and takes as long in traffic.",
              },
            ],
          },
          {
            type: "heading",
            attrs: { level: 2 },
            content: [{ type: "text", text: "Your first week" }],
          },
          {
            type: "bulletList",
            content: [
              {
                type: "listItem",
                content: [
                  {
                    type: "paragraph",
                    content: [
                      { type: "text", text: "Open a US bank account with Mercury on Tuesday." },
                    ],
                  },
                ],
              },
              {
                type: "listItem",
                content: [
                  {
                    type: "paragraph",
                    content: [{ type: "text", text: "Pick up an eSIM before you land." }],
                  },
                ],
              },
              {
                type: "listItem",
                content: [
                  {
                    type: "paragraph",
                    content: [
                      {
                        type: "text",
                        text: "Keep your passport with you on Monday: YC checks ID at the kickoff.",
                      },
                    ],
                  },
                ],
              },
            ],
          },
        ],
      },
    },
  },
};

const drafts: DraftView[] = [
  {
    kind: "draft",
    id: "draft-slack",
    destination: "slack",
    to: "#pricing",
    body: "Hey Maya, here's the latest pricing deck: the **Team** tier now starts at $24 a seat, and annual plans get two months free. Happy to join the Acme call if their CFO wants the numbers walked through.",
    author: { name: "Alex Novak" },
    send: "draft",
  },
  {
    kind: "draft",
    id: "draft-email",
    destination: "email",
    to: "sarah.lin@accel.com",
    subject: "Dinner tonight at Nopa",
    body: "Hi Sarah,\n\nYes, 7:30 at Nopa still works. The batch session ends at 6, so I'll come straight from Mission Bay.\n\nSee you there,\nAlex",
    author: { name: "Alex Novak" },
    send: "draft",
  },
  {
    kind: "draft",
    id: "draft-linkedin",
    destination: "linkedin",
    body: "Excited to share that we've joined Y Combinator's winter batch. 🚀\n\nWe're building the browser where an agent does the work with you, on a canvas you can watch and steer. If you're in SF this spring, come say hi.",
    author: { name: "Alex Novak", handle: "Founder at Quasar" },
    send: "draft",
  },
  {
    kind: "draft",
    id: "draft-github",
    destination: "github",
    to: "acme/app#123",
    body: "Fixed in #486: expiry is now compared in UTC, so invites no longer lapse early for teams east of London. Added `expires_after_a_week_in_any_timezone` to cover it.",
    author: { name: "alex-novak" },
    send: "draft",
  },
];

const photo: MediaView = {
  kind: "media",
  id: "photo",
  media: "image",
  title: "Tofu and egg rice bowl",
  url: "https://www.justonecookbook.com/simmered-tofu-and-egg-rice-bowl/",
  picture: picture("39d6ebc6c7.jpg", 1600, 2400),
};

const lecture: MediaView = {
  kind: "media",
  id: "lecture",
  media: "video",
  title: "Compilers: lexical analysis",
  url: "https://www.youtube.com/watch?v=FgzyLoSkL5k",
  provider: "youtube",
  poster: picture("479469f204.jpg", 480, 360),
  duration: "17:42",
};

const memo: MediaView = {
  kind: "media",
  id: "memo",
  media: "audio",
  title: "Voice memo · pricing thoughts",
  url: "file:///Users/alex/Voice%20Memos/pricing.m4a",
  provider: "file",
  duration: "2:14",
};

const docsPage: PageObjectView = {
  kind: "page",
  id: "docs-page",
  url: "https://developers.cloudflare.com/r2/",
  title: "Cloudflare R2 · Object storage without egress fees",
  frame: `${LOOK}/fb56387659.png`,
};

const failedPage: PageObjectView = {
  kind: "page",
  id: "failed-page",
  url: "https://www.ycombinator.com/people",
  title: "People at Y Combinator",
};

const note: NoteView = {
  kind: "note",
  id: "note",
  markdown:
    "# Questions for the host\n\nIs the desk big enough for **two monitors**? Ask about parking for a rental car in March.",
};

const files: FileView[] = [
  {
    kind: "file",
    id: "file-image",
    name: "rice-bowl.jpg",
    file: "image",
    kindLabel: "JPEG image",
    size: 412_000,
    picture: picture("39d6ebc6c7.jpg", 1600, 2400),
  },
  {
    kind: "file",
    id: "file-text",
    name: "pricing-notes.md",
    file: "text",
    kindLabel: "Markdown",
    size: 3_200,
    lines:
      "# Pricing\n\nTeam: $24 a seat, billed yearly.\nStarter stays free up to 3 people.\n\n## Open questions\n- Usage-based model calls?\n- Education discount",
  },
  {
    kind: "file",
    id: "file-code",
    name: "token.rs",
    file: "code",
    language: "rust",
    kindLabel: "Rust source",
    size: 5_400,
    lines: excerpt.text,
  },
  {
    kind: "file",
    id: "file-other",
    name: "Q3 board update.key",
    file: "other",
    kindLabel: "Keynote presentation",
    size: 18_400_000,
  },
];

const folder: FolderView = {
  kind: "folder",
  id: "folder",
  name: "YC application",
  path: "~/Documents/YC application",
  count: 14,
  entries: [
    { name: "Pitch deck.pdf", folder: false },
    { name: "Founder video.mov", folder: false },
    { name: "Financials", folder: true },
    { name: "rice-bowl.jpg", folder: false, picture: picture("39d6ebc6c7.jpg", 1600, 2400) },
  ],
};

const project: ProjectView = {
  kind: "project",
  id: "project",
  name: "Lunios",
  summary:
    "An AI writing studio: a SvelteKit web app over a Rust API, packaged for the desktop with Tauri.",
  root: "/Users/ada/Dev/Lunios",
  stack: [
    { name: "SvelteKit", host: "svelte.dev", version: "2.20", role: "Frontend" },
    { name: "TypeScript", host: "typescriptlang.org", version: "5.8" },
    { name: "Rust", host: "rust-lang.org", version: "1.89" },
    { name: "Tauri", host: "tauri.app", version: "2.4" },
    { name: "PostgreSQL", host: "postgresql.org" },
    { name: "Vitest" },
  ],
  tree: [
    {
      name: "apps",
      folder: true,
      children: [
        { name: "web", folder: true, more: 42 },
        { name: "desktop", folder: true, more: 9 },
      ],
    },
    {
      name: "crates",
      folder: true,
      children: [
        { name: "api", folder: true, more: 18 },
        { name: "editor-core", folder: true, more: 11 },
        { name: "sync", folder: true, more: 7 },
      ],
      more: 2,
    },
    { name: "docs", folder: true, more: 6 },
    { name: "Cargo.toml", folder: false },
    { name: "package.json", folder: false },
    { name: "pnpm-workspace.yaml", folder: false },
    { name: "README.md", folder: false },
  ],
  more: 4,
  scripts: [
    { name: "dev", command: "vite dev --port 5173", source: "package.json" },
    { name: "build", command: "vite build && tauri build", source: "package.json" },
    { name: "test", command: "vitest run && cargo test --workspace", source: "package.json" },
    { name: "lint", command: "cargo clippy --all-targets -- -D warnings", source: "Makefile" },
  ],
  git: { branch: "editor-sync", changed: 3, ahead: 2 },
};

const pullRequest: SubjectView = {
  id: "obj-subject",
  kind: "subject",
  noun: "Pull request",
  reference: "crynta/terax#1337",
  heading: "feat(explorer): support pasting files from OS clipboard",
  status: { label: "Open", tone: "active" },
  people: [{ role: "Author", name: "drunkardleo-10" }],
  facts: [
    { label: "Changed files", value: "6" },
    { label: "Checks", value: "Passing" },
    { label: "Review", value: "Changes requested" },
  ],
  labels: ["enhancement", "area: explorer"],
  summary: "Adds pasting files from the **OS clipboard** into the explorer with `Cmd+V`.",
  url: "https://github.com/crynta/terax/pull/1337",
};

const studyMonth: PlanView = {
  id: "obj-plan-study",
  kind: "plan",
  title: "CS exam study plan",
  checkable: true,
  steps: [
    {
      when: "Fri 9 Oct",
      date: "2026-10-09",
      title: "Set up and diagnose",
      kind: "task",
      done: true,
    },
    {
      when: "Sat 10 Oct",
      date: "2026-10-10",
      title: "Programming foundations",
      kind: "task",
      done: true,
    },
    { when: "Sun 11 Oct", date: "2026-10-11", title: "Control flow and functions", kind: "task" },
    { when: "Mon 12 Oct", date: "2026-10-12", title: "Sequences and strings", kind: "task" },
    { when: "Tue 13 Oct", date: "2026-10-13", title: "Recursion", kind: "task" },
    { when: "Wed 14 Oct", date: "2026-10-14", title: "Core data structures", kind: "task" },
    { when: "Thu 15 Oct", date: "2026-10-15", title: "Hash tables", kind: "task" },
    { when: "Fri 16 Oct", date: "2026-10-16", title: "Trees and heaps", kind: "task" },
    { when: "Sat 17 Oct", date: "2026-10-17", title: "Sorting", kind: "task" },
    { when: "Sun 18 Oct", date: "2026-10-18", title: "Searching", kind: "task" },
    { when: "Mon 19 Oct", date: "2026-10-19", title: "Big-O analysis", kind: "task" },
    { when: "Tue 20 Oct", date: "2026-10-20", title: "Recurrences", kind: "task" },
    { when: "Tue 20 Oct", date: "2026-10-20", title: "Office hours with the TA", kind: "event" },
    { when: "Wed 21 Oct", date: "2026-10-21", title: "Divide and conquer", kind: "task" },
    { when: "Thu 22 Oct", date: "2026-10-22", title: "Greedy algorithms", kind: "task" },
    { when: "Fri 23 Oct", date: "2026-10-23", title: "Dynamic programming", kind: "task" },
    { when: "Sat 24 Oct", date: "2026-10-24", title: "Graphs", kind: "task" },
    { when: "Sun 25 Oct", date: "2026-10-25", title: "Graph algorithms", kind: "task" },
    { when: "Mon 26 Oct", date: "2026-10-26", title: "Databases and SQL", kind: "task" },
    { when: "Tue 27 Oct", date: "2026-10-27", title: "Computer systems", kind: "task" },
    { when: "Wed 28 Oct", date: "2026-10-28", title: "Networking", kind: "task" },
    { when: "Thu 29 Oct", date: "2026-10-29", title: "Operating systems", kind: "task" },
    { when: "Fri 30 Oct", date: "2026-10-30", title: "Security basics", kind: "task" },
    { when: "Sat 31 Oct", date: "2026-10-31", title: "Software engineering", kind: "task" },
    { when: "Sun 1 Nov", date: "2026-11-01", title: "Mock exam 1", kind: "milestone" },
    { when: "Mon 2 Nov", date: "2026-11-02", title: "Review mistakes", kind: "task" },
    { when: "Tue 3 Nov", date: "2026-11-03", title: "Weak topics", kind: "task" },
    { when: "Wed 4 Nov", date: "2026-11-04", title: "Mixed practice", kind: "task" },
    { when: "Thu 5 Nov", date: "2026-11-05", title: "Mock exam 2", kind: "milestone" },
    { when: "Fri 6 Nov", date: "2026-11-06", title: "Final review", kind: "task" },
    { when: "Sat 7 Nov", date: "2026-11-07", title: "Light review", kind: "task" },
    { when: "Sun 8 Nov", date: "2026-11-08", title: "Exam day", kind: "milestone" },
  ],
};

const practice: ListView = {
  id: "obj-list-questions",
  kind: "list",
  style: "questions",
  title: "Practice questions",
  items: [
    {
      title: "What is the time complexity of two nested loops that each run n times?",
      detail: "O(n²), because the inner loop runs n times for each of n outer iterations.",
    },
    {
      title: "When is binary search valid, and what is its time complexity?",
      detail:
        "The search space must be ordered and support midpoint access; it takes O(log n) comparisons.",
    },
    {
      title: "How does a hash table handle two keys with the same hash index?",
      detail: "Chaining or open addressing; performance depends on load and probing.",
    },
    {
      title: "What is the difference between a stack and a queue?",
      detail: "A stack is LIFO; a queue is FIFO.",
    },
    {
      title: "What invariant does a binary search tree maintain?",
      detail: "Each node’s left-subtree keys are smaller and right-subtree keys are larger.",
    },
    {
      title: "How do BFS and DFS differ, and when does BFS find shortest paths?",
      detail:
        "BFS explores by distance layers and finds shortest paths in an unweighted graph; DFS follows branches deeply first.",
    },
  ],
};

const roadmap: PlanView = {
  kind: "plan",
  id: "roadmap",
  title: "Learn compilers",
  checkable: false,
  steps: [
    {
      when: "Jan 3–Feb 1, 2027",
      title: "Lexing and parsing",
      detail: "Crafting Interpreters, part one, and the Stanford CS143 lectures.",
      kind: "task",
    },
    { when: "Feb 2–Mar 1, 2027", title: "Semantic analysis and types", kind: "task" },
    {
      when: "Mar 2–Apr 1, 2027",
      title: "Code generation",
      detail: "Write a backend that emits LLVM IR.",
      kind: "task",
    },
    { when: "Apr 1, 2027", title: "A small language of your own", kind: "milestone" },
  ],
};

/** Every object, in the order the look test lays them out. */
export const allObjects: readonly ObjectView[] = [
  replySmall,
  replyFigures,
  stays,
  flights,
  products,
  videos,
  trip,
  today,
  providers,
  costs,
  growth,
  architecture,
  excerpt,
  fix,
  guide,
  ...drafts,
  photo,
  lecture,
  memo,
  docsPage,
  failedPage,
  note,
  ...files,
  folder,
  project,
  pullRequest,
];

const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun"];
const plot = (id: string, title: string, spec: PlotView["spec"]): PlotView => ({
  kind: "plot",
  id,
  title,
  spec,
});
const money = { format: "money", currency: "USD" } as const;
const quotes = [
  { x: "Hetzner", y: 64 },
  { x: "Cloudflare", y: 190 },
  { x: "Vercel", y: 420 },
  { x: "AWS", y: 780 },
];
/** One chart per plot style, with the data each style is for. */
const plots: PlotView[] = [
  plot("plot-bar", "Hosting per month", {
    kind: "bars",
    y: { label: "Cost", ...money },
    headline: { label: "Hetzner is the cheapest", value: "$64 / mo" },
    basis: "Smallest production tier with 4 vCPU, list prices on 28 Sep.",
    series: [{ name: "Cost", points: quotes }],
  }),
  plot("plot-bar-horizontal", "Time to first byte", {
    kind: "bars",
    horizontal: true,
    y: { label: "Latency", unit: "ms" },
    headline: { label: "Median from Warsaw", value: "38 ms" },
    series: [
      {
        name: "Latency",
        points: [
          { x: "Cloudflare Workers", y: 38 },
          { x: "Vercel Edge", y: 52 },
          { x: "AWS Lambda eu-central-1", y: 91 },
          { x: "Hetzner Falkenstein", y: 44 },
        ],
      },
    ],
  }),
  plot("plot-bar-stacked", "Monthly bill by service", {
    kind: "stacked",
    y: { label: "Cost", ...money },
    headline: { label: "June total", value: "$2,310" },
    series: [
      { name: "Model API", points: months.map((x, i) => ({ x, y: 600 + i * 150 })) },
      { name: "Database", points: months.map((x, i) => ({ x, y: 320 + i * 20 })) },
      { name: "Hosting", points: months.map((x, i) => ({ x, y: 180 + i * 16 })) },
    ],
  }),
  plot("plot-bar-grouped", "Signups by channel", {
    kind: "bars",
    y: { label: "Signups" },
    headline: { label: "Organic grew fastest", value: "+64%" },
    series: [
      { name: "Organic", points: months.map((x, i) => ({ x, y: 120 + i * 26 })) },
      { name: "Paid", points: months.map((x, i) => ({ x, y: 160 + (i % 3) * 18 })) },
    ],
  }),
  plot("plot-line", "Weekly active teams", {
    kind: "line",
    y: { label: "Teams" },
    headline: { label: "Up from 212 in January", value: "486" },
    series: [
      {
        name: "Teams",
        points: [212, 240, 268, 310, 402, 486].map((y, i) => ({ x: months[i]!, y })),
      },
    ],
  }),
  plot("plot-area", "Model spend as usage grows", {
    kind: "area",
    y: { label: "Spend", ...money },
    headline: { label: "By June", value: "$3,310" },
    basis: "At $3 per million input tokens and 40% cache hits.",
    series: [
      { name: "Model API", points: months.map((x, i) => ({ x, y: Math.round(820 * 1.32 ** i) })) },
    ],
  }),
  plot("plot-area-stacked", "Requests by region", {
    kind: "area",
    stack: true,
    y: { label: "Requests", unit: "M" },
    headline: { label: "Requests in June", value: "18.2M" },
    series: [
      { name: "Europe", points: months.map((x, i) => ({ x, y: 4 + i * 1.1 })) },
      { name: "North America", points: months.map((x, i) => ({ x, y: 3 + i * 0.9 })) },
      { name: "Asia", points: months.map((x, i) => ({ x, y: 1 + i * 0.5 })) },
    ],
  }),
  costs,
  plot("plot-radial", "Launch readiness", {
    kind: "radial",
    y: { format: "percent" },
    headline: { label: "Ready overall", value: "72%" },
    series: [
      {
        name: "Done",
        points: [
          { x: "Security review", y: 90 },
          { x: "Billing", y: 75 },
          { x: "Docs", y: 60 },
          { x: "Load tests", y: 40 },
        ],
      },
    ],
  }),
  plot("plot-radar", "How the providers compare", {
    kind: "radar",
    y: { label: "Score", max: 5 },
    series: [
      {
        name: "AWS",
        points: [
          { x: "Managed services", y: 5 },
          { x: "Price", y: 2 },
          { x: "Simplicity", y: 2 },
          { x: "Edge", y: 3 },
          { x: "Portability", y: 3 },
        ],
      },
      {
        name: "Cloudflare",
        points: [
          { x: "Managed services", y: 3 },
          { x: "Price", y: 4 },
          { x: "Simplicity", y: 4 },
          { x: "Edge", y: 5 },
          { x: "Portability", y: 2 },
        ],
      },
    ],
  }),
  plot("plot-range", "Rent near YC for a month", {
    kind: "range",
    y: { label: "Rent", ...money },
    headline: { label: "A furnished studio", value: "$3.2–4.8K" },
    series: [
      {
        name: "Rent",
        points: [
          { x: "Dogpatch", y: 3400, y2: 6900 },
          { x: "Mission Bay", y: 3200, y2: 4800 },
          { x: "SoMa", y: 2600, y2: 4400 },
          { x: "Mission", y: 2200, y2: 3900 },
        ],
      },
    ],
  }),
];

/** A legacy table as the adapter carries it: sentences in cells, read whole in the centre. */
const stack: SheetView = {
  kind: "sheet",
  id: "stack",
  title: "Recommended technology stack",
  columns: [
    { label: "Layer", kind: "text" },
    { label: "Technology", kind: "entity" },
    { label: "Why", kind: "text" },
  ],
  rows: [
    ["Web client", "Next.js", "Fast iteration, typed UI and server-rendered pages"],
    ["Edge", "Cloudflare", "TLS, caching, request filtering and edge protection"],
    ["Identity", "Amazon Cognito", "Managed identity with standard federation"],
    ["Primary data", "PostgreSQL on RDS", "Transactions, backups and room for pgvector"],
    ["Async work", "Amazon SQS", "Slow model calls leave the request path"],
    ["AI integration", "OpenAI API", "One adapter for models, timeouts and usage"],
    ["Observability", "OpenTelemetry", "Vendor-neutral traces, metrics and alerts"],
    ["Infrastructure", "Terraform", "Repeatable, reviewed environments"],
    ["Delivery", "GitHub Actions", "Checks, staged releases and a rollback path"],
    ["Secrets", "AWS KMS", "Managed keys and secret rotation"],
  ].map((cells) => ({ cells })),
};

const sets: SheetView = {
  kind: "sheet",
  id: "sets",
  title: "LEGO Architecture sets side by side",
  columns: [
    { label: "Set", kind: "entity" },
    { label: "Price", kind: "money", currency: "USD", best: "min" },
    { label: "Pieces", kind: "number", best: "max" },
    { label: "Per piece", kind: "money", currency: "USD", best: "min" },
    { label: "In stock", kind: "yes_no" },
    { label: "Rating", kind: "rating", best: "max" },
    { label: "Page", kind: "link" },
  ],
  rows: [
    {
      cells: [
        "Neuschwanstein Castle",
        "279.99",
        "3455",
        "0.08",
        "yes",
        "4.8/5",
        "https://www.lego.com/en-us/product/neuschwanstein-castle-21063",
      ],
      entity: { picture: picture("c89b72e433.jpg", 1500, 1132) },
    },
    {
      cells: [
        "Sagrada Família",
        "799.99",
        "12060",
        "0.07",
        "partial",
        "4.9/5",
        "https://www.lego.com/en-us/product/sagrada-familia-21065",
      ],
      entity: { picture: picture("b7b16b22a0.png", 703, 800) },
    },
    {
      cells: [
        "London",
        "39.99",
        "468",
        "0.09",
        "yes",
        "4.6/5",
        "https://www.lego.com/en-us/product/london-21034",
      ],
      entity: { picture: picture("816667de11.jpg", 800, 800) },
    },
  ],
};

/** What opens in the centre, each at the width the lift gives it. */
export const centred: { object: ObjectView; width: number }[] = [
  stays,
  providers,
  trip,
  fix,
  excerpt,
  guide,
  drafts[1]!,
  growth,
  replyFigures,
].map((object) => ({ object, width: centreSize(object).width - 80 }));

/** What the look test draws: one sheet per group, each object at its width. */
/** The trip run's five Airbnb picks as the lead published them: prices with a "was" fact. */
const airbnb = (
  name: string,
  subtitle: string,
  photo: string,
  price: number,
  was: string,
  facts: PickView["facts"],
  why: string,
  rating?: PickView["rating"],
): PickView => ({
  name,
  subtitle,
  picture: picture(photo, 720, 540),
  logo: "airbnb.com",
  url: "https://www.airbnb.com/rooms/929707397495574143",
  price: {
    display: `$${price.toLocaleString("en-US")} monthly`,
    amount: price,
    currency: "USD",
  },
  facts: [{ label: "Original monthly", value: was, kind: "text" }, ...facts],
  ...(rating ? { rating } : {}),
  why,
  tags: [],
  recommended: false,
});
const tripStays: PicksView = {
  kind: "picks",
  id: "trip-stays",
  title: "Airbnb options for provisional YC stay",
  facet: "stay",
  items: [
    airbnb(
      "Cozy Large Central Queen Bedroom in SoMa",
      "Private room in South of Market",
      "42dec9cc26.webp",
      2175,
      "$2,632",
      [
        { label: "Privacy", value: "Private room", kind: "text" },
        { label: "Amenities", value: "Workspace, laundry, kitchen", kind: "text" },
      ],
      "Lowest price among listed options; SoMa location and work setup. Total checkout price not shown.",
      { value: 4.89, max: 5, count: 9 },
    ),
    airbnb(
      "Beautiful apartment walk to UCSF & Golden Gate Park",
      "Apartment in Inner Sunset",
      "831b720e54.webp",
      3558,
      "$3,672",
      [
        { label: "Privacy", value: "Apartment", kind: "text" },
        { label: "Location", value: "Inner Sunset; walk to UCSF", kind: "text" },
      ],
      "Entire apartment with strong rating; location specifically mentions walking to UCSF.",
      { value: 4.93, max: 5, count: 15 },
    ),
    airbnb(
      "Orchid Studio 429",
      "Hotel in San Francisco",
      "8340bec274.webp",
      2360,
      "$3,035",
      [],
      "Relatively affordable studio-style hotel listing with many reviews; total checkout price not shown.",
      { value: 4.86, max: 5, count: 57 },
    ),
    airbnb(
      "Casa Castro Poppy | W/D, Wkly Clean, Private Bath",
      "Private room in San Francisco",
      "9fb9978c4c.webp",
      2329,
      "$2,670",
      [
        { label: "Privacy", value: "Private room, private bath", kind: "text" },
        { label: "Amenities", value: "Weekly cleaning, washer/dryer", kind: "text" },
      ],
      "Private bath plus weekly cleaning at a lower monthly rate; verify location and availability.",
    ),
    airbnb(
      "Sunny Telegraph Hill bay view condo w parking, gym",
      "Condo in San Francisco",
      "de811bb61d.webp",
      5140,
      "$6,240",
      [{ label: "Privacy", value: "Entire condo", kind: "text" }],
      "Whole condo with a bay view and parking; the most expensive of the five.",
    ),
  ],
};

/** The provider compare run's picks and capability sheet, as published. */
const providerPicks: PicksView = {
  kind: "picks",
  id: "provider-picks",
  title: "Provider choices for Lunios AI SaaS",
  facet: "service",
  items: [
    {
      name: "AWS",
      subtitle: "Core production platform for API, workers, data",
      logo: "aws.amazon.com",
      facts: [
        { label: "PG + vectors", value: "yes", kind: "yes" },
        { label: "Durable workflows", value: "yes", kind: "yes" },
        { label: "Managed Redis", value: "yes", kind: "yes" },
        { label: "Entry pricing", value: "Usage-based", kind: "text" },
      ],
      why: "Best full-backend fit: managed primitives, regional control, and a path from ECS to RDS, SQS, S3, and observability.",
      tags: ["recommended", "backend"],
      recommended: true,
    },
    {
      name: "Cloudflare",
      subtitle: "Edge, WAF, static delivery, lightweight APIs",
      logo: "cloudflare.com",
      price: { display: "$5/mo base", amount: 5, currency: "USD" },
      facts: [
        { label: "Edge/WAF", value: "yes", kind: "yes" },
        { label: "Durable workflows", value: "yes", kind: "yes" },
        { label: "PG + vectors", value: "partial", kind: "partial" },
        { label: "Included requests", value: "10M/mo", kind: "text" },
      ],
      why: "Excellent front door and cheap edge runtime; keep PostgreSQL and heavy workers outside Cloudflare.",
      tags: ["edge", "low-cost"],
      recommended: false,
    },
    {
      name: "Vercel",
      subtitle: "Next.js web app and frontend delivery",
      logo: "vercel.com",
      price: { display: "$20/mo", amount: 20, currency: "USD" },
      facts: [
        { label: "Next.js hosting", value: "yes", kind: "yes" },
        { label: "Durable workflows", value: "no", kind: "no" },
        { label: "PG + vectors", value: "no", kind: "no" },
        { label: "Function duration", value: "bounded", kind: "text" },
      ],
      why: "Best web-surface experience, but not the system of record or long-running AI-job platform.",
      tags: ["frontend"],
      recommended: false,
    },
    {
      name: "Hetzner",
      subtitle: "Low-cost compute with a self-managed stack",
      logo: "docs.hetzner.com",
      price: { display: "€5.49/mo", amount: 5.49, currency: "EUR" },
      facts: [
        { label: "PG + vectors", value: "partial", kind: "partial" },
        { label: "Durable workflows", value: "no", kind: "no" },
        { label: "Managed Redis", value: "no", kind: "no" },
        { label: "Object storage", value: "1TB + 1TB egress", kind: "text" },
      ],
      why: "Lowest compute cost, but you own HA, backups, upgrades, queues, observability, and most operations.",
      tags: ["budget", "self-managed"],
      recommended: false,
    },
  ],
};
const providerSheet: SheetView = {
  kind: "sheet",
  id: "provider-sheet",
  title: "Provider capability comparison",
  columns: [
    { label: "Provider", kind: "entity" },
    { label: "Monthly entry", kind: "money", currency: "USD", best: "min" },
    { label: "Edge/WAF", kind: "yes_no" },
    { label: "Managed Postgres", kind: "yes_no" },
    { label: "pgvector path", kind: "yes_no" },
    { label: "Durable workflows", kind: "yes_no" },
    { label: "Managed queue", kind: "yes_no" },
    { label: "Managed Redis", kind: "yes_no" },
    { label: "Object storage", kind: "yes_no" },
    { label: "Ops burden", kind: "text" },
  ],
  rows: [
    {
      cells: ["AWS", "", "yes", "yes", "yes", "yes", "yes", "yes", "yes", "medium"],
      entity: { logo: "aws.amazon.com" },
    },
    {
      cells: [
        "Cloudflare",
        "5",
        "yes",
        "no",
        "partial",
        "yes",
        "yes",
        "no",
        "yes",
        "low at edge; external data/workers",
      ],
      entity: { logo: "cloudflare.com" },
    },
    {
      cells: ["Vercel", "20", "yes", "no", "no", "no", "no", "no", "no", "low for frontend"],
      entity: { logo: "vercel.com" },
    },
    {
      cells: ["Hetzner", "", "partial", "no", "partial", "no", "no", "no", "yes", "high"],
      entity: { logo: "hetzner.com" },
    },
  ],
  note: "Published entry or representative rates; not an identical workload estimate. Blank means no comparable figure.",
};

export const looks: Record<string, { object: ObjectView; width: number }[]> = {
  replies: [
    {
      object: {
        kind: "reply",
        id: "reply-pending",
        state: "pending",
        headline: "The answer is on its way",
        text: "",
        figures: [],
        points: [],
      },
      width: 560,
    },
    { object: replySmall, width: 560 },
    { object: replyFigures, width: 640 },
  ],
  picks: [
    { object: stays, width: 860 },
    { object: products, width: 860 },
    { object: videos, width: 860 },
  ],
  flights: [{ object: flights, width: 460 }],
  plans: [
    { object: studyMonth, width: 904 },
    { object: trip, width: 720 },
    { object: roadmap, width: 560 },
  ],
  documents: [{ object: guide, width: 520 }],
  media: [
    { object: photo, width: 360 },
    { object: lecture, width: 520 },
    { object: memo, width: 420 },
  ],
  pages: [
    { object: docsPage, width: 480 },
    { object: failedPage, width: 480 },
  ],
  notes: [{ object: note, width: 420 }],
  questions: [{ object: practice, width: 880 }],
  subjects: [
    { object: pullRequest, width: 560 },
    {
      object: {
        id: "obj-subject-issue",
        kind: "subject",
        noun: "Issue",
        reference: "#103",
        heading: "Rerun workflows like Playwright",
        status: { label: "Open", tone: "active" },
        people: [{ role: "Opened by", name: "crynta" }],
        facts: [
          { label: "Assignee", value: "None" },
          { label: "Comments", value: "0" },
        ],
        labels: ["area: work", "enhancement"],
        summary:
          "Asks for an agent that learns which REST request powers an action and reuses it instead of driving the page.",
        url: "https://github.com/zephium-browser/Zephium/issues/103",
      },
      width: 560,
    },
    {
      object: {
        id: "obj-subject-merged",
        kind: "subject",
        noun: "Pull request",
        reference: "#48",
        heading: "perf: tab sleeping tiers",
        status: { label: "Merged", tone: "done" },
        people: [
          { role: "Author", name: "crynta" },
          { role: "Merged by", name: "crynta" },
        ],
        facts: [{ label: "Changed files", value: "14" }],
        labels: [],
        url: "https://github.com/zephium-browser/Zephium/pull/48",
      },
      width: 480,
    },
  ],
  files: [...files.map((object) => ({ object, width: 280 })), { object: folder, width: 300 }],
  projects: [
    { object: project, width: 720 },
    { object: { ...project, id: "project-narrow" }, width: 480 },
  ],
  drafts: drafts.map((object) => ({ object, width: 520 })),
  code: [
    { object: excerpt, width: 640 },
    { object: fix, width: 720 },
  ],
  lists: [{ object: today, width: 620 }],
  compare: [
    { object: providerPicks, width: 1120 },
    { object: providerSheet, width: 1120 },
    { object: { ...providerSheet, id: "provider-sheet-880" }, width: 880 },
  ],
  trip: [
    { object: tripStays, width: 976 },
    { object: { ...tripStays, id: "trip-stays-1120" }, width: 1120 },
    { object: { ...tripStays, id: "trip-stays-720" }, width: 720 },
  ],
  sheets: [
    { object: providers, width: 880 },
    { object: sets, width: 880 },
    { object: stack, width: 720 },
    // Given less room than its columns want: the telling ones stay, the rest wait behind a control.
    { object: { ...providers, id: "providers-narrow" }, width: 520 },
  ],
  "charts-bars": plots.slice(0, 4).map((object) => ({ object, width: 520 })),
  "charts-trends": plots.slice(4, 7).map((object) => ({ object, width: 520 })),
  "charts-round": plots.slice(7).map((object) => ({ object, width: 520 })),
};
