//! Loopback replicas of the person's daily apps' main views, in the
//! structure the real apps render: Slack's virtual message list, Gmail's
//! inbox grid, Calendar's event chips, Linear's issue rows, Notion's recent
//! pages and GitHub's notifications. The view readers read them as records.

/// One app's view: its path on the loopback account site, the fact its
/// rows carry, and its page.
pub(super) struct AppView {
    pub path: &'static str,
    pub fact: &'static str,
    pub html: &'static str,
}

pub(super) const VIEWS: [AppView; 6] = [
    AppView {
        path: "/app/slack",
        fact: "review of the onboarding PR before 11",
        html: SLACK,
    },
    AppView {
        path: "/app/gmail",
        fact: "Invoice 2026-0931 is due October 7",
        html: GMAIL,
    },
    AppView {
        path: "/app/calendar",
        fact: "Design review with Ana",
        html: CALENDAR,
    },
    AppView {
        path: "/app/linear",
        fact: "Offline sync loses edits after a conflict",
        html: LINEAR,
    },
    AppView {
        path: "/app/notion",
        fact: "Launch checklist for Quasar 1.0",
        html: NOTION,
    },
    AppView {
        path: "/app/github",
        fact: "Fix the sign-in redirect loop",
        html: GITHUB,
    },
];

/// A mail app whose views are routes in the address fragment: the address
/// moves between them and no document loads.
pub(super) const ROUTED_PATH: &str = "/edit/webmail-routes";

pub(super) fn routed(path: &str) -> Option<&'static str> {
    (path == ROUTED_PATH).then_some(ROUTED)
}

pub(super) fn view(path: &str) -> Option<&'static AppView> {
    VIEWS.iter().find(|view| view.path == path)
}

const ROUTED: &str = r##"<!doctype html><html><head><meta charset="utf-8"><title>Mail</title></head><body>
<nav aria-label="Mailboxes"><a href="/edit/webmail-routes">Inbox</a> <button id="drafts">Drafts</button></nav>
<main id="view"></main>
<script>
var views={
  inbox:'<h1>Inbox</h1><div role="list" aria-label="Messages"><div role="listitem">Dana: Quarterly report</div></div>',
  drafts:'<h1>Drafts</h1><div role="list" aria-label="Messages"><div role="listitem">Draft: Launch checklist for Quasar 1.0</div></div>'
};
function show(){
  var name=location.hash.slice(1).split('?')[0]||'inbox';
  document.getElementById('view').innerHTML=views[name]||'<h1>Not found</h1>';
}
addEventListener('hashchange',show);
document.getElementById('drafts').addEventListener('click',function(){ location.hash='drafts'; });
if(!location.hash) location.hash='inbox';
show();
</script></body></html>"##;

const SLACK: &str = r##"<!doctype html><html><head><meta charset="utf-8"><title>launch (Channel) - Acme - Slack</title></head><body>
<div class="p-client_container"><div class="p-client">
<nav class="p-tab_rail" aria-label="Workspace"><button aria-label="Home">Home</button><button aria-label="Activity">Activity</button></nav>
<div class="p-channel_sidebar" role="navigation" aria-label="Channels and direct messages">
<div role="tree" aria-label="Channels and direct messages">
<div role="treeitem" aria-level="1" aria-selected="false"><span class="p-channel_sidebar__name">general</span></div>
<div role="treeitem" aria-level="1" aria-selected="true"><span class="p-channel_sidebar__name">launch</span></div>
<div role="treeitem" aria-level="1" aria-selected="false"><span class="p-channel_sidebar__name">design</span></div>
<div role="treeitem" aria-level="1" aria-selected="false"><span class="p-channel_sidebar__name">Marta Nowak</span></div>
</div></div>
<div class="p-workspace__primary_view" role="main">
<div class="p-view_header"><h1 class="p-view_header__channel_title">launch</h1></div>
<div class="c-virtual_list c-message_list" role="list" aria-label="launch (channel)">
<div role="listitem" class="c-virtual_list__item"><div class="c-message_list__day_divider" role="separator"><button class="c-message_list__day_divider__label__pill">Yesterday</button></div></div>
<div role="listitem" class="c-virtual_list__item"><div class="c-message_kit__background" data-qa="message_container"><div class="c-message_kit__gutter__right">
<span class="c-message__sender"><button class="c-message__sender_button" data-qa="message_sender_name">Tomasz Lis</button></span>
<a class="c-timestamp" href="/archives/C07LAUNCH/p1727600000" aria-label="Yesterday at 5:48:10 PM"><span class="c-timestamp__label">5:48 PM</span></a>
<div class="c-message_kit__blocks"><div class="p-rich_text_section">Release notes draft is in the doc, comments welcome</div></div></div></div></div>
<div role="listitem" class="c-virtual_list__item"><div class="c-message_list__day_divider" role="separator"><button class="c-message_list__day_divider__label__pill">Today</button></div></div>
<div role="listitem" class="c-virtual_list__item"><div class="c-message_kit__background" data-qa="message_container"><div class="c-message_kit__gutter__right">
<span class="c-message__sender"><button class="c-message__sender_button" data-qa="message_sender_name">Marta Nowak</button></span>
<a class="c-timestamp" href="/archives/C07LAUNCH/p1727680000" aria-label="Today at 9:14:02 AM"><span class="c-timestamp__label">9:14 AM</span></a>
<div class="c-message_kit__blocks"><div class="p-rich_text_section">@you can I get a review of the onboarding PR before 11? It blocks the launch build</div></div>
<div class="c-reaction_bar"><button class="c-reaction" aria-label="2 reactions, react with eyes emoji"><span>2</span></button></div></div></div></div>
<div role="listitem" class="c-virtual_list__item"><div class="c-message_kit__background" data-qa="message_container"><div class="c-message_kit__gutter__right">
<span class="c-message__sender"><button class="c-message__sender_button" data-qa="message_sender_name">Ana Kowalska</button></span>
<a class="c-timestamp" href="/archives/C07LAUNCH/p1727683000" aria-label="Today at 10:02:40 AM"><span class="c-timestamp__label">10:02 AM</span></a>
<div class="c-message_kit__blocks"><div class="p-rich_text_section">Pricing page copy is final, shipping it with the post on Thursday</div></div></div></div></div>
</div>
<div class="p-message_input" role="textbox" contenteditable="true" aria-label="Message #launch"></div>
</div></div></div></body></html>"##;

