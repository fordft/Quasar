<script lang="ts">
  import * as m from "$shared/i18n/messages";
  import { IS_MAC } from "$shared/platform";
  import SettingsGroup from "$shared/ui/SettingsGroup";
  import PreferenceSelect from "../PreferenceSelect.svelte";
  import PreferenceSwitch from "../PreferenceSwitch.svelte";
</script>

<SettingsGroup title={m.tabs_group_opening()}
  ><PreferenceSelect id="tabs.new-position" preference="tabs.new-position" /><PreferenceSelect
    id="tabs.external"
    preference="tabs.external-position"
  /><PreferenceSwitch id="tabs.duplicates" preference="tabs.switch-to-open" /><PreferenceSwitch
    id="tabs.automatic-pip"
    preference="tabs.automatic-pip"
  /></SettingsGroup
>
{#if IS_MAC}
  <SettingsGroup title={m.tabs_group_sidebar()}
    ><PreferenceSwitch id="tabs.sidebar-autohide" preference="sidebar.autohide" /></SettingsGroup
  >
{/if}
<SettingsGroup title={m.tabs_group_closing()}
  ><PreferenceSelect id="tabs.close" preference="tabs.after-close" /><PreferenceSelect
    id="tabs.close-button"
    preference="tabs.close-button"
  /></SettingsGroup
>
<SettingsGroup title={m.tabs_group_archive()}
  ><PreferenceSelect id="tabs.archive" preference="tabs.archive" /></SettingsGroup
>
