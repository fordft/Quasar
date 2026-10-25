<script lang="ts">
  import {
    Briefcase02Icon,
    CheckListIcon,
    ChipIcon,
    Clock01Icon,
    Note01Icon,
  } from "@hugeicons/core-free-icons";
  import * as m from "$shared/i18n/messages";
  import { preferences, type PreferenceKey } from "$domain/preferences";
  import { commands } from "$shared/ipc/bindings";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import Switch from "$shared/ui/Switch";
  import { IS_LINUX } from "$shared/platform";

  type Feature = "ai" | "work" | "notes" | "tasks" | "time";
  let { feature, card = false }: { feature: Feature; card?: boolean } = $props();

  const copy = {
    ai: {
      icon: ChipIcon,
      title: m.feature_ai_title,
      on: m.feature_ai_on,
      off: m.feature_ai_off,
      confirm: m.feature_ai_confirm_title,
      leaves: [
        m.feature_ai_leaves_agents,
        m.feature_ai_leaves_models,
        m.feature_ai_leaves_dictation,
        m.feature_ai_leaves_settings,
      ],
    },
    work: {
      icon: Briefcase02Icon,
      title: m.feature_work_title,
      on: m.feature_work_on,
      off: m.feature_work_off,
      confirm: m.feature_work_confirm_title,
      leaves: [m.feature_work_leaves_mode, m.feature_work_leaves_settings],
    },
    notes: {
      icon: Note01Icon,
      title: m.feature_notes_title,
      on: m.feature_notes_on,
      off: m.feature_notes_off,
    },
    tasks: {
      icon: CheckListIcon,
      title: m.feature_tasks_title,
      on: m.feature_tasks_on,
      off: m.feature_tasks_off,
    },
    time: {
      icon: Clock01Icon,
      title: m.feature_time_title,
      on: m.feature_time_on,
      off: m.feature_time_off,
    },
  };
  const item = $derived(copy[feature]);
  const unavailable = $derived(IS_LINUX && (feature === "ai" || feature === "work"));
  const key = $derived(`${feature}.enabled` as PreferenceKey);
  const on = $derived(feature === "ai" ? preferences.aiEnabled() : preferences.enabled(feature));
  // AI and Work stop what runs, so turning them off is asked first and held
  // off while it saves; a refusal brings the switch back.
  let asking = $state(false);
  let leaving = $state(false);
  // Read when the question opens, so it says whether running work stops.
  let running = $state(false);
  let dialog = $state<HTMLDialogElement>();
  const uid = $props.id();
  // The platform's own modal: the page behind goes inert, Escape cancels, and
  // focus returns to the switch when it closes.
  $effect(() => {
    if (!dialog) return;
    if (asking && !dialog.open) dialog.showModal();
    else if (!asking && dialog.open) dialog.close();
  });

  async function turnOff() {
    asking = false;
    leaving = true;
    await preferences.set(key, "false");
    leaving = false;
  }

  function change(next: boolean) {
    if (next) void preferences.set(key, "true");
    else if (!("leaves" in item)) void turnOff();
    else {
      running = false;
      asking = true;
      void Promise.resolve()
        .then(() => commands.workRunning())
        .then((now) => (running = now))
        .catch(() => {});
    }
  }
</script>

