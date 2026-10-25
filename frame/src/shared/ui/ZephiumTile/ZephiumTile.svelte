<script lang="ts">
  /** The external Zephium AI service keeps its own provider identity. */
  let { size = 16 }: { size?: number } = $props();
</script>

<img
  class="zephium-tile"
  src="/zephium-icon.png"
  srcset="/zephium-icon.png 128w, /zephium-icon@2x.png 256w"
  sizes={`${size}px`}
  width={size}
  height={size}
  alt=""
  aria-hidden="true"
  draggable="false"
/>

<style>
  .zephium-tile {
    display: inline-block;
    flex: none;
    object-fit: contain;
  }
</style>
