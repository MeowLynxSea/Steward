<script lang="ts">
  import LlmConfigurationPanel from "../components/LlmConfigurationPanel.svelte";

  let { onComplete }: { onComplete: () => void | Promise<void> } = $props();
</script>

<div class="onboarding-wrapper">
  <LlmConfigurationPanel
    mode="onboarding"
    eyebrow="欢迎使用"
    title="连接模型服务"
    description="Steward 需要至少一个可用的模型服务配置。请选择一个提供商，填写所需凭据，确认要使用的模型。"
    submitLabel="继续"
    {onComplete}
  />
</div>

<style>
  .onboarding-wrapper {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 0;
    height: 100%;
    overflow: hidden;
    background: var(--bg-primary);
  }
</style>
