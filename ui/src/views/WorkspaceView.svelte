<script lang="ts">
  import { workspaceStore } from "../lib/stores/workspace.svelte";
</script>

<section class="view-grid split-grid">
  <section class="panel column-panel">
    <div class="card-head">
      <div>
        <p class="eyebrow">Workspace</p>
        <h2>Indexed tree</h2>
      </div>
      <button class="button button-primary" onclick={() => void workspaceStore.refresh()}>Refresh</button>
    </div>

    <div class="inline-form">
      <input bind:value={workspaceStore.path} placeholder="Folder path to index" />
      <button class="button button-secondary" onclick={() => void workspaceStore.index(workspaceStore.path)}>
        Index Folder
      </button>
    </div>

    {#if workspaceStore.indexJob}
      <article class="mini-card">
        <strong>{workspaceStore.indexJob.phase}</strong>
        <span>
          {workspaceStore.indexJob.processed_files} / {workspaceStore.indexJob.total_files || "?"}
          files · {workspaceStore.indexJob.indexed_files} indexed · {workspaceStore.indexJob.skipped_files} skipped
        </span>
      </article>
    {/if}

    {#if workspaceStore.loading}
      <p class="muted">Loading workspace...</p>
    {:else if workspaceStore.entries.length === 0}
      <p class="muted">Workspace is empty. Index a folder to get started.</p>
    {:else}
      <div class="stack compact">
        {#each workspaceStore.entries as entry}
          <article class="mini-card">
            <strong>{entry.path}</strong>
            <span>{entry.is_directory ? "dir" : "file"}</span>
            {#if entry.updated_at}
              <span>{new Date(entry.updated_at).toLocaleString()}</span>
            {/if}
          </article>
        {/each}
      </div>
    {/if}
  </section>

  <section class="panel detail-panel">
    <div class="card-head">
      <div>
        <p class="eyebrow">Search</p>
        <h2>Indexed documents</h2>
      </div>
    </div>

    <div class="inline-form">
      <input bind:value={workspaceStore.searchQuery} placeholder="Search indexed notes and documents" />
      <button class="button button-primary" onclick={() => void workspaceStore.search(workspaceStore.searchQuery)}>
        Search
      </button>
    </div>

    {#if workspaceStore.searchLoading}
      <p class="muted">Searching...</p>
    {:else if workspaceStore.searchResults.length === 0}
      <div class="empty-state">
        <h3>No results yet</h3>
        <p>Run a search above to inspect chunks from indexed workspace memory.</p>
      </div>
    {:else}
      <div class="stack compact">
        {#each workspaceStore.searchResults as result}
          <article class="feature-card soft-card">
            <div class="mini-card-head">
              <strong>{result.document_path}</strong>
              <span>score {result.score.toFixed(3)}</span>
            </div>
            {#if result.source_path}
              <span class="muted">{result.source_path}</span>
            {/if}
            <p>{result.content}</p>
          </article>
        {/each}
      </div>
    {/if}
  </section>
</section>
