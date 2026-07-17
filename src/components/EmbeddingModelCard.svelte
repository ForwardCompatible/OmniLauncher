<script>
  import { CACHE_TYPES } from "../lib/types.js";
  import * as cmd from "../lib/commands.js";
  import {
    pushError,
    flags,
    hardware,
    models,
    processes,
    refreshProcesses,
    refreshProxy,
  } from "../lib/stores.svelte.js";

  let selectedModelId = $state(null);
  let form = $state({
    vram_allocation_mb: null, ctx_size: null, flash_attn: null, cpu_mode: null,
    batch_size: null, ubatch_size: null, threads: null, threads_batch: null,
    mlock: null, no_mmap: null, cache_type_k: null, cache_type_v: null, cache_prompt: null,
    temp: null, top_k: null, top_p: null, min_p: null, repeat_penalty: null,
    repeat_last_n: null, seed: null, presence_penalty: null, frequency_penalty: null,
    typical_p: null, xtc_probability: null, xtc_threshold: null,
    mirostat: null, mirostat_lr: null, mirostat_ent: null,
    dry_multiplier: null, dry_base: null, dry_allowed_length: null,
    predict: null, context_shift: null, parallel: null, cont_batching: null, timeout: null,
    rope_scaling: null, rope_freq_base: null, reasoning_format: null,
    pooling_type_override: null, embd_normalize: null, rerank: null,
  });
  let loadedSnapshot = $state("");
  let loading = $state(false);
  let saving = $state(false);
  let launching = $state(false);
  let savedVram = $state(0);

  let dirty = $derived(JSON.stringify(form) !== loadedSnapshot);
  let maxVram = $derived(hardware.data?.total_vram_mb ?? 8192);
  let runningProc = $derived(processes.list.find((p) => p.role === "embedding"));
  let availableModels = $derived(models.list.filter((m) => m.role === "embedding" || m.role === null));
  let selectedModel = $derived(
    selectedModelId !== null ? models.list.find((m) => m.id === selectedModelId) ?? null : null,
  );

  function tooltip(cliArg) {
    return flags.map.get(cliArg)?.description ?? "";
  }

  async function loadSettings(modelId) {
    loading = true;
    try {
      const s = await cmd.getModelSettings(modelId);
      form = { ...s };
      loadedSnapshot = JSON.stringify(s);
      savedVram = form.vram_allocation_mb ?? 0;
    } catch (e) {
      pushError(String(e), "error");
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (selectedModelId !== null) loadSettings(selectedModelId);
  });

  function toggleCpuMode(checked) {
    form.cpu_mode = checked;
    if (checked) {
      savedVram = form.vram_allocation_mb ?? 0;
      form.vram_allocation_mb = 0;
    } else {
      form.vram_allocation_mb = savedVram > 0 ? savedVram : Math.floor(maxVram * 0.8);
    }
  }

  async function handleSave() {
    if (selectedModelId === null) return;
    saving = true;
    try {
      await cmd.saveModelSettings(selectedModelId, form);
      loadedSnapshot = JSON.stringify(form);
    } catch (e) {
      pushError(String(e), "error");
    } finally {
      saving = false;
    }
  }

  async function handleLaunch() {
    if (selectedModelId === null) return;
    launching = true;
    try {
      await cmd.saveModelSettings(selectedModelId, form);
      loadedSnapshot = JSON.stringify(form);
      await cmd.launchModel(selectedModelId, "embedding");
      await Promise.all([refreshProcesses(), refreshProxy()]);
    } catch (e) {
      pushError(String(e), "error");
    } finally {
      launching = false;
    }
  }

  async function handleStop() {
    if (selectedModelId === null) return;
    try {
      await cmd.stopModel(selectedModelId);
      await Promise.all([refreshProcesses(), refreshProxy()]);
    } catch (e) {
      pushError(String(e), "error");
    }
  }
</script>

