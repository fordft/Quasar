<script lang="ts">
  import ProviderMark from "../ProviderMark.svelte";
  import QuasarTile from "$shared/ui/QuasarTile";

  const providers = [
    "anthropic",
    "open_ai",
    "google",
    "xai",
    "mistral",
    "deep_seek",
    "groq",
    "cerebras",
    "open_router",
    "compatible",
    "cloud",
  ] as const;
</script>

<div class="sheet">
  {#each [16, 20] as size (size)}<div class="row">
      {#each providers as provider (provider)}<span class="tile" style:--tile={`${size + 12}px`}
          ><ProviderMark {provider} {size} /></span
        >{/each}
      <QuasarTile size={size + 12} />
    </div>{/each}
  <div class="row"><QuasarTile size={32} /><QuasarTile size={36} /><QuasarTile size={52} /></div>
</div>

<style>
  .sheet {
    display: grid;
    gap: 16px;
    padding: 24px;
    background: var(--color-surface);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .tile {
    display: grid;
    inline-size: var(--tile);
    block-size: var(--tile);
    border-radius: var(--radius-control);
    background: var(--color-fill);
    color: var(--color-text);
    place-items: center;
  }
</style>
