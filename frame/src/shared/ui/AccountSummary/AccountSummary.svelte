<script lang="ts">
  import * as m from "$shared/i18n/messages";
  let {
    email,
    initials,
    sharing = false,
  }: { email: string | null; initials: string; sharing?: boolean } = $props();
</script>

<span class="account-summary">
  <span class="disc" aria-hidden="true">{initials}</span>
  <span class="who"
    ><span class="email">{email ?? "ChatGPT"}</span><span class="plan"
      >{sharing ? m.chatgpt_plan() : m.chatgpt_connected()}</span
    ></span
  >
</span>

<style>
  .account-summary {
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
    min-inline-size: 0;
    text-align: start;
  }

  .disc {
    display: grid;
    inline-size: 32px;
    block-size: 32px;
    border-radius: var(--radius-capsule);
    background: var(--color-fill-active);
    color: var(--color-text);
    font-size: var(--text-label);
    font-weight: 600;
    place-items: center;
  }

  .who {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }

  .email {
    overflow: hidden;
    color: var(--color-text);
    font-size: var(--text-body);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .plan {
    color: var(--color-muted);
    font-size: var(--text-caption);
  }
</style>