const GMAIL: &str = r##"<!doctype html><html><head><meta charset="utf-8"><title>Inbox (3) - you@acme.test - Gmail</title></head><body>
<div class="nH"><div role="navigation" aria-label="Main menu"><a href="#inbox" aria-label="Inbox 3 unread">Inbox</a><a href="#starred">Starred</a><a href="#sent">Sent</a></div>
<div role="main"><div class="aeH"><div role="button" aria-label="Refresh">Refresh</div><span class="Dj">1–3 of 3</span></div>
<table class="F cf zt" role="grid" aria-label="Inbox"><tbody>
<tr class="zA zE" role="row" tabindex="-1"><td class="oZ-x3 xY" role="gridcell"><div role="checkbox" aria-label="Select" aria-checked="false"></div></td>
<td class="yX xY"><div class="yW"><span class="bA4"><span class="yP" email="billing@hetzner.test" name="Hetzner Billing">Hetzner Billing</span></span></div></td>
<td class="xY a4W" role="link"><div class="xS"><div class="xT"><span class="bog"><span>Invoice 2026-0931 is due October 7</span></span><span class="y2"> - Your cloud invoice for September totals €84.20 and will be charged on October 7.</span></div></div></td>
<td class="xW xY"><span title="Tue, Sep 30, 2026, 8:02 AM" aria-label="8:02 AM"><span>8:02 AM</span></span></td></tr>
<tr class="zA zE" role="row" tabindex="-1"><td class="oZ-x3 xY" role="gridcell"><div role="checkbox" aria-label="Select" aria-checked="false"></div></td>
<td class="yX xY"><div class="yW"><span class="bA4"><span class="yP" email="dana@acme.test" name="Dana Wells">Dana Wells</span></span></div></td>
<td class="xY a4W" role="link"><div class="xS"><div class="xT"><span class="bog"><span>Board deck for Thursday</span></span><span class="y2"> - Can you send the metrics slide by Wednesday noon?</span></div></div></td>
<td class="xW xY"><span title="Tue, Sep 30, 2026, 7:41 AM" aria-label="7:41 AM"><span>7:41 AM</span></span></td></tr>
<tr class="zA yO" role="row" tabindex="-1"><td class="oZ-x3 xY" role="gridcell"><div role="checkbox" aria-label="Select" aria-checked="false"></div></td>
<td class="yX xY"><div class="yW"><span class="bA4"><span class="yP" email="noreply@github.test" name="GitHub">GitHub</span></span></div></td>
<td class="xY a4W" role="link"><div class="xS"><div class="xT"><span class="bog"><span>[acme/app] Release v1.4.0 published</span></span><span class="y2"> - The release includes 23 merged pull requests.</span></div></div></td>
<td class="xW xY"><span title="Mon, Sep 29, 2026, 6:10 PM" aria-label="Sep 29"><span>Sep 29</span></span></td></tr>
</tbody></table></div></div></body></html>"##;

const CALENDAR: &str = r##"<!doctype html><html><head><meta charset="utf-8"><title>Google Calendar - Tuesday, September 30, 2026</title></head><body>
<header role="banner"><div role="button" aria-label="Today">Today</div><div role="button" aria-label="Previous day"></div><div role="button" aria-label="Next day"></div><h1>September 30, 2026</h1></header>
<div role="main"><div role="grid" aria-label="Tuesday, September 30, 2026"><div role="row"><div role="columnheader">TUE 30</div></div>
<div role="row"><div role="gridcell" data-datekey="27010">
<div role="button" data-eventchip="" aria-label="9:15am to 9:30am, Standup, Marta Nowak, Accepted, Google Meet, September 30, 2026"><div aria-hidden="true"><span>Standup</span><span>9:15 – 9:30am</span></div></div>
<div role="button" data-eventchip="" aria-label="11am to 12pm, Onboarding PR review, Needs RSVP, Room Warsaw 2, September 30, 2026"><div aria-hidden="true"><span>Onboarding PR review</span><span>11am – 12pm</span></div></div>
<div role="button" data-eventchip="" aria-label="2:30pm to 3:15pm, Design review with Ana, Accepted, Figma, September 30, 2026"><div aria-hidden="true"><span>Design review with Ana</span><span>2:30 – 3:15pm</span></div></div>
</div></div></div></div></body></html>"##;

