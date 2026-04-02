<script lang="ts">
  import LlmConfigurationPanel from "../components/LlmConfigurationPanel.svelte";
</script>

<section class="settings-page">
  <LlmConfigurationPanel
    mode="settings"
    eyebrow="Settings"
    title="Model provider"
    description="The desktop runtime reads provider configuration only from persisted settings. Environment-based model routing is disabled."
    submitLabel="Save changes"
  />
</section>

<style>
  .settings-page {
    padding: 28px;
    overflow: auto;
    background:
      radial-gradient(circle at top left, rgba(204, 175, 133, 0.2), transparent 30%),
      #f5f0e8;
  }
</style>
