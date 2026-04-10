<script lang="ts">
  import { CalendarDays, ChevronRight, FileText, Search } from "lucide-svelte";
  import { memoryGroups, type MemoryNavItem } from "./memory";

  let {
    memoryError = null,
    onOpenItem,
    onOpenRegression
  }: {
    memoryError?: string | null;
    onOpenItem: (item: MemoryNavItem) => Promise<void> | void;
    onOpenRegression: () => Promise<void> | void;
  } = $props();
</script>

<section class="settings-section">
  <div class="section-header">
    <h4>记忆管理</h4>
    <p>管理 Agent 的核心记忆、身份与上下文信息。</p>
  </div>

  {#if memoryError}
    <p class="memory-status error">{memoryError}</p>
  {/if}

  {#each memoryGroups as group (group.title)}
    <section class="memory-block">
      <div class="block-header compact">
        <div>
          <span class="block-kicker">Memory</span>
          <h5>{group.title}</h5>
        </div>
      </div>

      <div class="memory-list" role="list">
        {#each group.items as item (item.key)}
          <button class="memory-row" type="button" onclick={() => void onOpenItem(item)}>
            <div class="memory-row-icon">
              {#if item.kind === "daily"}
                <CalendarDays size={15} strokeWidth={2} />
              {:else}
                <FileText size={15} strokeWidth={2} />
              {/if}
            </div>
            <div class="memory-row-main">
              <div class="memory-row-head">
                <strong>{item.title}</strong>
              </div>
              <p>{item.description}</p>
            </div>
            <div class="memory-row-tail">
              <ChevronRight size={14} strokeWidth={2} />
            </div>
          </button>
        {/each}
      </div>
    </section>
  {/each}

  <section class="memory-block">
    <div class="block-header compact">
      <div>
        <span class="block-kicker">Regression</span>
        <h5>检索调试</h5>
      </div>
    </div>

    <div class="memory-list" role="list">
      <button class="memory-row" type="button" onclick={() => void onOpenRegression()}>
        <div class="memory-row-icon">
          <Search size={15} strokeWidth={2} />
        </div>
        <div class="memory-row-main">
          <div class="memory-row-head">
            <strong>回归搜索</strong>
          </div>
          <p>使用与 Agent 相同的方法，对记忆进行检索和调试。</p>
        </div>
        <div class="memory-row-tail">
          <ChevronRight size={14} strokeWidth={2} />
        </div>
      </button>
    </div>
  </section>
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

  .memory-block {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding-top: 16px;
    border-top: 1px solid var(--border-subtle, var(--border-default));
  }

  .block-header {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .block-header.compact {
    gap: 2px;
  }

  .block-kicker {
    margin: 0;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .block-header h5 {
    margin: 0;
    font-size: 16px;
    font-weight: 700;
    color: var(--text-primary);
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
    padding: 13px 0;
    border: none;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
    background: transparent;
    text-align: left;
    color: inherit;
    cursor: pointer;
  }

  .memory-row-icon {
    width: 28px;
    height: 28px;
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

  .memory-row-head strong {
    min-width: 0;
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .memory-row-main p {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-secondary);
    word-break: break-word;
  }

  .memory-row-tail {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .memory-status {
    margin: 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .memory-status.error {
    color: var(--accent-danger, #c8594f);
  }
</style>
