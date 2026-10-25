<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import ProviderMark from "$shared/ui/ProviderMark";
  import type { Mind } from "../../lib/minds";
  let { mind = $bindable(null), onsubmit }: { mind?: Mind | null; onsubmit: () => void } = $props();
</script>

<div class="options" role="radiogroup" aria-label={m.onb_ai_options()}>
  <label class="option">
    <input
      type="radio"
      name="onboarding-model"
      value="chatgpt"
      bind:group={mind}
      onkeydown={(event) => {
        if (event.key === "Enter" && !event.isComposing) {
          event.preventDefault();
          onsubmit();
        }
      }}
    />
    <ProviderMark provider="open_ai" size={32} />
    <span class="title">ChatGPT</span>
    <span class="body">{m.chatgpt_onboarding_body()}</span>
  </label>
</div>

<style>
  .options {
    display: flex;
    justify-content: center;
  }

  .option {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 12px;
    inline-size: 280px;
    padding: 24px;
    border-radius: var(--radius-card);
    background: var(--color-card);
    box-shadow: var(--shadow-raise);
  }

  input {
    position: absolute;
    inset: 0;
    margin: 0;
    opacity: 0;
  }

  .option:has(input:checked) {
    background: var(--color-fill-active);
  }

  .option:has(input:focus-visible) {
    outline: 2px solid var(--color-ring);
    outline-offset: 2px;
  }

  .title {
    color: var(--color-text);
    font-size: var(--text-title);
    font-weight: 600;
  }

  .body {
    color: var(--color-muted);
    font-size: var(--text-body);
    line-height: 1.5;
  }
</style>
