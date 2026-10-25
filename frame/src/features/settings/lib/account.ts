import type { AccountFaultV1 } from "$shared/ipc/bindings";
import * as m from "$shared/i18n/messages";

export function explainAccountFault(fault: AccountFaultV1): string {
  switch (fault) {
    case "offline":
      return m.chatgpt_error_offline();
    case "denied":
      return m.chatgpt_error_denied();
    case "cancelled":
      return m.chatgpt_error_cancelled();
    case "timed_out":
      return m.chatgpt_error_timeout();
    case "busy":
      return m.chatgpt_error_busy();
    case "vault":
      return m.chatgpt_error_vault();
    case "unavailable":
      return m.chatgpt_unavailable();
    case "signed_out":
      return m.chatgpt_error_signed_out();
    case "plan_permission_required":
      return m.chatgpt_error_permission();
    case "usage_limit":
      return m.chatgpt_error_limit();
    case "rate_limited":
      return m.chatgpt_error_rate();
    case "remote_revocation_unconfirmed":
      return m.chatgpt_error_revoke();
    case "protocol":
    case "invalid_identity":
    case "invalid_callback":
      return m.chatgpt_error_identity();
  }
}
