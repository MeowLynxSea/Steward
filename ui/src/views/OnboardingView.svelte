<script lang="ts">
  import LlmConfigurationPanel from "../components/LlmConfigurationPanel.svelte";

  let { onComplete }: { onComplete: () => void | Promise<void> } = $props();
</script>

<LlmConfigurationPanel
  mode="onboarding"
  eyebrow="Onboarding"
  title="Connect a model provider"
  description="IronCowork stays locked until a usable provider configuration is saved. Choose a provider, add the required credentials, and confirm the model you want the desktop app to use."
  submitLabel="Continue"
  {onComplete}
/>
