<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import type { WorkModelRole } from "$shared/ipc/bindings";
  import { tabs } from "$domain/tabs";
  import { ModelsSession, entryOf, pickerGroups, usable } from "$domain/ai";
  import Select from "$shared/ui/Select";
  import Button from "$shared/ui/Button";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import SettingsRow from "$shared/ui/SettingsRow";
  import ChatGPTConnection from "../account/ChatGPTConnection.svelte";
  import AiVoiceRow from "../AiVoiceRow.svelte";
  import WorkDecisions from "../WorkDecisions.svelte";
  let profile = $derived(tabs.profile());
  let session = $state.raw<ModelsSession | null>(null);
  $effect(() => {
    const id = profile && profile.kind !== "incognito" ? profile.id : null;
    if (!id) return;
    const owner = new ModelsSession(id);
    session = owner;
    void owner.start();
    return () => {
      owner.dispose();
      if (session === owner) session = null;
    };
  });
  const models = $derived(session?.models ?? null);
  const ready = $derived(usable(models, "open_ai"));
  const failed = $derived(
    !!session?.fault || !!models?.providers.find((p) => p.provider === "open_ai")?.fault,
  );
  const roles = [
    { role: "lead", title: m.ai_role_lead, description: m.ai_role_lead_desc },
    { role: "page", title: m.ai_role_page, description: m.chatgpt_models_page },
    { role: "light", title: m.ai_role_light, description: m.ai_role_light_desc },
  ] as const;
  function options(role: WorkModelRole) {
    const effective = entryOf(models, models?.effective[role as "lead" | "page" | "light"]);
    return [
      {
        value: "auto",
        label: effective
          ? m.ai_role_auto({ model: effective.display_name })
          : m.ai_role_auto_none(),
      },
      ...pickerGroups(models, role)
        .ready.flatMap((group) => group.entries)
        .map((entry) => ({ value: entry.id, label: entry.display_name })),
    ];
  }
</script>

{#if !profile || profile.kind === "incognito"}<p class="note">{m.work_regular_profile()}</p>
{:else}
  <ChatGPTConnection />
  <SettingsGroup
    title={m.ai_roles_group()}
    description={ready ? m.chatgpt_models_note() : m.chatgpt_models_empty()}
  >
    {#each roles as row (row.role)}
      <SettingsRow title={row.title()} description={row.description()}>
        <Select
          label={row.title()}
          labelHidden
          options={options(row.role)}
          value={models?.chosen[row.role] ?? "auto"}
          disabled={!ready || !!session?.busy}
          onchange={(value) => void session?.choose(row.role, value === "auto" ? null : value)}
        />
      </SettingsRow>
    {/each}
    <SettingsRow title={m.chatgpt_refresh_models()}>
      <Button
        variant="ghost"
        pending={session?.busy === "refresh"}
        disabled={!models || !!session?.busy || session?.unavailable}
        onclick={() => void session?.refresh()}>{m.chatgpt_retry()}</Button
      >
    </SettingsRow>
    {#if failed}<p class="note" role="alert">{m.chatgpt_models_failed()}</p>{/if}
  </SettingsGroup>
  <AiVoiceRow />
  <WorkDecisions profile={profile.id} />
{/if}

<style>
  .note {
    margin: 0 16px;
    font-size: var(--text-label);
    line-height: 1.5;
    color: var(--color-muted);
  }
</style>
