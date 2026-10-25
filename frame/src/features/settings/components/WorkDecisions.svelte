<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import type { WorkDecisionChoiceV1 } from "$shared/ipc/bindings";
  import { WorkDecisionSession } from "$domain/work-decision";
  import SegmentedControl from "$shared/ui/SegmentedControl";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import SettingsRow from "$shared/ui/SettingsRow";
  let { profile }: { profile: string } = $props();
  let session = $state.raw<WorkDecisionSession | null>(null);
  $effect(() => {
    const owner = new WorkDecisionSession(profile);
    session = owner;
    void owner.start();
    return () => owner.dispose();
  });
  const options = $derived([
    { value: "standard", label: m.settings_decisions_standard() },
    { value: "off", label: m.settings_decisions_off() },
  ]);
</script>

<SettingsGroup title={m.settings_decisions()} description={m.chatgpt_decision_note()}>
  <SettingsRow title={m.settings_decisions()}>
    <SegmentedControl
      label={m.settings_decisions()}
      {options}
      value={session?.preference?.effective ?? "standard"}
      disabled={!session?.preference || session.busy}
      onchange={(value) => void session?.choose(value as WorkDecisionChoiceV1)}
    />
  </SettingsRow>
  {#if session?.unavailable}<p class="note" role="alert">{m.ai_unavailable()}</p>{/if}
</SettingsGroup>

<style>
  .note {
    margin: 12px 16px;
    font-size: var(--text-label);
    color: var(--color-muted);
  }
</style>
