<script>
  /**
   * Right pane of the Models page: the local model library. All filter/sort/
   * search is client-side over the existing `models` store — no new backend
   * command needed (verified: every field is already on the ModelDto).
   */
  import { models } from "../lib/stores.svelte.js";
  import LocalModelCard from "./LocalModelCard.svelte";

  let query = $state("");
  let roleFilter = $state("all"); // all | chat | embedding | untagged
  let sortBy = $state("name"); // name | size_desc | size_asc | ctx_desc

  // Derived filtered + sorted list (computed reactively from inputs + store).
  let filtered = $derived.by(() => {
    const q = query.toLowerCase().trim();
    let list = models.list.filter((m) => {
      if (q) {
        const hay = `${m.filename} ${m.model_name} ${m.architecture} ${m.author ?? ""}`.toLowerCase();
        if (!hay.includes(q)) return false;
      }
      if (roleFilter === "chat" && m.role !== "chat") return false;
      if (roleFilter === "embedding" && m.role !== "embedding") return false;
      if (roleFilter === "untagged" && m.role !== null) return false;
      return true;
    });
    list = [...list].sort((a, b) => {
      switch (sortBy) {
        case "size_desc": return b.filesize_bytes - a.filesize_bytes;
        case "size_asc": return a.filesize_bytes - b.filesize_bytes;
        case "ctx_desc": return b.context_length - a.context_length;
        default: return a.model_name.localeCompare(b.model_name);
      }
    });
    return list;
  });

  function clearFilters() {
    query = ""; roleFilter = "all"; sortBy = "name";
  }
</script>

<div class="local-library">
  <div class="lib-header">
    <h2>Your Library</h2>
    <span class="count">{filtered.length} model{filtered.length === 1 ? "" : "s"}</span>
  </div>

  <div class="lib-filters">
    <input class="text-input" type="search" placeholder="Filter your models…" bind:value={query} />
    <div class="select-row">
      <select class="select" bind:value={roleFilter}>
        <option value="all">All roles</option>
        <option value="chat">Chat</option>
        <option value="embedding">Embeddings</option>
        <option value="untagged">Untagged</option>
      </select>
      <select class="select" bind:value={sortBy}>
        <option value="name">Name (A–Z)</option>
        <option value="size_desc">Largest first</option>
        <option value="size_asc">Smallest first</option>
        <option value="ctx_desc">Longest context</option>
      </select>
      <button class="link-btn" onclick={clearFilters}>Clear</button>
    </div>
  </div>

  <div class="lib-grid">
    {#if filtered.length === 0}
      <div class="empty">
        {#if models.list.length === 0}
          No models yet. Use the HuggingFace browser on the left to download some.
        {:else}
          No models match the current filters.
        {/if}
      </div>
    {:else}
      {#each filtered as model (model.id)}
        <LocalModelCard {model} />
      {/each}
    {/if}
  </div>
</div>

<style>
  .local-library { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 0.6rem; }
  .lib-header { display: flex; align-items: baseline; gap: 0.5rem; }
  .lib-header h2 { margin: 0; font-size: 0.95rem; }
  .count { color: var(--text-muted); font-size: 0.72rem; }
  .lib-filters { display: flex; flex-direction: column; gap: 0.4rem; }
  .text-input, .select {
    padding: 0.35rem 0.45rem; font-size: 0.76rem;
    background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: var(--radius-sm); color: var(--text-primary);
  }
  .select-row { display: flex; gap: 0.4rem; align-items: center; }
  .select { font-size: 0.72rem; }
  .link-btn { background: none; border: none; color: var(--text-muted); font-size: 0.7rem; cursor: pointer; text-decoration: underline; }
  .link-btn:hover { color: var(--text-secondary); }
  .lib-grid { overflow-y: auto; flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 0.4rem; }
  .empty { color: var(--text-muted); font-size: 0.78rem; text-align: center; padding: 2rem 1rem; }
</style>
