<script lang="ts">
  import { onMount } from "svelte";
  import { commands } from "$shared/ipc/bindings";
  import * as m from "$shared/i18n/messages";
  import type { DoubleTap, LauncherTrigger, Rejection } from "$shared/ipc/bindings";
  import { IS_MAC } from "$shared/platform";
  import { acceleratorFrom, acceleratorKeys, heldModifiers } from "$shared/lib/accelerator";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import SettingsRow from "$shared/ui/SettingsRow";
  import Select from "$shared/ui/Select";
  import Button from "$shared/ui/Button";
  import ShortcutField from "./ShortcutField.svelte";
  import { ShortcutRecorder } from "../lib/shortcut-recording.svelte";

  let trigger = $state<LauncherTrigger | null>(null);
  const recorder = new ShortcutRecorder("launcher");
  const recording = $derived(recorder.ready);
  let held = $state<string[]>([]);
  let rejection = $state<Rejection | null>(null);
  let failed = $state(false);
  let applying = $state(false);
  let checking = $state(false);
  let alive = false;
  let generation = 0;
  let reads = 0;

  const current = (started: number) => alive && generation === started;

  let keys = $derived(trigger ? acceleratorKeys(trigger.shortcut, IS_MAC) : []);
  let custom = $derived(!!trigger && trigger.shortcut !== trigger.default_shortcut);
  let waiting = $derived(!!trigger && trigger.double_tap !== "off" && !trigger.accessibility);

  const reasons: Record<Rejection, () => string> = {
    invalid: m.launcher_shortcut_invalid,
    needs_modifier: IS_MAC
      ? m.launcher_shortcut_needs_modifier
      : m.launcher_shortcut_needs_modifier_windows,
    system: m.launcher_shortcut_system,
    app_command: m.launcher_shortcut_app_command,
    unavailable: m.launcher_shortcut_unavailable,
    unsupported: m.launcher_shortcut_unsupported,
  };

  let status = $derived.by(() => {
    if (recorder.pending || checking) return m.settings_saving();
    if (rejection) return reasons[rejection]();
    if (recording) return m.launcher_shortcut_recording();
    if (failed || recorder.failed) return m.panel_action_failed();
    if (trigger && !trigger.registered) return m.launcher_shortcut_unregistered();
    return m.launcher_shortcut_ready();
  });

  async function refresh() {
    const started = generation;
    const read = ++reads;
    const next = await recorder.watch(Promise.resolve().then(() => commands.launcherTrigger()));
    if (!current(started) || read !== reads) return;
    if (next.state === "received" && next.value) trigger = next.value;
    else failed = true;
  }

  async function begin() {
    if (!trigger?.editable) return;
    generation++;
    rejection = null;
    failed = false;
    applying = false;
    checking = false;
    held = [];
    await recorder.start();
  }

  async function end(refreshTrigger = true) {
    if (!recorder.engaged && !recorder.cleanupPending) return;
    const started = ++generation;
    reads++;
    applying = false;
    checking = refreshTrigger;
    rejection = null;
    held = [];
    const confirmed = await recorder.stop();
    if (confirmed && refreshTrigger && current(started)) await refresh();
    if (current(started)) checking = false;
  }

  async function apply(accelerator: string) {
    if (applying) return;
    const listening = recording;
    const token = recorder.token;
    const started = listening ? generation : ++generation;
    applying = true;
    held = [];
    try {
      const result = await recorder.watch(
        commands.launcherSetShortcut(accelerator, listening ? token : null),
      );
      if (!current(started) || (listening ? recorder.token !== token : recorder.engaged)) return;
      const change = result.state === "received" ? result.value : null;
      if (!change) {
        failed = true;
        if (listening) await end();
        return;
      }
      trigger = change.trigger;
      rejection = change.type === "rejected" ? change.reason : null;
      if (!rejection && listening) await end();
    } catch {
      if (!current(started) || (listening && recorder.token !== token)) return;
      failed = true;
      if (listening) await end();
    } finally {
      if (current(started)) applying = false;
    }
  }

  function keydown(event: KeyboardEvent) {
    if (!recorder.engaged) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey) {
      void end();
      return;
    }
    if (!recording || applying || event.repeat) return;
    const accelerator = acceleratorFrom(event, IS_MAC);
    if (accelerator) {
      rejection = null;
      void apply(accelerator);
    } else held = heldModifiers(event, IS_MAC);
  }

  function keyup(event: KeyboardEvent) {
    if (!recorder.engaged) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    held = heldModifiers(event, IS_MAC);
  }

  async function setDoubleTap(mode: DoubleTap) {
    try {
      trigger = (await commands.launcherSetDoubleTap(mode)) ?? trigger;
    } catch {
      failed = true;
    }
  }

  onMount(() => {
    alive = true;
    void refresh();
    return () => {
      void end(false);
      alive = false;
      generation++;
    };
  });
  // Window capture precedes BrowserApp's document-level shortcut dispatcher.
  $effect(() => {
    if (!recorder.engaged) return;
    const options = { capture: true };
    window.addEventListener("keydown", keydown, options);
    window.addEventListener("keyup", keyup, options);
    window.addEventListener("blur", endOnBlur);
    return () => {
      window.removeEventListener("keydown", keydown, options);
      window.removeEventListener("keyup", keyup, options);
      window.removeEventListener("blur", endOnBlur);
    };
  });
  const endOnBlur = () => void end();
  // Permission is granted in System Settings, which tells nobody; look
  // again while this page is waiting for it, and only then.
  $effect(() => {
    if (!waiting) return;
    const poll = setInterval(() => {
      if (document.visibilityState === "visible" && !checking) void refresh();
    }, 1500);
    return () => clearInterval(poll);
  });
