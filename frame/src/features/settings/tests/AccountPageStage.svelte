<script lang="ts">
  import AccountPage from "../components/sections/AccountPage.svelte";
</script>

<!-- The Settings shell's own page frame around the section, as Settings.svelte draws it. -->
<section class="settings-shell" style="height: 100vh">
  <div class="settings-content scrolls">
    <div class="settings-page">
      <div class="settings-view">
        <header class="settings-page-heading">
          <h1>Account</h1>
          <p>Your optional connection to Quasar services.</p>
        </header>
        <AccountPage />
      </div>
    </div>
  </div>
</section>
