<script module lang="ts">
  /** Whether a name is Quasar itself, which has no site mark to borrow. */
  export const isQuasar = (name: string) => /^quasar\b/iu.test(name.trim());
</script>

<script lang="ts">
  import QuasarTile from "$shared/ui/QuasarTile";

  let { size = 16 }: { size?: number } = $props();
</script>

<QuasarTile {size} />
