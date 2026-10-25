import type { RuntimeSecurityUpdateTarget, RuntimeStatus } from "$shared/ipc/bindings";

export type RuntimeNotification = {
  id:
    | "restart_required"
    | "review_overdue"
    | "update_zephium"
    | "update_operating_system"
    | "update_browser_runtime"
    | "unreviewed_runtime"
    | "user_content_degraded";
  title: string;
  detail: string;
  tone: "info" | "warning";
};

function updateRecommendation(target: RuntimeSecurityUpdateTarget): RuntimeNotification {
  switch (target) {
    case "operating_system":
      return {
        id: "update_operating_system",
        title: "System update recommended",
        detail:
          "Install the latest operating-system update to receive current browser-engine security fixes.",
        tone: "warning",
      };
    case "browser_runtime":
      return {
        id: "update_browser_runtime",
        title: "Browser runtime update recommended",
        detail: "Install the latest stable browser runtime before continuing sensitive browsing.",
        tone: "warning",
      };
    case "zephium":
      return {
        id: "update_zephium",
        title: "Quasar update recommended",
        detail: "Install the latest Quasar release to refresh its reviewed browser-runtime policy.",
        tone: "warning",
      };
  }
}

export function runtimeNotifications(status: RuntimeStatus): RuntimeNotification[] {
  const notifications: RuntimeNotification[] = [];
  const ids = new Set<RuntimeNotification["id"]>();
  const push = (notification: RuntimeNotification) => {
    if (ids.has(notification.id)) return;
    ids.add(notification.id);
    notifications.push(notification);
  };

  if (status.restart_required) {
    push({
      id: "restart_required",
      title: "Restart Quasar",
      detail: "A newer browser runtime is ready. Restart Quasar to use it.",
      tone: "info",
    });
  }

  if (status.user_content_degraded_scope_count > 0) {
    const count = Math.min(Math.trunc(status.user_content_degraded_scope_count), 65);
    push({
      id: "user_content_degraded",
      title: "Some add-on changes weren't applied",
      detail:
        count === 1
          ? "An extension or userscript change couldn't be applied. Quasar kept the previous verified version when available."
          : "Some extension or userscript changes couldn't be applied. Quasar kept previous verified versions when available.",
      tone: "warning",
    });
  }

  // Rust emits a canonical closed set of at most five entries. Retain the
  // same hard UI bound and deduplicate ids defensively before keyed rendering.
  for (const advisory of status.security_advisories.slice(0, 5)) {
    switch (advisory.kind) {
      case "update_recommended":
        push(updateRecommendation(advisory.update_target));
        break;
      case "review_overdue":
        push({
          id: "review_overdue",
          title: "Security review is overdue",
          detail:
            "This build uses its last reviewed hard floor. Check for a newer Quasar release before sensitive browsing.",
          tone: "warning",
        });
        break;
      case "unreviewed_runtime":
        push({
          id: "unreviewed_runtime",
          title: "New browser runtime detected",
          detail:
            "The stable runtime passed mandatory checks, but this Quasar build has not reviewed its release line yet. Check for a newer Quasar release.",
          tone: "warning",
        });
        break;
    }
  }
  return notifications;
}
