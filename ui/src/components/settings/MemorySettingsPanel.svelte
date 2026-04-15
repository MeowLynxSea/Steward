<script lang="ts">
  import {
    ChevronRight,
    Network,
    Search
  } from "lucide-svelte";

  let {
    memoryError = null,
    onOpenGraph,
    onOpenSearch
  }: {
    memoryError?: string | null;
    onOpenGraph: () => Promise<void> | void;
    onOpenSearch: () => Promise<void> | void;
  } = $props();
</script>

<section class="settings-section">
  <div class="section-header">
    <h4>记忆管理</h4>
    <p>查看和搜索助手积累的知识与记忆。</p>
  </div>

  {#if memoryError}
    <p class="memory-status error">{memoryError}</p>
  {/if}

  <div class="memory-list" role="list">
    <button class="memory-row" type="button" onclick={() => void onOpenGraph()}>
      <div class="memory-row-icon">
        <Network size={15} strokeWidth={2} />
      </div>
      <div class="memory-row-main">
        <div class="memory-row-head">
          <strong>记忆图谱</strong>
        </div>
        <p>以可视化图谱的方式浏览所有记忆。</p>
      </div>
      <div class="memory-row-tail">
        <ChevronRight size={14} strokeWidth={2} />
      </div>
    </button>

    <button class="memory-row" type="button" onclick={() => void onOpenSearch()}>
      <div class="memory-row-icon">
        <Search size={15} strokeWidth={2} />
      </div>
      <div class="memory-row-main">
        <div class="memory-row-head">
          <strong>记忆搜索</strong>
        </div>
        <p>按关键词搜索记忆内容。</p>
      </div>
      <div class="memory-row-tail">
        <ChevronRight size={14} strokeWidth={2} />
      </div>
    </button>
  </div>
</section>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .section-header h4 {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .section-header p {
    margin: 6px 0 0;
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1.5;
  }

  .memory-status {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-secondary);
  }

  .memory-status.error {
    color: var(--accent-danger, #c8594f);
  }

  .memory-list {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border-subtle, var(--border-default));
  }

  .memory-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 0;
    border: none;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
    background: transparent;
    text-align: left;
    color: inherit;
    cursor: pointer;
  }

  .memory-row-icon {
    width: 30px;
    height: 30px;
    border-radius: 10px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
    background: color-mix(in srgb, var(--bg-input) 88%, transparent);
    flex-shrink: 0;
  }

  .memory-row-main {
    min-width: 0;
    flex: 1;
  }

  .memory-row-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .memory-row-head strong {
    font-size: 14px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .memory-row-main p {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  .memory-row-tail {
    width: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .memory-row:hover .memory-row-icon {
    color: var(--accent-primary);
    background: color-mix(in srgb, var(--accent-primary) 10%, var(--bg-input));
  }

  .memory-row:hover .memory-row-tail {
    color: var(--text-primary);
    transform: translateX(2px);
  }
</style>
