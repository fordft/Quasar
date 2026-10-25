<script lang="ts">
  import { onMount } from "svelte";
  import type { VoiceEngineV1 } from "$shared/ipc/bindings";
  import { VoiceSettings, engineName, languageName } from "$domain/voice";
  import * as m from "$shared/i18n/messages";
  import { languages } from "$shared/lib/locale.svelte";
  import Select from "$shared/ui/Select";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import SettingsRow from "$shared/ui/SettingsRow";

  let { settings = new VoiceSettings() }: { settings?: VoiceSettings } = $props();

  onMount(() => {
    void settings.start();
    return () => settings.dispose();
  });

  const AUTO = "auto";
  const NONE = "none";
  const status = $derived(settings.status);
  const locale = () => document.documentElement.lang || "en";
  const engines = $derived(status?.engines ?? []);
  const engineLabel = (engine: VoiceEngineV1) =>
    engine === "open_ai" || engine === "groq"
      ? m.voice_engine_key({ provider: engineName(engine) })
      : engineName(engine);
  const engineOptions = $derived(
    engines.length
      ? engines.map((engine) => ({ value: engine, label: engineLabel(engine) }))
      : [{ value: NONE, label: m.voice_engine_none() }],
  );
  /** With nothing chosen, the first engine (this device's, where it has one) is the default. */
  const engineValue = $derived(
    status?.chosen && engines.includes(status.chosen) ? status.chosen : (engines[0] ?? NONE),
  );
  const spoken = $derived(status?.language ?? status?.languages[0] ?? null);
  const spokenName = $derived(spoken ? languageName(spoken, locale()) : "");
  const engineNote = $derived.by(() => {
    if (!status || !engines.length) return m.voice_engine_setup();
    const effective = status.effective;
    if (effective === "on_device") return m.voice_engine_private();
    const wantsDevice = engineValue === "on_device";
    if (wantsDevice && !effective) return m.voice_engine_unsupported({ language: spokenName });
    if (wantsDevice && effective)
      return m.voice_engine_fallback({ language: spokenName, engine: engineName(effective) });
    return effective ? m.voice_engine_cloud({ engine: engineName(effective) }) : "";
  });
  const base = (tag: string) => tag.split("-")[0]!.toLowerCase();
  /** The system's languages first, then every language the interface offers. */
  const languageOptions = $derived.by(() => {
    const preferred = status?.languages ?? [];
    const others = languages()
      .filter((tag) => !preferred.some((known) => base(known) === base(tag)))
      .map((tag) => ({ value: tag, label: languageName(tag, locale()) }))
      .sort((a, b) => a.label.localeCompare(b.label, locale()));
    const chosen = status?.language;
    const extra =
      chosen && !preferred.includes(chosen) && !others.some((option) => option.value === chosen)
        ? [{ value: chosen, label: languageName(chosen, locale()) }]
        : [];
    return [
      {
        value: AUTO,
        label: preferred[0]
          ? m.voice_language_auto_named({ language: languageName(preferred[0], locale()) })
          : m.voice_language_auto(),
      },
      ...preferred.map((tag) => ({ value: tag, label: languageName(tag, locale()) })),
      ...extra,
      ...others,
    ];
  });
</script>

<SettingsGroup title={m.voice_group()} description={m.chatgpt_voice_note()}>
  {#if settings.unavailable && !status}<p class="voice-note" role="alert">
      {m.voice_unavailable()}
    </p>{:else}
    <SettingsRow settingId="voice.engine" title={m.voice_engine_title()} description={engineNote}
      ><Select
        label={m.voice_engine_title()}
        labelHidden
        options={engineOptions}
        value={engineValue}
        disabled={!engines.length || settings.busy}
        onchange={(value) => void settings.choose(value === NONE ? null : (value as VoiceEngineV1))}
      /></SettingsRow
    >
    {#if status}<SettingsRow
        settingId="voice.language"
        title={m.voice_language_title()}
        description={m.voice_language_desc()}
        ><Select
          label={m.voice_language_title()}
          labelHidden
          options={languageOptions}
          value={status.language ?? AUTO}
          disabled={settings.busy}
          onchange={(value) => void settings.chooseLanguage(value === AUTO ? null : value)}
        /></SettingsRow
      >{/if}
  {/if}
</SettingsGroup>

<style>
  .voice-note {
    margin: 0;
    padding: 14px 18px;
    font-size: var(--text-label);
    line-height: 1.5;
    color: var(--color-muted);
  }
</style>
