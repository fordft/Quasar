<script lang="ts">
  import Sidebar from "../components/Sidebar.svelte";
</script>

<Sidebar>
  {#snippet settingsNavigation()}{/snippet}
  {#snippet toolPanel()}{/snippet}
  {#snippet browserBody()}
    <input aria-label="Sidebar field" />
    <button>First action</button>
    <button>Second action</button>
  {/snippet}
  {#snippet dock()}{/snippet}
</Sidebar>
<button>Outside action</button>
