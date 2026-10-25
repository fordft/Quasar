<script lang="ts">
  import { onMount } from "svelte";
  import { accountSession as session } from "$domain/account";
  import * as m from "$shared/i18n/messages";
  import Button from "$shared/ui/Button";
  import ProviderMark from "$shared/ui/ProviderMark";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import SettingsRow from "$shared/ui/SettingsRow";
  import { explainAccountFault } from "../../lib/account";
  onMount(() => {
    void session.retain();
    return () => session.release();
  });
  const account = $derived(session.account());
  const busy = $derived(session.pending());
  const fault = $derived(session.fault()?.fault ?? account?.fault);
  let usageFailed = $state(false);
</script>

<SettingsGroup title="ChatGPT" description={m.chatgpt_description()}>
  {#if session.unavailable()}<p class="note" role="status">
      {fault ? explainAccountFault(fault) : m.chatgpt_unavailable()}
    </p>
  {:else if !account}<p class="note" role="status">{m.chatgpt_loading()}</p>
  {:else}
    {#if account.signed_in}
      <SettingsRow
        title={account.name ?? account.email ?? "ChatGPT"}
        description={account.email ?? m.chatgpt_connected()}
      >
        <span class="status"
          >{account.sharing ? m.chatgpt_plan() : m.chatgpt_error_permission()}</span
        >
      </SettingsRow>
      {#if !account.sharing && !account.pending}
        <SettingsRow title={m.chatgpt_permission_title()} description={m.chatgpt_permission_desc()}>
          <Button
            disabled={!!busy}
            onclick={() => void session.signIn(account?.active, false, true)}
            >{m.chatgpt_continue()}</Button
          >
        </SettingsRow>
      {/if}
      <SettingsRow title={m.chatgpt_usage_title()} description={m.chatgpt_usage_desc()}>
        <Button
          variant="ghost"
          disabled={!!busy}
          onclick={async () => {
            usageFailed = !(await session.manageUsage());
          }}>{m.chatgpt_manage_usage()}</Button
        >
      </SettingsRow>
    {/if}
    {#if account.pending}
      <SettingsRow title={m.chatgpt_waiting()} description={m.chatgpt_waiting_desc()}>
        <Button variant="ghost" disabled={!!busy} onclick={() => void session.cancel()}
          >{m.chatgpt_cancel()}</Button
        >
      </SettingsRow>
    {:else if !account.signed_in}
      <SettingsRow title={m.chatgpt_sign_in_title()} description={m.chatgpt_sign_in_desc()}>
        <Button pending={busy === "sign_in"} disabled={!!busy} onclick={() => void session.signIn()}
          ><ProviderMark provider="open_ai" size={18} />{m.chatgpt_continue()}</Button
        >
      </SettingsRow>
    {/if}
    {#each account.profiles.filter((p) => p.signed_in && p.id !== account.active) as profile (profile.id)}
      <SettingsRow title={profile.label} description={profile.email ?? undefined}>
        <Button variant="ghost" disabled={!!busy} onclick={() => void session.select(profile.id)}
          >{m.chatgpt_switch()}</Button
        >
      </SettingsRow>
    {/each}
    {#each account.profiles.filter((p) => !p.signed_in && p.id !== account.active) as profile (profile.id)}
      <SettingsRow title={profile.label} description={profile.email ?? undefined}>
        <Button
          variant="ghost"
          disabled={!!busy || account.pending}
          onclick={() => void session.signIn(profile.id)}>{m.chatgpt_continue()}</Button
        >
      </SettingsRow>
    {/each}
    {#if account.signed_in && !account.pending}
      <SettingsRow title={m.chatgpt_sign_out_title()} description={m.chatgpt_sign_out_desc()}>
        <Button
          variant="ghost"
          pending={busy === "sign_out"}
          disabled={!!busy}
          onclick={() => void session.signOut()}>{m.chatgpt_sign_out()}</Button
        >
        <Button
          variant="ghost"
          disabled={!!busy || account.profiles.length >= 8}
          onclick={() => void session.signIn(null, true)}>{m.chatgpt_add_account()}</Button
        >
      </SettingsRow>
    {/if}
    {#if fault}<p class="note error" role="alert">{explainAccountFault(fault)}</p>{/if}
    {#if usageFailed}<p class="note error" role="alert">{m.chatgpt_error_open()}</p>{/if}
  {/if}
  {#if account?.migration_pending}
    <SettingsRow title={m.chatgpt_cleanup_title()} description={m.chatgpt_cleanup_desc()}>
      <Button disabled={!!busy} onclick={() => void session.retryCleanup()}
        >{m.chatgpt_retry()}</Button
      >
    </SettingsRow>
  {/if}
</SettingsGroup>

<style>
  .note {
    margin: 12px 16px;
    color: var(--color-muted);
    font-size: var(--text-label);
    line-height: 1.5;
  }

  .error {
    color: var(--color-danger);
  }

  .status {
    color: var(--color-muted);
    font-size: var(--text-label);
  }
</style>
