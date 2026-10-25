<script lang="ts">
  let { size = 16 }: { size?: number } = $props();
</script>

<img
  class="quasar-tile"
  src="/quasar-icon.png"
  srcset="/quasar-icon.png 128w, /quasar-icon@2x.png 256w"
  sizes={`${size}px`}
  width={size}
  height={size}
  alt=""
  aria-hidden="true"
  draggable="false"
/>

<style>
  .quasar-tile {
    display: inline-block;
    flex: none;
    object-fit: contain;
  }
</style>
