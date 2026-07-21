<script>
  /**
   * Left pane of the Models page: HuggingFace OAuth + search + download.
   *
   * BUG-PREVENTION NOTE: the previous (deleted) version of this used a Svelte 5
   * `$effect` that wrote to a `$state` it read, creating an infinite loop that
   * crashed AppShell reactivity. Here the debounce timer is a plain `let` (NOT
   * `$state`), so mutating it does not trigger reactivity. Search runs on input
   * change via a plain handler + onMount (initial fetch), never via `$effect`.
   */
  import { onMount, onDestroy } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import { hfAuth, hfSearch, hfDownloads, setHfDownload, clearHfDownload, refreshHfAuth, refreshModels, pushError, models } from "../lib/stores.svelte.js";
  import { hfAuthStart, hfAuthPoll, hfAuthLogout, hfSearch as hfSearchCmd, hfListFiles, hfFileSize, hfReadme, hfDownload, hfCancelDownload } from "../lib/commands.js";
  import { fmtBytes } from "../lib/format.js";
  import HfFileRow from "./HfFileRow.svelte";

  // ── Search state (plain lets for non-reactive plumbing) ──
  let query = $state("");
  let sort = $state("downloads");
  let pipelineTag = $state("");
  let ggufOnly = $state(true);
  let timer = null; // PLAIN let, not $state — debounce timer
  let hasSearched = $state(false); // false until the user actually runs a search

  // ── Auth state ──
  let authModal = $state(null); // null | { user_code, verification_uri, expires_in, device_code }
  let authError = $state("");
  let pollTimer = null;
  let pollFailures = 0; // consecutive transient poll errors; abort after 5
  const MAX_POLL_FAILURES = 5;
  let copied = $state(false); // copy-button feedback state

  // ── README modal state ──
  // null | { repoId, html, loading, error }
  let readmeModal = $state(null);

  // ── Expanded repos: repoId → { files, meta, loading, error } ──
  let expanded = $state({});

  // ── Download id tracking is no longer needed here — the download id is
  // stored on each hfDownloads.map entry (entry.id), and the progress listener
  // in Models.svelte scans the map by id.

  // On mount: only refresh auth status. We do NOT pre-fetch search results —
  // HuggingFace is only queried once the user actually submits a search or
  // changes a filter, per the requirement to avoid hitting HF on page load.
  onMount(() => {
    refreshHfAuth();
  });
  onDestroy(() => {
    if (timer) clearTimeout(timer);
    if (pollTimer) clearInterval(pollTimer);
  });

  // ── Search (debounced via plain setTimeout, never $effect) ──
  function onSearchInput() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(doSearch, 750);
  }

  async function doSearch(cursor = null) {
    hasSearched = true;
    hfSearch.loading = true;
    try {
      const page = await hfSearchCmd({
        query, sort,
        pipeline_tag: pipelineTag || undefined,
        gguf_only: ggufOnly,
        cursor: cursor || undefined,
      });
      if (cursor) {
        // Append for "load more"
        hfSearch.results = [...hfSearch.results, ...page.results];
      } else {
        hfSearch.results = page.results;
      }
      hfSearch.nextCursor = page.next_cursor;
    } catch (e) {
      pushError(`HuggingFace search failed: ${e}`, "warning");
    } finally {
      hfSearch.loading = false;
    }
  }

  function clearFilters() {
    query = ""; sort = "downloads"; pipelineTag = ""; ggufOnly = true;
    hasSearched = false;
    hfSearch.results = [];
    hfSearch.nextCursor = null;
  }

  // ── Repo expand (lazy file listing) ──
  async function toggleRepo(repoId) {
    if (expanded[repoId]) {
      delete expanded[repoId];
      return;
    }
    expanded[repoId] = { files: [], meta: null, loading: true };
    try {
      const resp = await hfListFiles(repoId);
      // Store the repo-level GGUF metadata (architecture, context_length,
      // total_file_size) for display above the file list.
      const meta = {
        architecture: resp.architecture ?? null,
        context_length: resp.context_length ?? null,
        total_file_size: resp.total_file_size ?? null,
      };
      // Lazily fetch each file's size (siblings don't carry it).
      const filesWithSizes = await Promise.all(
        resp.files.map(async (f) => {
          try {
            const size = await hfFileSize(repoId, f.filename);
            return { ...f, size_bytes: size };
          } catch {
            return f; // size stays null (e.g. gated file)
          }
        })
      );
      expanded[repoId] = { files: filesWithSizes, meta, loading: false };
    } catch (e) {
      expanded[repoId] = { files: [], meta: null, loading: false, error: String(e) };
    }
  }

  /** Format total_file_size (bytes) from the gguf dict as a human-readable
   *  param-count string, e.g. 7615616512 → "7.6B params". */
  /** @param {number|null} bytes */
  function fmtParams(bytes) {
    if (bytes === null) return null;
    const billion = bytes / 1_000_000_000;
    if (billion >= 1) return billion.toFixed(1) + "B params";
    const million = bytes / 1_000_000;
    if (million >= 1) return million.toFixed(0) + "M params";
    return bytes + " params";
  }

  /** @param {number|null} ctx */
  function fmtContext(ctx) {
    if (ctx === null) return null;
    if (ctx >= 1000) return (ctx / 1000).toFixed(0) + "k context";
    return ctx + " context";
  }

  // ── README (model card) modal ──
  async function openReadme(repoId) {
    readmeModal = { repoId, html: "", loading: true, error: null };
    try {
      const markdown = await hfReadme(repoId);
      if (!markdown.trim()) {
        readmeModal = { repoId, html: "", loading: false, error: "This repository has no README." };
        return;
      }
      // Strip YAML frontmatter (---\n...\n---) before rendering — marked would
      // otherwise render it as a visible hr + raw key:value text.
      const stripped = markdown.replace(/^---\n[\s\S]*?\n---\n?/, "");
      const rawHtml = marked.parse(stripped, { breaks: true });
      // Sanitize to prevent XSS from untrusted HF content before rendering.
      const cleanHtml = DOMPurify.sanitize(rawHtml);
      readmeModal = { repoId, html: cleanHtml, loading: false, error: null };
    } catch (e) {
      readmeModal = { repoId, html: "", loading: false, error: String(e) };
    }
  }

  function closeReadmeModal() {
    readmeModal = null;
  }

  // ── Download handling ──
  async function handleDownload(dlKey, repoId, filename) {
    setHfDownload(dlKey, { id: 0, downloaded: 0, total: null, status: "active" });
    try {
      const id = await hfDownload(repoId, filename);
      setHfDownload(dlKey, { id, downloaded: 0, total: null, status: "active" });
    } catch (e) {
      setHfDownload(dlKey, { id: 0, downloaded: 0, total: null, status: "failed", message: String(e) });
      pushError(`Download failed to start: ${e}`, "warning");
    }
  }

  async function handleCancel(dlKey) {
    // Read the download id from the store entry (where handleDownload stored it).
    const entry = hfDownloads.map[dlKey];
    if (entry && entry.id) {
      try { await hfCancelDownload(entry.id); } catch (e) { pushError(`Cancel failed: ${e}`, "warning"); }
    }
    clearHfDownload(dlKey);
  }

  // ── OAuth device-code flow ──
  async function startAuth() {
    authError = "";
    pollFailures = 0;
    try {
      const info = await hfAuthStart();
      authModal = { ...info, device_code: info.device_code };
      // Open the verification page in the user's default system browser via the
      // opener plugin (window.open would target Tauri's webview, not the OS).
      try {
        await openUrl(info.verification_uri);
      } catch (e) {
        // Non-fatal — the modal still shows the URL as a clickable link.
        console.warn("openUrl failed:", e);
      }
      // Start polling every 5s (RFC 8628 default interval).
      pollTimer = setInterval(pollAuth, 5000);
    } catch (e) {
      // Surface in the auth-bar (NOT just the modal, which never opened).
      authError = `Could not start sign-in: ${e}`;
    }
  }

  async function pollAuth() {
    if (!authModal) return;
    try {
      const outcome = await hfAuthPoll(authModal.device_code);
      pollFailures = 0; // got a valid response — reset the transient-error counter
      if (outcome.status === "granted") {
        // The grant response IS authoritative for the badge — it carries the
        // username + expires_at. Do NOT call refreshHfAuth() here: that would
        // race the keychain write and overwrite this with a load_token() round-
        // trip that may return None transiently. The onMount refreshHfAuth()
        // handles cross-restart hydration; in-session the grant is the truth.
        hfAuth.data = {
          connected: true,
          username: outcome.username ?? null,
          expires_at: outcome.expires_at ?? null,
          keychain_unavailable: false,
        };
        closeAuthModal();
      } else if (outcome.status === "expired" || outcome.status === "denied") {
        authError = outcome.message ?? (outcome.status === "expired"
          ? "The sign-in code expired. Try again."
          : "Authorization was denied.");
        closeAuthModal();
      }
      // "pending" and "slow_down" → keep polling silently
    } catch (e) {
      // Issue 4: DON'T kill the poll loop on a single transient error (network
      // blip, IPC hiccup). Increment a counter and only abort after several
      // consecutive failures — this prevents one dropped packet from aborting
      // an in-progress sign-in.
      pollFailures++;
      if (pollFailures >= MAX_POLL_FAILURES) {
        authError = `Sign-in check failed after ${MAX_POLL_FAILURES} attempts: ${e}`;
        closeAuthModal();
      } else {
        // Log and keep polling — the interval continues.
        console.warn(`pollAuth transient failure (${pollFailures}/${MAX_POLL_FAILURES}):`, e);
      }
    }
  }

  // ── Copy-to-clipboard for the device code ──
  async function copyUserCode() {
    if (!authModal) return;
    try {
      await navigator.clipboard.writeText(authModal.user_code);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      // clipboard API can be unavailable in some webview contexts; ignore.
    }
  }

  function closeAuthModal() {
    authModal = null;
    if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
  }

  async function handleLogout() {
    try {
      await hfAuthLogout();
      await refreshHfAuth();
    } catch (e) { pushError(`Sign-out failed: ${e}`, "warning"); }
  }

  // ── Helpers ──
  /** @param {number|null|undefined} n */
  function fmtCount(n) {
    if (n === null || n === undefined) return "";
    if (n >= 1000) return (n / 1000).toFixed(1) + "k";
    return String(n);
  }

  // Active downloads pinned at top of results so they survive search-query
  // changes. Derived from the shared hfDownloads store (module-level state).
  let activeDownloads = $derived(
    Object.entries(hfDownloads.map)
      .filter(([, entry]) => entry.status === "active")
      .map(([key, entry]) => {
        const [repoId, filename] = key.split("/");
        const pct = entry.total ? Math.min(100, (entry.downloaded / entry.total) * 100) : 0;
        const hasTotal = entry.total !== null && entry.total > 0;
        return { key, repoId, filename: filename ?? key, entry, pct, hasTotal };
      })
  );