</script>

<SettingsGroup title={m.launcher_settings()}>
  <SettingsRow title={m.launcher_shortcut()} description={status}>
    <div class="shortcut" aria-busy={recorder.pending || checking}>
      {#if custom && !recorder.engaged && !recorder.pending && !checking}<Button
          variant="ghost"
          size="compact"
          disabled={applying}
          onclick={() => trigger && void apply(trigger.default_shortcut)}
          >{m.launcher_shortcut_reset()}</Button
        >{/if}
      <ShortcutField
        keys={recorder.pending || checking ? [] : keys}
        {held}
        {recording}
        prompt={m.launcher_shortcut_press()}
        empty={m.settings_saving()}
        warn={!!trigger && !trigger.registered}
        disabled={!trigger?.editable}
        label={recorder.pending || checking
          ? m.settings_saving()
          : recording
            ? m.launcher_shortcut_recording()
            : m.launcher_shortcut_record()}
        onclick={() => (recorder.engaged ? void end() : void begin())}
      />
    </div>
  </SettingsRow>
  {#if trigger?.double_tap_supported}
    <SettingsRow
      title={m.launcher_double_tap()}
      description={waiting ? m.launcher_double_tap_permission() : m.launcher_double_tap_desc()}
    >
      <div class="shortcut">
        {#if waiting}<Button
            variant="secondary"
            size="compact"
            onclick={() => void commands.launcherOpenAccessibility()}
            >{m.launcher_open_accessibility()}</Button
          >{/if}
        <Select
          label={m.launcher_double_tap()}
          labelHidden
          value={trigger.double_tap}
          options={[
            { value: "off", label: m.launcher_double_tap_off() },
            { value: "command", label: m.launcher_double_tap_command() },
            { value: "option", label: m.launcher_double_tap_option() },
          ]}
          onchange={(value) => void setDoubleTap(value as DoubleTap)}
        />
      </div>
    </SettingsRow>
  {/if}
</SettingsGroup>

<style>
  .shortcut {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
</style>