<section class="model-card">
  <header>
    <h2>Embedding Model</h2>
    {#if runningProc}
      <span class="badge running">running :{runningProc.port}</span>
    {/if}
  </header>

  {#if availableModels.length === 0}
    <p class="empty-hint">No embedding models found. Add a <code>.gguf</code> to the models directory.</p>
  {:else}
    <div class="field">
      <label for="model-emb">Model</label>
      <select id="model-emb" bind:value={selectedModelId} disabled={!!runningProc}>
        <option value={null}>— Select —</option>
        {#each availableModels as m (m.id)}
          <option value={m.id}>{m.model_name} ({m.quantization}, {m.architecture})</option>
        {/each}
      </select>
    </div>

    {#if selectedModelId !== null}
      {#if loading}
        <p class="muted">Loading settings…</p>
      {:else}
        <!-- Primary controls -->
        <div class="field">
          <div class="label-row">
            <label for="ctx-emb">Context Size</label>
            {#if selectedModel}<span class="max-hint">(max: {selectedModel.context_length.toLocaleString()})</span>{/if}
            {#if tooltip("--ctx-size")}<span class="info-icon" title={tooltip("--ctx-size")}>ℹ</span>{/if}
          </div>
          <input id="ctx-emb" type="number" value={form.ctx_size ?? ""} placeholder="auto (0)"
            oninput={(e) => (form.ctx_size = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} />
        </div>

        <label class="checkbox-field">
          <input type="checkbox" checked={form.flash_attn ?? false}
            onchange={(e) => (form.flash_attn = e.currentTarget.checked)} />
          <span>Flash Attention</span>
          {#if tooltip("--flash-attn")}<span class="info-icon" title={tooltip("--flash-attn")}>ℹ</span>{/if}
        </label>

        <div class="slider-field">
          <div class="slider-header">
            <span class="field-label">VRAM Allocation</span>
            <span class="slider-value">{form.vram_allocation_mb ?? 0} / {maxVram} MiB</span>
          </div>
          <input type="range" min="0" max={maxVram} step="64" disabled={form.cpu_mode ?? false}
            value={form.vram_allocation_mb ?? 0}
            oninput={(e) => (form.vram_allocation_mb = parseInt(e.currentTarget.value, 10))} />
          <label class="checkbox-field cpu-toggle">
            <input type="checkbox" checked={form.cpu_mode ?? false}
              onchange={(e) => toggleCpuMode(e.currentTarget.checked)} />
            <span>CPU Mode</span>
            {#if tooltip("--n-gpu-layers")}<span class="info-icon" title={tooltip("--n-gpu-layers")}>ℹ</span>{/if}
          </label>
        </div>

        <!-- Advanced -->
        <details class="advanced">
          <summary>Advanced settings</summary>

          <h3 class="advanced-subhead">Embedding-Specific</h3>
          <div class="advanced-grid">
            <div class="field"><div class="label-row"><span class="field-label">Pooling Type</span>{#if tooltip("--pooling")}<span class="info-icon" title={tooltip("--pooling")}>ℹ</span>{/if}</div>
              <select value={form.pooling_type_override ?? ""} onchange={(e) => (form.pooling_type_override = e.currentTarget.value || null)}>
                <option value="">Model default</option>
                <option value="none">none</option>
                <option value="mean">mean</option>
                <option value="cls">cls</option>
                <option value="last">last</option>
                <option value="rank">rank</option>
              </select></div>
            <div class="field"><div class="label-row"><span class="field-label">Embedding Normalization</span>{#if tooltip("--embd-normalize")}<span class="info-icon" title={tooltip("--embd-normalize")}>ℹ</span>{/if}</div>
              <select value={form.embd_normalize?.toString() ?? ""} onchange={(e) => (form.embd_normalize = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)}>
                <option value="">Default (2)</option>
                <option value="-1">None (-1)</option>
                <option value="0">Max absolute int16 (0)</option>
                <option value="1">Taxicab (1)</option>
                <option value="2">Euclidean (2)</option>
                <option value="3">P-norm (3)</option>
                <option value="4">P-norm (4)</option>
              </select></div>
            <label class="checkbox-field advanced-flag">
              <input type="checkbox" checked={form.rerank ?? false}
                onchange={(e) => (form.rerank = e.currentTarget.checked)} />
              <span>Enable Reranking (--rerank)</span>
              {#if tooltip("--rerank")}<span class="info-icon" title={tooltip("--rerank")}>ℹ</span>{/if}
            </label>
          </div>

          <h3 class="advanced-subhead">Performance</h3>
          <div class="advanced-grid">
            <div class="field"><div class="label-row"><span class="field-label">Generation Threads</span>{#if tooltip("--threads")}<span class="info-icon" title={tooltip("--threads")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto" value={form.threads ?? ""}
                oninput={(e) => (form.threads = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
            <div class="field"><div class="label-row"><span class="field-label">Prompt Threads</span>{#if tooltip("--threads-batch")}<span class="info-icon" title={tooltip("--threads-batch")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto" value={form.threads_batch ?? ""}
                oninput={(e) => (form.threads_batch = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
            <div class="field"><div class="label-row"><span class="field-label">Batch Size</span>{#if tooltip("--batch-size")}<span class="info-icon" title={tooltip("--batch-size")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto (2048)" value={form.batch_size ?? ""}
                oninput={(e) => (form.batch_size = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
            <div class="field"><div class="label-row"><span class="field-label">Ubatch Size</span>{#if tooltip("--ubatch-size")}<span class="info-icon" title={tooltip("--ubatch-size")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto (512)" value={form.ubatch_size ?? ""}
                oninput={(e) => (form.ubatch_size = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
            <div class="field"><div class="label-row"><span class="field-label">K-Cache Type</span>{#if tooltip("--cache-type-k")}<span class="info-icon" title={tooltip("--cache-type-k")}>ℹ</span>{/if}</div>
              <select value={form.cache_type_k ?? "f16"} onchange={(e) => (form.cache_type_k = e.currentTarget.value)}>
                {#each CACHE_TYPES as ct}<option value={ct}>{ct}</option>{/each}</select></div>
            <div class="field"><div class="label-row"><span class="field-label">V-Cache Type</span>{#if tooltip("--cache-type-v")}<span class="info-icon" title={tooltip("--cache-type-v")}>ℹ</span>{/if}</div>
              <select value={form.cache_type_v ?? "f16"} onchange={(e) => (form.cache_type_v = e.currentTarget.value)}>
                {#each CACHE_TYPES as ct}<option value={ct}>{ct}</option>{/each}</select></div>
            <label class="checkbox-field advanced-flag">
              <input type="checkbox" checked={form.cache_prompt ?? false}
                onchange={(e) => (form.cache_prompt = e.currentTarget.checked)} />
              <span>Prompt Caching</span>
              {#if tooltip("--cache-prompt")}<span class="info-icon" title={tooltip("--cache-prompt")}>ℹ</span>{/if}
            </label>
            <label class="checkbox-field advanced-flag">
              <input type="checkbox" checked={form.mlock ?? false}
                onchange={(e) => (form.mlock = e.currentTarget.checked)} />
              <span>Memory Lock (--mlock)</span>
              {#if tooltip("--mlock")}<span class="info-icon" title={tooltip("--mlock")}>ℹ</span>{/if}
            </label>
            <label class="checkbox-field advanced-flag">
              <input type="checkbox" checked={form.no_mmap ?? false}
                onchange={(e) => (form.no_mmap = e.currentTarget.checked)} />
              <span>Disable Memory Mapping (--no-mmap)</span>
              {#if tooltip("--no-mmap")}<span class="info-icon" title={tooltip("--no-mmap")}>ℹ</span>{/if}
            </label>
          </div>

          <h3 class="advanced-subhead">Server Config</h3>
          <div class="advanced-grid">
            <div class="field"><div class="label-row"><span class="field-label">Parallel Slots</span>{#if tooltip("--parallel")}<span class="info-icon" title={tooltip("--parallel")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto (-1)" value={form.parallel ?? ""}
                oninput={(e) => (form.parallel = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
            <div class="field"><div class="label-row"><span class="field-label">Timeout</span>{#if tooltip("--timeout")}<span class="info-icon" title={tooltip("--timeout")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto (3600)" value={form.timeout ?? ""}
                oninput={(e) => (form.timeout = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
            <label class="checkbox-field advanced-flag">
              <input type="checkbox" checked={form.cont_batching ?? false}
                onchange={(e) => (form.cont_batching = e.currentTarget.checked)} />
              <span>Continuous Batching</span>
              {#if tooltip("--cont-batching")}<span class="info-icon" title={tooltip("--cont-batching")}>ℹ</span>{/if}
            </label>
          </div>

          <h3 class="advanced-subhead">RoPE / Context Extension</h3>
          <div class="advanced-grid">
            <div class="field"><div class="label-row"><span class="field-label">RoPE Scaling</span>{#if tooltip("--rope-scaling")}<span class="info-icon" title={tooltip("--rope-scaling")}>ℹ</span>{/if}</div>
              <select value={form.rope_scaling ?? ""} onchange={(e) => (form.rope_scaling = e.currentTarget.value || null)}>
                <option value="">auto (linear)</option>
                <option value="none">none</option>
                <option value="linear">linear</option>
                <option value="yarn">yarn</option>
              </select></div>
            <div class="field"><div class="label-row"><span class="field-label">RoPE Base Frequency</span>{#if tooltip("--rope-freq-base")}<span class="info-icon" title={tooltip("--rope-freq-base")}>ℹ</span>{/if}</div>
              <input type="number" step="0.01" placeholder="auto (from model)" value={form.rope_freq_base ?? ""}
                oninput={(e) => (form.rope_freq_base = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
          </div>
        </details>

        <div class="card-actions">
          <button class="btn save" disabled={!dirty || saving} onclick={handleSave}>{saving ? "Saving…" : "Save Settings"}</button>
          {#if runningProc}
            <button class="btn stop" onclick={handleStop}>Stop</button>
          {:else}
            <button class="btn launch" disabled={launching} onclick={handleLaunch}>{launching ? "Starting…" : "Launch"}</button>
          {/if}
        </div>
      {/if}
    {/if}
  {/if}
</section>