</script>

<div class="hf-browser">
  <!-- Auth bar -->
  <div class="auth-bar">
    {#if hfAuth.data?.keychain_unavailable}
      <span class="auth-note">OS keychain unavailable — sign-in disabled on this machine.</span>
    {:else if hfAuth.data?.connected}
      <span class="auth-ok">✓ Connected as {hfAuth.data.username ?? "(unknown)"}</span>
      <button class="link-btn" onclick={handleLogout}>Sign out</button>
    {:else}
      <button class="auth-btn" onclick={startAuth}>Sign in to HuggingFace</button>
      <span class="auth-note">Optional — raises rate limits, enables gated models</span>
    {/if}
    {#if authError}<span class="auth-error-inline" title={authError}>{authError}</span>{/if}
  </div>

  <!-- Search controls -->
  <div class="filters">
    <input class="text-input" type="search" placeholder="Search models…" bind:value={query} oninput={onSearchInput} />
    <div class="select-row">
      <select class="select" bind:value={sort} onchange={() => doSearch()}>
        <option value="downloads">Most downloaded</option>
        <option value="likes">Most liked</option>
        <option value="trendingScore">Trending</option>
        <option value="lastModified">Recently updated</option>
        <option value="createdAt">Recently created</option>
      </select>
      <select class="select" bind:value={pipelineTag} onchange={() => doSearch()}>
        <option value="">Any task</option>
        <option value="text-generation">Chat (text-generation)</option>
        <option value="feature-extraction">Embeddings (feature-extraction)</option>
        <option value="image-text-to-text">Vision (image-text-to-text)</option>
      </select>
      <label class="checkbox">
        <input type="checkbox" bind:checked={ggufOnly} onchange={() => doSearch()} />
        <span>GGUF only</span>
      </label>
      <button class="link-btn" onclick={clearFilters}>Clear</button>
    </div>
  </div>

  <!-- Results -->
  <div class="results">
    {#if activeDownloads.length > 0}
      <div class="pinned-downloads">
        <div class="pinned-header">Downloads in progress</div>
        {#each activeDownloads as d (d.key)}
          <div class="pinned-row">
            <div class="pinned-info">
              <span class="pinned-filename" title={d.filename}>{d.filename}</span>
              <span class="pinned-repo">{d.repoId}</span>
            </div>
            <div class="pinned-progress">
              {#if d.hasTotal}
                <span class="pinned-pct">{d.pct.toFixed(0)}%</span>
                <div class="pinned-bar"><div class="pinned-fill" style="width: {d.pct}%"></div></div>
              {:else}
                <span class="pinned-pct">{fmtBytes(d.entry.downloaded)}</span>
                <div class="pinned-bar"><div class="pinned-fill pinned-indeterminate"></div></div>
              {/if}
              <button class="pinned-cancel" onclick={() => handleCancel(d.key)}>Cancel</button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {#if hfSearch.loading && hfSearch.results.length === 0}
      <div class="status">Searching…</div>
    {:else if !hasSearched}
      <div class="status prompt">Enter a search term or adjust filters to find models on HuggingFace.</div>
    {:else if hfSearch.results.length === 0}
      <div class="status">No GGUF models found. Try a more specific search term — some queries return non-GGUF repos that are filtered out.</div>
    {:else}
      {#each hfSearch.results as model (model.id)}
        <div class="repo">
          <button class="repo-header" onclick={() => toggleRepo(model.id)}>
            <span class="chevron">{expanded[model.id] ? "▾" : "▸"}</span>
            <span class="repo-name">{model.id}</span>
            {#if model.gated !== "no"}<span class="gated-badge" title="Gated repo — requires access approval">gated</span>{/if}
            {#if model.downloads !== null && model.downloads !== undefined}
              <span class="dl-count" title="30-day downloads">↓ {fmtCount(model.downloads)}</span>
            {/if}
            {#if model.pipeline_tag}<span class="pipeline-tag">{model.pipeline_tag}</span>{/if}
          </button>
          {#if expanded[model.id]}
            <div class="repo-files">
              {#if expanded[model.id].loading}
                <div class="status">Loading files…</div>
              {:else if expanded[model.id].error}
                <div class="status error">Failed: {expanded[model.id].error}</div>
              {:else if expanded[model.id].files.length === 0}
                <div class="status">No .gguf files in this repo.</div>
              {:else}
                {#if expanded[model.id].meta}
                  {@const m = expanded[model.id].meta}
                  {@const parts = [m.architecture, fmtParams(m.total_file_size), fmtContext(m.context_length)].filter(Boolean)}
                  {#if parts.length > 0}
                    <div class="repo-meta-bar">{parts.join(" · ")}</div>
                  {/if}
                {/if}
                <button class="model-info-btn" onclick={() => openReadme(model.id)}>
                  Model Info
                </button>
                {#each expanded[model.id].files as file (file.filename)}
                  <HfFileRow
                    repoId={model.id} {file} {models} downloads={hfDownloads}
                    onDownload={handleDownload} onCancel={handleCancel}
                  />
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      {/each}
      {#if hfSearch.nextCursor}
        <button class="load-more" onclick={() => doSearch(hfSearch.nextCursor)} disabled={hfSearch.loading}>
          {hfSearch.loading ? "Loading…" : "Load more"}
        </button>
      {/if}
    {/if}
  </div>
</div>

<!-- Auth modal -->
{#if authModal}
  <!-- Overlay: role=button + tabindex + Escape handling for a11y (Svelte
       warns on click-only divs). Escape closes, matching native modal UX. -->
  <div
    class="auth-modal-overlay"
    role="button"
    tabindex="-1"
    aria-label="Close sign-in dialog"
    onclick={closeAuthModal}
    onkeydown={(e) => e.key === "Escape" && closeAuthModal()}
  >
    <div class="auth-modal" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="auth-modal-title" tabindex="-1">
      <h3 id="auth-modal-title">Sign in to HuggingFace</h3>
      <p>Enter this code at the page that opened in your browser:</p>
      <div class="user-code-row">
        <div class="user-code">{authModal.user_code}</div>
        <button type="button" class="copy-code-btn" onclick={copyUserCode} title="Copy code to clipboard">
          {copied ? "✓" : "⧉"}
        </button>
      </div>
      {#if copied}<p class="copied-confirm">✓ Copied to clipboard</p>{/if}
      <p class="modal-link">
        Didn't open?
        <button type="button" class="modal-link-btn" onclick={() => openUrl(authModal.verification_uri)}>
          {authModal.verification_uri}
        </button>
      </p>
      <p class="waiting">Waiting for authorization…</p>
      {#if authError}<p class="auth-error">{authError}</p>{/if}
      <button class="cancel-auth" onclick={closeAuthModal}>Cancel</button>
    </div>
  </div>
{/if}

<!-- README (model card) modal -->
{#if readmeModal}
  <div
    class="readme-modal-overlay"
    role="button"
    tabindex="-1"
    aria-label="Close model info"
    onclick={closeReadmeModal}
    onkeydown={(e) => e.key === "Escape" && closeReadmeModal()}
  >
    <div class="readme-modal" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
      <div class="readme-modal-header">
        <h3>{readmeModal.repoId}</h3>
        <button type="button" class="readme-close" onclick={closeReadmeModal} aria-label="Close">✕</button>
      </div>
      <div class="readme-content">
        {#if readmeModal.loading}
          <p>Loading model card…</p>
        {:else if readmeModal.error}
          <p class="readme-error">{readmeModal.error}</p>
        {:else}
          <!-- The HTML is sanitized via DOMPurify in openReadme() before
               assignment, so {@html} is safe here. -->
          {@html readmeModal.html}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .hf-browser { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 0.6rem; }
  .auth-bar { display: flex; align-items: center; gap: 0.5rem; padding: 0.4rem 0.5rem; background: var(--bg-surface); border-radius: var(--radius-md); border: 1px solid var(--border); }
  .auth-ok { color: var(--success); font-size: 0.78rem; }
  .auth-note { color: var(--text-muted); font-size: 0.7rem; }
  .auth-error-inline { color: var(--danger); font-size: 0.7rem; margin-left: auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .auth-btn { padding: 0.3rem 0.8rem; font-size: 0.75rem; border: 1px solid var(--accent-border); background: var(--accent-bg); color: var(--accent); border-radius: var(--radius-sm); cursor: pointer; }
  .auth-btn:hover { background: var(--accent); color: var(--bg-base); }
  .link-btn { background: none; border: none; color: var(--text-muted); font-size: 0.7rem; cursor: pointer; text-decoration: underline; }
  .link-btn:hover { color: var(--text-secondary); }

  .filters { display: flex; flex-direction: column; gap: 0.4rem; }
  .text-input, .select {
    padding: 0.35rem 0.45rem; font-size: 0.76rem;
    background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: var(--radius-sm); color: var(--text-primary);
  }
  .select-row { display: flex; gap: 0.4rem; align-items: center; flex-wrap: wrap; }
  .select { font-size: 0.72rem; }
  .checkbox { display: flex; align-items: center; gap: 0.25rem; font-size: 0.72rem; color: var(--text-secondary); cursor: pointer; }

  .results { overflow-y: auto; flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 0.25rem; }
  .repo-header {
    display: flex; align-items: center; gap: 0.4rem; width: 100%; text-align: left;
    padding: 0.4rem 0.5rem; background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: var(--radius-sm); cursor: pointer; color: var(--text-primary); font-size: 0.76rem;
  }
  .repo-header:hover { background: var(--bg-elevated); }
  .chevron { color: var(--text-muted); font-size: 0.65rem; flex-shrink: 0; }
  .repo-name { flex: 1; font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .gated-badge { font-size: 0.6rem; color: var(--warning); background: var(--warning-bg); border: 1px solid var(--warning-border); padding: 0 0.25rem; border-radius: var(--radius-sm); flex-shrink: 0; }
  .dl-count { color: var(--text-muted); font-size: 0.66rem; flex-shrink: 0; }
  .pipeline-tag { font-size: 0.6rem; color: var(--accent); background: var(--accent-bg); padding: 0 0.25rem; border-radius: var(--radius-sm); flex-shrink: 0; }
  .repo-files { display: flex; flex-direction: column; gap: 0.25rem; padding: 0.35rem 0 0.35rem 1rem; }
  .repo-meta-bar { font-size: 0.68rem; color: var(--text-muted); font-family: var(--font-mono); padding: 0.15rem 0; }

  .status { padding: 0.4rem; color: var(--text-muted); font-size: 0.74rem; text-align: center; }
  .status.prompt { padding: 2rem 1rem; font-style: italic; }
  .status.error { color: var(--danger); }
  .load-more { align-self: center; padding: 0.3rem 1rem; font-size: 0.72rem; background: var(--bg-surface); border: 1px solid var(--border); color: var(--text-secondary); border-radius: var(--radius-sm); cursor: pointer; }
  .load-more:hover:not(:disabled) { background: var(--bg-elevated); }

  .auth-modal-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.6); display: flex; align-items: center; justify-content: center; z-index: 100; }
  .auth-modal { background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--radius-lg); padding: 1.5rem; max-width: 360px; text-align: center; }
  .auth-modal h3 { margin: 0 0 0.5rem; font-size: 1rem; }
  .auth-modal p { font-size: 0.78rem; color: var(--text-secondary); margin: 0.3rem 0; }
  .user-code { font-family: var(--font-mono); font-size: 1.6rem; letter-spacing: 0.15em; background: var(--bg-elevated); padding: 0.6rem; border-radius: var(--radius-md); color: var(--accent); flex: 1; text-align: center; }
  .user-code-row { display: flex; align-items: stretch; gap: 0.4rem; margin: 0.6rem 0; }
  .copy-code-btn { background: var(--bg-elevated); border: 1px solid var(--border); color: var(--text-secondary); font-size: 1.1rem; padding: 0 0.8rem; border-radius: var(--radius-md); cursor: pointer; flex-shrink: 0; }
  .copy-code-btn:hover { background: var(--bg-surface-2); color: var(--accent); }
  .copied-confirm { color: var(--success); font-size: 0.72rem; margin: 0.2rem 0; }
  .modal-link-btn {
    background: none; border: none; padding: 0; cursor: pointer;
    color: var(--accent); font-family: var(--font-mono); font-size: 0.78rem;
    text-decoration: underline; display: inline;
  }
  .modal-link-btn:hover { color: var(--text-primary); }
  .waiting { color: var(--text-muted); font-style: italic; }
  .auth-error { color: var(--danger); }
  .cancel-auth { margin-top: 0.6rem; padding: 0.3rem 0.8rem; font-size: 0.75rem; background: var(--bg-elevated); border: 1px solid var(--border); color: var(--text-secondary); border-radius: var(--radius-sm); cursor: pointer; }

  /* Model Info button */
  .model-info-btn {
    padding: 0.25rem 0.6rem; font-size: 0.72rem;
    border: 1px solid var(--border); background: var(--bg-surface);
    color: var(--text-secondary); border-radius: var(--radius-sm);
    cursor: pointer; margin-bottom: 0.35rem; align-self: flex-start;
  }
  .model-info-btn:hover { background: var(--bg-elevated); color: var(--text-primary); }

  /* README modal */
  .readme-modal-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.7); display: flex; align-items: center; justify-content: center; z-index: 110; }
  .readme-modal {
    background: var(--bg-base); border: 1px solid var(--border);
    border-radius: var(--radius-lg); width: 90vw; max-width: 720px;
    max-height: 85vh; display: flex; flex-direction: column; overflow: hidden;
  }
  .readme-modal-header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 0.6rem 1rem; border-bottom: 1px solid var(--border);
    background: var(--bg-surface); flex-shrink: 0;
  }
  .readme-modal-header h3 { margin: 0; font-size: 0.85rem; font-family: var(--font-mono); }
  .readme-close { background: none; border: none; color: var(--text-muted); font-size: 1rem; cursor: pointer; padding: 0 0.3rem; }
  .readme-close:hover { color: var(--text-primary); }
  .readme-content { overflow-y: auto; padding: 1rem 1.2rem; color: var(--text-primary); font-size: 0.82rem; line-height: 1.5; }
  .readme-content :global(h1) { font-size: 1.1rem; margin: 0.8rem 0 0.4rem; }
  .readme-content :global(h2) { font-size: 0.98rem; margin: 0.7rem 0 0.35rem; }
  .readme-content :global(h3) { font-size: 0.88rem; margin: 0.6rem 0 0.3rem; }
  .readme-content :global(p) { margin: 0.35rem 0; }
  .readme-content :global(code) { font-family: var(--font-mono); font-size: 0.76rem; background: var(--bg-elevated); padding: 0.05rem 0.25rem; border-radius: 3px; }
  .readme-content :global(pre) { background: var(--bg-surface-2); padding: 0.6rem; border-radius: var(--radius-sm); overflow-x: auto; margin: 0.5rem 0; }
  .readme-content :global(pre code) { background: none; padding: 0; }
  .readme-content :global(table) { border-collapse: collapse; margin: 0.5rem 0; font-size: 0.76rem; width: 100%; }
  .readme-content :global(th), .readme-content :global(td) { border: 1px solid var(--border); padding: 0.25rem 0.4rem; text-align: left; }
  .readme-content :global(th) { background: var(--bg-surface); }
  .readme-content :global(a) { color: var(--accent); }
  .readme-content :global(img) { max-width: 100%; }
  .readme-content :global(ul), .readme-content :global(ol) { padding-left: 1.2rem; margin: 0.35rem 0; }
  .readme-content :global(li) { margin: 0.15rem 0; }
  .readme-content :global(hr) { border: none; border-top: 1px solid var(--border); margin: 0.6rem 0; }
  .readme-content :global(blockquote) { border-left: 3px solid var(--border); padding-left: 0.8rem; color: var(--text-muted); margin: 0.5rem 0; }
  .readme-error { color: var(--danger); }

  /* Pinned active downloads */
  .pinned-downloads {
    background: var(--bg-surface); border: 1px solid var(--accent-border);
    border-radius: var(--radius-md); padding: 0.4rem 0.5rem; margin-bottom: 0.5rem;
  }
  .pinned-header { font-size: 0.65rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); margin-bottom: 0.3rem; }
  .pinned-row { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; padding: 0.25rem 0; }
  .pinned-info { display: flex; flex-direction: column; gap: 0.05rem; min-width: 0; flex: 1; }
  .pinned-filename { font-family: var(--font-mono); font-size: 0.72rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pinned-repo { font-size: 0.64rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pinned-progress { display: flex; align-items: center; gap: 0.35rem; flex-shrink: 0; }
  .pinned-pct { font-size: 0.64rem; color: var(--text-muted); font-variant-numeric: tabular-nums; min-width: 3rem; text-align: right; }
  .pinned-bar { width: 70px; height: 4px; background: var(--bg-surface-2); border-radius: 3px; overflow: hidden; }
  .pinned-fill { height: 100%; background: var(--accent); transition: width 0.3s ease; }
  .pinned-fill.pinned-indeterminate { width: 40%; animation: pinned-pulse 1.5s ease-in-out infinite; }
  @keyframes pinned-pulse { 0% { transform: translateX(-100%); } 100% { transform: translateX(250%); } }
  .pinned-cancel { padding: 0.1rem 0.4rem; font-size: 0.66rem; border: 1px solid var(--danger-border); background: var(--danger-bg); color: var(--danger); border-radius: var(--radius-sm); cursor: pointer; }
</style>
