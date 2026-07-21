<script>
  /**
   * One .gguf file row inside an expanded HuggingFace repo. Shows filename,
   * quant (parsed from filename), size (fetched lazily), and a context-aware
   * download button.
   */
  import { fmtBytes } from "../lib/format.js";

  let { repoId, file, models, downloads, onDownload, onCancel } = $props();

  // Quant convention: embedded in filename (e.g. "model-Q4_K_M.gguf").
  let quant = $derived(parseQuant(file.filename));
  let dlKey = $derived(`${repoId}/${file.filename}`);

  // Is this file already in the local library?
  let alreadyLocal = $derived(models.list.some((m) => m.filename === file.filename));

  // Active or completed download for this file?
  let dl = $derived(downloads.map[dlKey]);
  let isDownloading = $derived(dl?.status === "active");
  // Percentage requires a known total. When the CDN doesn't send Content-Length,
  // dl.total is null — show an indeterminate bar (100% width pulse via CSS) and
  // display the raw downloaded byte count instead of a frozen 0%.
  let pct = $derived(dl && dl.total ? Math.min(100, (dl.downloaded / dl.total) * 100) : 0);
  let hasTotal = $derived(dl && dl.total !== null && dl.total > 0);

  /** Extract a quant label like "Q4_K_M" from a GGUF filename. */
  function parseQuant(name) {
    const m = name.match(/Q\d+[A-Za-z0-9_]*/i);
    return m ? m[0] : "—";
  }
</script>

<div class="hf-file-row" class:local={alreadyLocal}>
  <div class="file-main">
    <span class="file-name" title={file.filename}>{file.filename}</span>
    <div class="file-meta">
      {#if quant !== "—"}<span class="tag">{quant}</span>{/if}
      {#if file.size_bytes}<span class="file-size">{fmtBytes(file.size_bytes)}</span>{/if}
    </div>
  </div>

  <div class="file-action">
    {#if alreadyLocal}
      <span class="in-lib" title="Already in your library">✓ In Library</span>
    {:else if isDownloading}
      <div class="dl-progress" title={hasTotal ? `${fmtBytes(dl.downloaded)} / ${fmtBytes(dl.total)}` : `${fmtBytes(dl.downloaded)} downloaded`}>
        {#if hasTotal}
          <span class="dl-pct">{pct.toFixed(0)}%</span>
          <div class="dl-bar"><div class="dl-fill" style="width: {pct}%"></div></div>
        {:else}
          <span class="dl-bytes">{fmtBytes(dl.downloaded)}</span>
          <div class="dl-bar"><div class="dl-fill dl-indeterminate"></div></div>
        {/if}
        <button class="cancel-btn" onclick={() => onCancel(dlKey)}>Cancel</button>
      </div>
    {:else if dl?.status === "completed"}
      <span class="done">✓ Downloaded</span>
    {:else if dl?.status === "failed"}
      <button class="retry-btn" title={dl.message ?? "Download failed"}
        onclick={() => onDownload(dlKey, repoId, file.filename)}>Retry</button>
    {:else}
      <button class="dl-btn" onclick={() => onDownload(dlKey, repoId, file.filename)}>Download</button>
    {/if}
  </div>
</div>

<style>
  .hf-file-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
  }
  .hf-file-row.local { opacity: 0.55; }
  .file-main { display: flex; flex-direction: column; gap: 0.1rem; min-width: 0; flex: 1; }
  .file-name {
    font-family: var(--font-mono);
    font-size: 0.76rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .file-meta { display: flex; gap: 0.4rem; align-items: center; font-size: 0.68rem; color: var(--text-muted); }
  .tag {
    font-family: var(--font-mono);
    color: var(--accent);
    background: var(--accent-bg);
    padding: 0 0.3rem;
    border-radius: var(--radius-sm);
  }
  .file-action { flex-shrink: 0; }
  .dl-btn, .retry-btn {
    padding: 0.2rem 0.6rem;
    font-size: 0.72rem;
    border: 1px solid var(--accent-border);
    background: var(--accent-bg);
    color: var(--accent);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .dl-btn:hover, .retry-btn:hover { background: var(--accent); color: var(--bg-base); }
  .cancel-btn {
    padding: 0.1rem 0.4rem;
    font-size: 0.68rem;
    border: 1px solid var(--danger-border);
    background: var(--danger-bg);
    color: var(--danger);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .in-lib, .done { font-size: 0.7rem; color: var(--success); }
  .dl-progress { display: flex; align-items: center; gap: 0.35rem; }
  .dl-pct { font-size: 0.66rem; color: var(--text-muted); font-variant-numeric: tabular-nums; min-width: 2rem; text-align: right; }
  .dl-bytes { font-size: 0.66rem; color: var(--text-muted); font-variant-numeric: tabular-nums; min-width: 3rem; text-align: right; }
  .dl-bar { width: 80px; height: 4px; background: var(--bg-surface-2); border-radius: 3px; overflow: hidden; }
  .dl-fill { height: 100%; background: var(--accent); transition: width 0.3s ease; }
  /* Indeterminate bar: pulse animation when total is unknown */
  .dl-fill.dl-indeterminate {
    width: 40%;
    animation: dl-pulse 1.5s ease-in-out infinite;
  }
  @keyframes dl-pulse {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(250%); }
  }
</style>