const LINEAR: &str = r##"<!doctype html><html><head><meta charset="utf-8"><title>My issues › Assigned</title></head><body>
<nav aria-label="Sidebar"><a href="/acme/my-issues/assigned">My issues</a><a href="/acme/inbox">Inbox</a><a href="/acme/team/ZEP/active">Active issues</a></nav>
<main><header><h1>My issues</h1><div role="tablist"><button role="tab" aria-selected="true">Assigned</button><button role="tab" aria-selected="false">Created</button></div></header>
<div class="ListView"><div class="ListGroupHeader"><span>In Progress</span><span>2</span></div>
<a class="ListItem" href="/acme/issue/ZEP-118/offline-sync"><span class="Priority" aria-label="Urgent priority"></span><span class="Identifier">ZEP-118</span><span class="Status" aria-label="In Progress"></span><span class="Title">Offline sync loses edits after a conflict</span><span class="Labels">Bug</span><span class="Due">Due Oct 2</span></a>
<a class="ListItem" href="/acme/issue/ZEP-121/canvas-zoom"><span class="Priority" aria-label="High priority"></span><span class="Identifier">ZEP-121</span><span class="Status" aria-label="In Progress"></span><span class="Title">Canvas zoom jumps at 50 percent</span><span class="Labels">Frame</span></a>
<div class="ListGroupHeader"><span>Todo</span><span>1</span></div>
<a class="ListItem" href="/acme/issue/ZEP-130/export-pdf"><span class="Priority" aria-label="Medium priority"></span><span class="Identifier">ZEP-130</span><span class="Status" aria-label="Todo"></span><span class="Title">Export a work as PDF with its sources</span></a>
</div></main></body></html>"##;

const NOTION: &str = r##"<!doctype html><html><head><meta charset="utf-8"><title>Home</title></head><body>
<nav class="notion-sidebar" aria-label="Sidebar"><div role="tree" aria-label="Private"><div role="treeitem">Roadmap</div><div role="treeitem">Meeting notes</div></div></nav>
<main class="notion-frame"><h1>Good morning</h1><section aria-label="Recently visited"><h2>Recently visited</h2>
<div class="notion-scroller"><div role="list">
<div role="listitem"><a href="/acme/Launch-checklist-1f2e" class="notion-home-card"><div class="card-title">Launch checklist for Quasar 1.0</div><div class="card-meta">Edited 2h ago by Marta</div></a></div>
<div role="listitem"><a href="/acme/Pricing-research-9a8b" class="notion-home-card"><div class="card-title">Pricing research notes</div><div class="card-meta">Edited yesterday by you</div></a></div>
<div role="listitem"><a href="/acme/Hiring-plan-Q4-77aa" class="notion-home-card"><div class="card-title">Hiring plan for Q4</div><div class="card-meta">Edited Sep 26 by Dana</div></a></div>
</div></div></section></main></body></html>"##;

const GITHUB: &str = r##"<!doctype html><html><head><meta charset="utf-8"><title>Notifications</title></head><body>
<header class="AppHeader" role="banner"><a href="/" aria-label="Homepage">GitHub</a><nav aria-label="Global"><a href="/pulls">Pull requests</a><a href="/issues">Issues</a></nav></header>
<main><div class="notifications-v2"><nav aria-label="Notifications filters"><a href="/notifications?query=reason%3Areview-requested">Review requested</a><a href="/notifications?query=reason%3Amention">Mentioned</a></nav>
<h1 class="sr-only">Notifications</h1>
<ul class="notifications-list" aria-label="Notifications">
<li class="notifications-list-item notification-unread"><a class="notification-list-item-link" href="/acme/app/pull/412"><span class="text-small">acme/app #412</span><p class="markdown-title">Fix the sign-in redirect loop</p><span class="text-small">review requested</span></a><relative-time datetime="2026-09-30T06:40:00Z">2 hours ago</relative-time></li>
<li class="notifications-list-item notification-unread"><a class="notification-list-item-link" href="/acme/app/issues/398"><span class="text-small">acme/app #398</span><p class="markdown-title">Crash when a work has no title</p><span class="text-small">mention</span></a><relative-time datetime="2026-09-29T15:02:00Z">yesterday</relative-time></li>
<li class="notifications-list-item"><a class="notification-list-item-link" href="/acme/site/pull/57"><span class="text-small">acme/site #57</span><p class="markdown-title">Update pricing table for EU</p><span class="text-small">author</span></a><relative-time datetime="2026-09-28T09:10:00Z">2 days ago</relative-time></li>
</ul></div></main></body></html>"##;
