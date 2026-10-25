<script lang="ts">
  import { onMount } from "svelte";
  import * as m from "$shared/i18n/messages";
  import { commands, type AboutInfo } from "$shared/ipc/bindings";
  import { preferences } from "$domain/preferences";
  import { runtime } from "$domain/runtime";
  import { releaseNotesUrl, systemUpdateTarget, updates } from "$domain/updates";
  import { IS_MAC } from "$shared/platform";
  import { keymap } from "$domain/keymap";
  import * as notices from "$session/notice.svelte";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import SettingsRow from "$shared/ui/SettingsRow";
  import Button from "$shared/ui/Button";
  import QuasarTile from "$shared/ui/QuasarTile";
  import AboutLinks from "../about/AboutLinks.svelte";
  import { WEBSITE } from "../../lib/links";
  import PreferenceSwitch from "../PreferenceSwitch.svelte";

  let about = $state<AboutInfo | null>(null);
  let confirming = $state(false);
  let resetting = $state(false);
  let outcome = $state<"done" | "failed" | null>(null);

  onMount(() => {
    let live = true;
    void updates.refresh();
    void commands
      .aboutInfo()
      .then((info) => {
        if (live) about = info;
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  });

  let platform = $derived(about ? m.about_platform({ os: about.os, arch: about.arch }) : "");

  let update = $derived(updates.status());
  let updatable = $derived(update.state !== "unavailable");
  let updateLine = $derived.by(() => {
    switch (update.state) {
      case "unavailable":
        return m.update_status_unavailable();
      case "idle":
        return m.pref_about_updates_help();
      case "checking":
        return m.update_status_checking();
      case "upToDate":
        return m.update_status_current();
      case "downloading":
        return m.update_status_downloading();
      case "ready":
        return update.retry_reason ?? m.update_status_ready({ version: update.version });
      case "manualInstall":
        return m.update_status_manual({ version: update.version });
      case "installing":
        return m.update_installing();
      case "failed":
        return m.update_status_failed();
    }
  });
  let security = $derived(
    updatable ? systemUpdateTarget(runtime.status().security_advisories) : null,
  );

  async function copyDetails() {
    if (!about) return;
    const details = `Quasar ${about.version}\n${about.os} (${about.arch})`;
    try {
      await navigator.clipboard.writeText(details);
      notices.show(m.about_details_copied());
    } catch {
      // A denied clipboard leaves the details on screen to read.
    }
  }

  async function reset() {
    resetting = true;
    outcome = null;
    const settled = await preferences.resetAll();
    const shortcuts = await keymap.reset(null);
    resetting = false;
    confirming = false;
    outcome = settled && shortcuts ? "done" : "failed";
  }
</script>

<header class="settings-about hero">
  <QuasarTile size={88} />
  <div class="identity">
    <span class="quasar-wordmark" role="img" aria-label="Quasar"></span>
    <p class="meta">
      <span class="version">{about ? m.about_version({ version: about.version }) : ""}</span>
      {#if updatable && about}<Button
          size="compact"
          variant="ghost"
          onclick={() =>
            void commands.browserOpenUrl(releaseNotesUrl(about!.version), true).catch(() => {})}
          >{m.update_release_notes()}</Button
        >{/if}
    </p>
    <p class="platform">{platform}</p>
    {#if about?.os.toLowerCase().includes("linux")}
      <p class="platform" role="note">{m.preview_linux_notice()}</p>
    {/if}
  </div>
  <AboutLinks />
</header>
<SettingsGroup title={m.about_group_software()}>
  <SettingsRow settingId="about.updates" title={m.pref_about_updates()} description={updateLine}>
    {#if update.state === "ready" || update.state === "installing"}
      <Button
        size="compact"
        variant="primary"
        pending={update.state === "installing" || updates.pendingRelaunch()}
        onclick={() => void updates.relaunch()}>{m.update_relaunch()}</Button
      >
    {:else if update.state === "manualInstall"}
      <Button
        size="compact"
        onclick={() => {
          if (update.state === "manualInstall")
            void commands.browserOpenUrl(releaseNotesUrl(update.version), true).catch(() => {});
        }}>{m.update_install_manual()}</Button
      >
    {:else if update.state === "failed"}
      <div class="actions">
        <Button size="compact" onclick={() => void updates.check()}>{m.update_check_now()}</Button>
        <Button
          size="compact"
          onclick={() => void commands.browserOpenUrl(WEBSITE, true).catch(() => {})}
          >{m.update_download_latest()}</Button
        >
      </div>
    {:else if updatable}
      <Button
        size="compact"
        pending={update.state === "checking" || update.state === "downloading"}
        onclick={() => void updates.check()}>{m.update_check_now()}</Button
      >
    {/if}
  </SettingsRow>
  {#if updatable}<PreferenceSwitch id="updates.auto-check" preference="updates.auto-check" />{/if}
  {#if security}
    <SettingsRow
      title={security === "browser_runtime"
        ? m.update_webview_title()
        : IS_MAC
          ? m.update_security_mac_title()
          : m.update_security_system_title()}
      description={security === "browser_runtime"
        ? m.update_webview_detail()
        : m.update_security_detail()}
    >
      {#if security === "operating_system" && IS_MAC}<Button
          size="compact"
          onclick={() => void commands.openSoftwareUpdate().catch(() => {})}
          >{m.update_security_open()}</Button
        >{/if}
    </SettingsRow>
  {/if}
  <SettingsRow title={m.pref_about_diagnostics()} description={m.about_copy_details_help()}>
    <Button size="compact" disabled={!about} onclick={() => void copyDetails()}
      >{m.about_copy_details()}</Button
    >
  </SettingsRow>
  <SettingsRow title={m.about_logs()} description={m.about_logs_help()}>
    <Button size="compact" onclick={() => void commands.diagnosticsShowLogs().catch(() => false)}
      >{m.about_logs_show()}</Button
    >
  </SettingsRow>
</SettingsGroup>
<SettingsGroup title={m.settings_advanced()}>
  <SettingsRow
    settingId="about.reset"
    title={m.pref_about_reset()}
    description={outcome === "done"
      ? m.about_reset_done()
      : outcome === "failed"
        ? m.about_reset_failed()
        : m.pref_about_reset_help()}
  >
    <div class="actions">
      {#if confirming}
        <Button
          size="compact"
          variant="ghost"
          disabled={resetting}
          onclick={() => (confirming = false)}>{m.action_cancel()}</Button
        >
        <Button size="compact" variant="danger" pending={resetting} onclick={() => void reset()}
          >{m.about_reset_confirm()}</Button
        >
      {:else}
        <Button
          size="compact"
          onclick={() => {
            outcome = null;
            confirming = true;
          }}>{m.about_reset_action()}</Button
        >
      {/if}
    </div>
  </SettingsRow>
</SettingsGroup>

<style>
  .hero {
    gap: 20px;
    padding-block: 18px 40px;
  }

  .identity {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .quasar-wordmark {
    inline-size: 150px;
    block-size: 22px;
  }

  .meta,
  .platform {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-label);
    line-height: 1.4;
  }

  .version {
    color: var(--color-label-secondary);
    font-weight: 500;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
