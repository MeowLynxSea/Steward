<script lang="ts">
  import { fly, fade } from "svelte/transition";
  import { getToasts } from "../lib/stores/toast.svelte";

  const toasts = $derived(getToasts());
</script>

{#if toasts.length > 0}
  <div class="toast-container">
    {#each toasts as toast (toast.id)}
      <div
        class="toast toast-{toast.type}"
        in:fly={{ y: 30, duration: 220 }}
        out:fade={{ duration: 160 }}
      >
        {toast.message}
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    bottom: 20px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 100;
    display: flex;
    flex-direction: column-reverse;
    gap: 8px;
    pointer-events: none;
  }

  .toast {
    padding: 10px 18px;
    border-radius: 12px;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    background: var(--bg-sidebar);
    border: 1px solid var(--border-default);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.1);
    pointer-events: auto;
    max-width: 400px;
    text-align: center;
  }

  .toast-success {
    border-color: color-mix(in srgb, var(--accent-green) 30%, var(--border-default));
    background: color-mix(in srgb, var(--accent-green) 8%, var(--bg-sidebar));
  }

  .toast-error {
    border-color: color-mix(in srgb, var(--accent-danger-text) 24%, var(--border-default));
    background: color-mix(in srgb, var(--accent-danger-text) 6%, var(--bg-sidebar));
  }
</style>
