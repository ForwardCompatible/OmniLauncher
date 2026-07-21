<script>
  /**
   * One local-model card in the right pane. Pure display — shows the parsed
   * GGUF metadata. Launching happens on the Loader page.
   */
  import { fmtBytes } from "../lib/format.js";

  let { model } = $props();

  let roleLabel = $derived(
    model.role === "chat" ? "chat" :
    model.role === "embedding" ? "embedding" : "untagged"
  );
</script>

<div class="local-card" data-role={roleLabel}>
  <div class="card-head">
    <span class="model-name" title={model.model_name}>{model.model_name}</span>
    <span class="role-badge {roleLabel}">{roleLabel}</span>
  </div>
  <div class="card-meta">
    <span class="meta-tag" title="Architecture">{model.architecture}</span>
    <span class="meta-tag quant" title="Quantization">{model.quantization}</span>
    <span class="meta-tag" title="File size">{fmtBytes(model.filesize_bytes)}</span>
    {#if model.context_length}
      <span class="meta-tag" title="Max context">{(model.context_length / 1000).toFixed(0)}k ctx</span>
    {/if}
  </div>
  <div class="card-foot">
    <span class="filename" title={model.filename}>{model.filename}</span>
    {#if model.has_settings}<span class="settings-badge" title="Has custom launch settings">⚙</span>{/if}
  </div>
</div>

<style>
  .local-card {
    display: flex; flex-direction: column; gap: 0.3rem;
    padding: 0.6rem 0.7rem;
    background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }
  .local-card:hover { border-color: var(--accent-border); }
  .card-head { display: flex; justify-content: space-between; align-items: baseline; gap: 0.5rem; }
  .model-name { font-weight: 600; font-size: 0.82rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .role-badge { font-size: 0.6rem; padding: 0.05rem 0.3rem; border-radius: var(--radius-sm); text-transform: uppercase; letter-spacing: 0.03em; flex-shrink: 0; }
  .role-badge.chat { color: var(--accent); background: var(--accent-bg); }
  .role-badge.embedding { color: var(--success); background: var(--success-bg); }
  .role-badge.untagged { color: var(--text-muted); background: var(--bg-elevated); }
  .card-meta { display: flex; gap: 0.3rem; flex-wrap: wrap; }
  .meta-tag { font-size: 0.66rem; color: var(--text-muted); font-family: var(--font-mono); background: var(--bg-elevated); padding: 0.05rem 0.3rem; border-radius: var(--radius-sm); }
  .meta-tag.quant { color: var(--accent); }
  .card-foot { display: flex; justify-content: space-between; align-items: center; gap: 0.4rem; }
  .filename { font-family: var(--font-mono); font-size: 0.65rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .settings-badge { font-size: 0.7rem; color: var(--text-secondary); flex-shrink: 0; }
</style>