<div class={["feature", card ? "card" : "ui-settings-row"]} data-setting={key} data-on={on}>
  <span class="tile" aria-hidden="true"><Icon icon={item.icon} size={card ? 20 : 17} /></span>
  <div class="copy">
    <h3>{item.title()}</h3>
    <p>{unavailable ? m.preview_linux_notice() : on ? item.on() : item.off()}</p>
  </div>
  <Switch
    label={item.title()}
    labelHidden
    checked={on && !asking && !leaving}
    disabled={preferences.saving() || unavailable}
    onchange={change}
  />
  {#if "leaves" in item}<dialog
      bind:this={dialog}
      class="confirm settings-modal"
      aria-labelledby={`${uid}-title`}
      aria-describedby={`${uid}-body`}
      oncancel={() => (asking = false)}
      onclose={() => (asking = false)}
      onclick={(event) => {
        if (event.target === dialog) asking = false;
      }}
    >
      <h2 id={`${uid}-title`}>{item.confirm()}</h2>
      <p id={`${uid}-body`}>{m.feature_confirm_body()}</p>
      {#if running}<p class="stops" role="status">{m.ai_switch_stops_running()}</p>{/if}
      <ul class="leaves">
        {#each item.leaves as leave, index (index)}<li>{leave()}</li>{/each}
      </ul>
      <footer>
        <Button onclick={() => (asking = false)}>{m.action_cancel()}</Button>
        <Button variant="primary" onclick={() => void turnOff()}
          >{m.ai_switch_confirm_action()}</Button
        >
      </footer>
    </dialog>{/if}
</div>

<style>
  .feature {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    box-sizing: border-box;
    min-block-size: 60px;
    padding: 11px 16px 11px 14px;
  }

  /* Inset at the label, as Settings rows divide. */
  :global(.ui-settings-row) + .feature:not(.card)::before {
    content: "";
    position: absolute;
    inset-inline: 58px 0;
    top: 0;
    height: 1px;
    background: var(--color-border);
  }

  .feature.card {
    gap: 14px;
    margin-block-end: 34px;
    padding: 16px 18px 16px 16px;
    border-radius: var(--radius-card);
    background: var(--color-surface);
    box-shadow: var(--shadow-card);
  }

  .tile {
    display: grid;
    flex: none;
    inline-size: 32px;
    block-size: 32px;
    border-radius: var(--radius-control);
    background: var(--color-fill);
    color: var(--color-text);
    place-items: center;
    transition: color var(--motion-base) var(--ease-out);
  }

  .card .tile {
    inline-size: 40px;
    block-size: 40px;
  }

  .feature[data-on="false"] .tile {
    color: var(--color-muted);
  }

  .copy {
    flex: 1;
    min-width: 0;
  }

  h3 {
    margin: 0;
    color: var(--color-text);
    font-size: 13px;
    font-weight: 550;
    line-height: 18px;
    letter-spacing: -0.006em;
  }

  .card h3 {
    font-size: 15px;
    font-weight: 600;
    line-height: 20px;
    letter-spacing: -0.012em;
  }

  .copy > p {
    margin: 2px 0 0;
    color: var(--color-muted);
    font-size: var(--text-label);
    line-height: 1.5;
  }

  .confirm {
    box-sizing: border-box;
    inline-size: min(480px, calc(100vw - 48px));
    max-block-size: calc(100vh - 48px);
    margin: auto;
    padding: 26px;
    overflow-y: auto;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-card);
    color: var(--color-text);
    background: var(--color-raised);
    box-shadow: var(--shadow-overlay);
  }

  .confirm[open] {
    animation: interface-fade var(--motion-base) var(--ease-out);
  }

  .confirm h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 550;
    line-height: 1.3;
    letter-spacing: -0.02em;
  }

  .confirm > p {
    margin: 10px 0 22px;
    color: var(--color-muted);
    font-size: 12px;
    line-height: 1.6;
  }

  .confirm footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-block-start: 28px;
  }

  .stops {
    margin: -8px 0 18px;
    color: var(--color-text);
    font-size: var(--text-label);
    font-weight: 500;
    line-height: 1.5;
  }

  .leaves {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .leaves li {
    position: relative;
    padding-inline-start: 16px;
    color: var(--color-text);
    font-size: var(--text-label);
    line-height: 1.5;
  }

  .leaves li::before {
    content: "";
    position: absolute;
    inset-block-start: 0.62em;
    inset-inline-start: 2px;
    inline-size: 5px;
    block-size: 5px;
    border-radius: 50%;
    background: var(--color-faint);
  }

  @media (prefers-reduced-motion: reduce) {
    .confirm[open] {
      animation: none;
    }
  }

  @media (forced-colors: active) {
    .confirm {
      border-color: CanvasText;
    }

    /* Inset at the label, as Settings rows divide. */
    :global(.ui-settings-row) + .feature:not(.card)::before {
      content: "";
      position: absolute;
      inset-inline: 58px 0;
      top: 0;
      height: 1px;
      background: var(--color-border);
    }

    .feature.card {
      border: 1px solid ButtonText;
      box-shadow: none;
    }
  }
</style>
