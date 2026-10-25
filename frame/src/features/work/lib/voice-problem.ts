import type { VoiceEngineV1 } from "$shared/ipc/bindings";
import type { VoiceProblem } from "$domain/voice";
import { engineName } from "$domain/voice";
import * as m from "$shared/i18n/messages";
import { IS_MAC } from "$shared/platform";

/** What the bar offers after a take fails. */
export type VoiceRemedy =
  | "settings"
  | "plans"
  | "account"
  | "limit"
  | "microphone"
  | "recognition"
  | "dictation"
  | "retry"
  | null;

export function voiceProblem(
  problem: VoiceProblem,
  engine: VoiceEngineV1 | null,
): { message: string; remedy: VoiceRemedy } {
  if (engine && engine !== "on_device")
    return { message: m.voice_problem_signed_out(), remedy: "settings" };
  const name = engineName("on_device");
  switch (problem) {
    case "microphone_denied":
      return { message: m.voice_problem_microphone(), remedy: "microphone" };
    case "recognition_denied":
      return { message: m.voice_problem_recognition(), remedy: "recognition" };
    case "dictation_off":
      return { message: m.voice_problem_dictation_off(), remedy: "dictation" };
    case "no_microphone":
      return { message: m.voice_problem_no_microphone(), remedy: "retry" };
    case "language_unavailable":
      return { message: m.voice_problem_language(), remedy: "settings" };
    case "no_engine":
      return {
        message: IS_MAC ? m.voice_problem_no_engine_mac() : m.voice_problem_no_engine(),
        remedy: "settings",
      };
    case "key_refused":
    case "signed_out":
    case "out_of_credit":
    case "plan_required":
    case "free_usage_paused":
    case "billing_review":
    case "daily_limit":
    case "hosted_paused":
      return { message: m.voice_problem_signed_out(), remedy: "settings" };
    case "invalid_audio":
      return { message: m.voice_problem_invalid_audio(), remedy: "retry" };
    case "unsupported_audio":
      return { message: m.voice_problem_unsupported_audio(), remedy: "retry" };
    case "audio_too_long":
      return { message: m.voice_problem_audio_too_long(), remedy: "retry" };
    case "rate_limited":
    case "provider_down":
      return { message: m.voice_problem_busy({ engine: name }), remedy: "retry" };
    case "offline":
      return { message: m.voice_problem_offline({ engine: name }), remedy: "retry" };
    case "too_long":
      return { message: m.voice_problem_too_long(), remedy: "retry" };
    case "nothing_heard":
      return { message: m.voice_problem_nothing(), remedy: "retry" };
    case "busy":
    case "unavailable":
      return { message: m.voice_problem_unavailable(), remedy: "retry" };
  }
}
