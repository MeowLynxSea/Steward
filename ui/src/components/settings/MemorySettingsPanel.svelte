<script lang="ts">
  import {
    CalendarDays,
    ChevronRight,
    FileText,
    GitBranch,
    Search
  } from "lucide-svelte";
  import type {
    MemoryChangeSet,
    MemorySidebarItem,
    MemorySidebarSection,
    MemoryTimelineEntry
  } from "../../lib/types";
  import {
    formatMemoryTimestamp,
    memoryItemLabel,
    memoryKindLabel,
    memoryRouteKey
  } from "./memory";

  let {
    memorySections = [],
    memoryTimeline = [],
    memoryReviews = [],
    memoryError = null,
    onOpenItem,
    onOpenSearch,
    onOpenReviews
  }: {
    memorySections?: MemorySidebarSection[];
    memoryTimeline?: MemoryTimelineEntry[];
    memoryReviews?: MemoryChangeSet[];
    memoryError?: string | null;
    onOpenItem: (item: MemorySidebarItem) => Promise<void> | void;
    onOpenSearch: () => Promise<void> | void;
    onOpenReviews: () => Promise<void> | void;
  } = $props();

  const contentSections = $derived(memorySections.filter((section) => section.key !== "reviews"));
</script>

<section class="settings-section">
  <div class="section-header">
    <h4>Memory Inspector</h4>
    <p>检查 Steward 的原生记忆图谱，而不是任何旧的文件式记忆视图。</p>
  </div>

  {#if memoryError}
    <p class="memory-status error">{memoryError}</p>
  {/if}

  {#each contentSections as section (section.key)}
    <section class="memory-block">
      <div class="block-header compact">
        <div>
          <span class="block-kicker">Memory</span>
          <h5>{section.title}</h5>
        </div>
      </div>

      <div class="memory-list" role="list">
        {#each section.items as item (memoryRouteKey(item))}
          <button class="memory-row" type="button" onclick={() => void onOpenItem(item)}>
            <div class="memory-row-icon">
              {#if item.kind === "episode"}
                <CalendarDays size={15} strokeWidth={2} />
              {:else}
                <FileText size={15} strokeWidth={2} />
              {/if}
            </div>
            <div class="memory-row-main">
              <div class="memory-row-head">
                <strong>{memoryItemLabel(item)}</strong>
                <span>{memoryKindLabel(item.kind)}</span>
              </div>
              <p>{item.subtitle ?? "打开节点详情"}</p>
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
        <span class="block-kicker">Timeline</span>
        <h5>Recent Episodes</h5>
      </div>
    </div>

    {#if memoryTimeline.length === 0}
      <p class="memory-status">最近还没有时间线记忆。</p>
    {:else}
      <div class="timeline-list" role="list">
        {#each memoryTimeline.slice(0, 4) as entry (entry.node_id)}
          <div class="timeline-row" role="listitem">
            <div class="timeline-head">
              <strong>{entry.title}</strong>
              <span>{formatMemoryTimestamp(entry.updated_at)}</span>
            </div>
            <p>{entry.content_snippet}</p>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <section class="memory-block">
    <div class="block-header compact">
      <div>
        <span class="block-kicker">Review</span>
        <h5>变更审查</h5>
      </div>
    </div>

    <div class="memory-list" role="list">
      <button class="memory-row" type="button" onclick={() => void onOpenReviews()}>
        <div class="memory-row-icon">
          <GitBranch size={15} strokeWidth={2} />
        </div>
        <div class="memory-row-main">
          <div class="memory-row-head">
            <strong>Review Queue</strong>
          </div>
          <p>
            {#if memoryReviews.length > 0}
              {memoryReviews.length} 个待处理 changeset
            {:else}
              当前没有待审查的记忆变更
            {/if}
          </p>
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
            <strong>Recall Search</strong>
          </div>
          <p>使用 graph memory 的检索入口调试 route、trigger 和 recall 结果。</p>
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

  .memory-list,
  .timeline-list {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border-subtle, var(--border-default));
  }

  .memory-row,
  .timeline-row {
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
  }

  .memory-row {
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

  .memory-row-main,
  .timeline-row {
    min-width: 0;
    flex: 1;
  }

  .memory-row-head,
  .timeline-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .memory-row-head strong,
  .timeline-head strong {
    min-width: 0;
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .memory-row-head span,
  .timeline-head span {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .memory-row-main p,
  .timeline-row p {
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
