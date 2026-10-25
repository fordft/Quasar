<script lang="ts">
  import type { WorkModelProvider } from "$shared/ipc/bindings";
  import ProviderMark from "$shared/ui/ProviderMark";
  import Button from "$shared/ui/Button";
  import Icon from "$shared/ui/Icon";
  import "$shared/ui/Shine/shine.css";
  import { UserIcon } from "../../lib/icons";
  import * as m from "$shared/i18n/messages";
  import type { Standing } from "./ModelRail.svelte";

  let {
    provider,
    name,
    standing,
    onact,
  }: {
    provider: WorkModelProvider;
    name: string;
    standing: Exclude<Standing, "ready">;
    onact: () => void;
  } = $props();
</script>

<div class="ask" data-state={standing}>
  <span class="tile ui-shine" aria-hidden="true"><ProviderMark {provider} size={26} /></span>
  <h3 class="title">{name}</h3>
  <p class="body" data-refused={standing === "refused" ? "" : undefined}>
    {m.chatgpt_sign_in_desc()}
  </p>
  <Button onclick={onact}
    ><Icon icon={UserIcon} size={14} strokeWidth={1.8} />{m.chatgpt_continue()}</Button
  >
</div>

<style>
  .ask {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 8px 28px 36px;
    text-align: center;
  }

  .tile {
    --shine-radius: var(--radius-control-large);

    display: grid;
    inline-size: 52px;
    block-size: 52px;
    place-items: center;
  }

  .title {
    margin: 4px 0 -4px;
    color: var(--color-text);
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.012em;
  }

  .body {
    max-inline-size: 240px;
    margin: 0 0 6px;
    color: var(--color-muted);
    font-size: var(--text-body);
    line-height: 19px;
    text-wrap: balance;
  }

  .body[data-refused] {
    color: var(--color-danger);
  }
</style>
