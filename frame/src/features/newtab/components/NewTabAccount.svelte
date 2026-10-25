<script lang="ts">
  import { UserCircleIcon } from "@hugeicons/core-free-icons";
  import type { accountFoot } from "$domain/account";
  import * as m from "$shared/i18n/messages";
  import AccountSummary from "$shared/ui/AccountSummary";
  import Disclosure from "$shared/ui/Disclosure";
  import Icon from "$shared/ui/Icon";

  let {
    account,
    onaccount,
    signInWidth = $bindable(0),
  }: {
    account: ReturnType<typeof accountFoot>;
    onaccount: () => void;
    /** How wide the words of the signed-out button are, for the pane around them. */
    signInWidth?: number;
  } = $props();

  const tip = $derived([m.account_menu(), account.email].filter(Boolean).join(" · "));

  // A dialog in all but name: Tab goes round the actions instead of leaving
  // them behind an open panel.
  function keep(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    const items = [
      ...(event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ];
    if (items.length === 0) return;
    event.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = at + (event.shiftKey ? -1 : 1);
    items[(next + items.length) % items.length]?.focus();
  }
</script>

{#if account.signedIn}
  <Disclosure
    label={m.account_menu()}
    triggerTitle={tip}
    menu
    align="end"
    triggerClass="ntp-account"
    panelClass="ntp-account-panel"
  >
    {#snippet trigger()}<span class="disc" aria-hidden="true">{account.initials}</span>{/snippet}
    <div class="body" role="presentation" onkeydown={keep}>
      <div class="summary">
        <AccountSummary
          email={account.email}
          initials={account.initials}
          sharing={account.sharing}
        />
      </div>
      <div class="rule" role="separator"></div>

      <button type="button" role="menuitem" class="ui-menu-item row" onclick={() => onaccount()}
        ><span class="ui-menu-icon"><Icon icon={UserCircleIcon} size={16} /></span
        >{m.account_manage_open()}</button
      >
    </div>
  </Disclosure>
{:else}
  <button type="button" class="sign-in" bind:offsetWidth={signInWidth} onclick={() => onaccount()}
    >{m.settings_identity_sign_in()}</button
  >
{/if}

<style>
  /* The account as the page's other control is drawn: a round pane's worth
     of plate, the initials on it. */
  :global(.ntp-account) {
    position: relative;
    display: grid;
    inline-size: 32px;
    block-size: 32px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-capsule);
    background: transparent;
    color: var(--color-text);
    cursor: default;
    outline: none;
    place-items: center;
    transition: scale var(--motion-slow) var(--ease-spring);
  }

  :global(.ntp-account:active) {
    scale: 0.94;
    transition-duration: var(--motion-instant);
  }

  :global(button.ntp-account:focus-visible) {
    outline: none;
    box-shadow: 0 0 0 2px var(--color-ring);
  }

  .disc {
    display: grid;
    inline-size: 26px;
    block-size: 26px;
    border-radius: var(--radius-capsule);
    background: var(--color-fill-active);
    box-shadow: var(--shine-top);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.03em;
    place-items: center;
    transition: background-color var(--motion-fast) var(--ease-out);
  }

  :global(.ntp-account:hover) .disc,
  :global(.ntp-account[data-state="open"]) .disc {
    background: var(--color-fill-strong);
  }

  /* Down out of the corner, close to it and quick: the panel belongs to the
     button it opened from. Solid rather than glass: the page's name is cut
     into the ground beneath it, and words over words through a blur read as
     neither. */
  :global(.ntp-account-panel.ui-disclosure-panel) {
    top: calc(100% + 8px);
    background: var(--color-float);
    backdrop-filter: none;
    inline-size: 272px;
    max-inline-size: calc(100vw - 24px);
    min-inline-size: 0;
    translate: 0 -2px;
  }

  :global(.ntp-account-panel.ui-disclosure-panel[data-open="true"]) {
    translate: 0 0;
    transition:
      opacity var(--motion-fast) var(--ease-out),
      scale 160ms var(--ease-out),
      translate 160ms var(--ease-out),
      visibility 0s;
  }

  .summary {
    padding: 8px var(--menu-item-inset) 12px;
  }

  .rule {
    block-size: 1px;
    margin: 4px var(--menu-item-inset);
    background: var(--color-border);
  }

  .row {
    inline-size: 100%;
    min-block-size: 32px;
    padding-block: 4px;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: var(--text-body);
    text-align: start;
  }

  .row:hover,
  .row:focus-visible {
    outline: none;
    background: var(--row-active);
  }

  .row:active {
    background: var(--row-pressed);
  }

  /* Words in a capsule pane, quieter than the page's own name. */
  .sign-in {
    block-size: 32px;
    padding: 0 14px;
    border: 0;
    border-radius: var(--radius-capsule);
    background: transparent;
    color: var(--color-label-secondary);
    cursor: default;
    font: inherit;
    font-size: var(--text-body);
    font-weight: 500;
    white-space: nowrap;
    outline: none;
    transition:
      background-color var(--motion-instant) var(--ease-smooth),
      color var(--motion-instant) var(--ease-smooth),
      scale var(--motion-slow) var(--ease-spring);
  }

  .sign-in:hover {
    background: var(--color-fill-hover);
    color: var(--color-text);
  }

  .sign-in:active {
    scale: 0.96;
    transition-duration: var(--motion-instant);
  }

  .sign-in:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--color-ring);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.ntp-account),
    .sign-in {
      animation: none;
      transition: none;
    }

    :global(.ntp-account-panel.ui-disclosure-panel) {
      scale: 1;
      translate: 0 0;
    }
  }
</style>
