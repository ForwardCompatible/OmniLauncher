<script>
  /**
   * Unified model card for both chat and embedding roles. The `role` prop
   * controls: the model-list filter, the launch command's role arg, the header
   * text, and which sections render (Sampling/Generation/Reasoning for chat;
   * Embedding-Specific for embeddings). All shared fields (ctx, flash-attn,
   * VRAM, CPU mode, Performance, Server Config, RoPE) render for both roles.
   *
   * Replaces the former ChatModelCard.svelte + EmbeddingModelCard.svelte which
   * were ~80% duplicated.
   */
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

  let { role } = $props();

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
  let runningProc = $derived(processes.list.find((p) => p.role === role));
  let availableModels = $derived(models.list.filter((m) => m.role === role || m.role === null));
  let selectedModel = $derived(
    selectedModelId !== null ? models.list.find((m) => m.id === selectedModelId) ?? null : null,
  );
  let idSuffix = $derived(role === "chat" ? "chat" : "emb");

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
      await cmd.launchModel(selectedModelId, role);
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
    <h2>{role === "chat" ? "Chat Model" : "Embedding Model"}</h2>
    {#if runningProc}
      <span class="badge running">running :{runningProc.port}</span>
    {/if}
  </header>

  {#if availableModels.length === 0}
    <p class="empty-hint">No {role} models found. Add a <code>.gguf</code> to the models directory.</p>
  {:else}
    <div class="field">
      <label for="model-{idSuffix}">Model</label>
      <select id="model-{idSuffix}" bind:value={selectedModelId} disabled={!!runningProc}>
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
            <label for="ctx-{idSuffix}">Context Size</label>
            {#if selectedModel}<span class="max-hint">(max: {selectedModel.context_length.toLocaleString()})</span>{/if}
            {#if tooltip("--ctx-size")}<span class="info-icon" title={tooltip("--ctx-size")}>ℹ</span>{/if}
          </div>
          <input id="ctx-{idSuffix}" type="number" value={form.ctx_size ?? ""} placeholder="auto (0)"
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

        <!-- Sampling grid (chat only) -->
        {#if role === "chat"}
          <div class="sampling-grid">
            <div class="field">
              <div class="label-row"><span class="field-label">Temperature</span>{#if tooltip("--temp")}<span class="info-icon" title={tooltip("--temp")}>ℹ</span>{/if}</div>
              <input type="number" step="0.05" placeholder="auto (0.80)" value={form.temp ?? ""}
                oninput={(e) => (form.temp = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} />
            </div>
            <div class="field">
              <div class="label-row"><span class="field-label">Top-K</span>{#if tooltip("--top-k")}<span class="info-icon" title={tooltip("--top-k")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto (40)" value={form.top_k ?? ""}
                oninput={(e) => (form.top_k = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} />
            </div>
            <div class="field">
              <div class="label-row"><span class="field-label">Top-P</span>{#if tooltip("--top-p")}<span class="info-icon" title={tooltip("--top-p")}>ℹ</span>{/if}</div>
              <input type="number" step="0.01" placeholder="auto (0.95)" value={form.top_p ?? ""}
                oninput={(e) => (form.top_p = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} />
            </div>
            <div class="field">
              <div class="label-row"><span class="field-label">Min-P</span>{#if tooltip("--min-p")}<span class="info-icon" title={tooltip("--min-p")}>ℹ</span>{/if}</div>
              <input type="number" step="0.01" placeholder="auto (0.05)" value={form.min_p ?? ""}
                oninput={(e) => (form.min_p = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} />
            </div>
            <div class="field">
              <div class="label-row"><span class="field-label">Repeat Penalty</span>{#if tooltip("--repeat-penalty")}<span class="info-icon" title={tooltip("--repeat-penalty")}>ℹ</span>{/if}</div>
              <input type="number" step="0.01" placeholder="auto (1.00)" value={form.repeat_penalty ?? ""}
                oninput={(e) => (form.repeat_penalty = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} />
            </div>
            <div class="field">
              <div class="label-row"><span class="field-label">Repeat Last N</span>{#if tooltip("--repeat-last-n")}<span class="info-icon" title={tooltip("--repeat-last-n")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto (64)" value={form.repeat_last_n ?? ""}
                oninput={(e) => (form.repeat_last_n = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} />
            </div>
            <div class="field">
              <div class="label-row"><span class="field-label">Seed</span>{#if tooltip("--seed")}<span class="info-icon" title={tooltip("--seed")}>ℹ</span>{/if}</div>
              <input type="number" placeholder="auto (-1)" value={form.seed ?? ""}
                oninput={(e) => (form.seed = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} />
            </div>
          </div>
        {/if}

        <!-- Advanced -->
        <details class="advanced">
          <summary>Advanced settings</summary>

          {#if role === "embedding"}
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
                <select value={form.embd_normalize ?? ""} onchange={(e) => (form.embd_normalize = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)}>
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
                <span>Enable Reranking</span>
                {#if tooltip("--rerank")}<span class="info-icon" title={tooltip("--rerank")}>ℹ</span>{/if}
              </label>
            </div>
          {/if}

          {#if role === "chat"}
            <h3 class="advanced-subhead">Generation</h3>
            <div class="advanced-grid">
              <div class="field"><div class="label-row"><span class="field-label">Presence Penalty</span>{#if tooltip("--presence-penalty")}<span class="info-icon" title={tooltip("--presence-penalty")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (0.00)" value={form.presence_penalty ?? ""}
                  oninput={(e) => (form.presence_penalty = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">Frequency Penalty</span>{#if tooltip("--frequency-penalty")}<span class="info-icon" title={tooltip("--frequency-penalty")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (0.00)" value={form.frequency_penalty ?? ""}
                  oninput={(e) => (form.frequency_penalty = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">Typical P</span>{#if tooltip("--typical-p")}<span class="info-icon" title={tooltip("--typical-p")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (1.00)" value={form.typical_p ?? ""}
                  oninput={(e) => (form.typical_p = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">XTC Probability</span>{#if tooltip("--xtc-probability")}<span class="info-icon" title={tooltip("--xtc-probability")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (0.00)" value={form.xtc_probability ?? ""}
                  oninput={(e) => (form.xtc_probability = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">XTC Threshold</span>{#if tooltip("--xtc-threshold")}<span class="info-icon" title={tooltip("--xtc-threshold")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (0.10)" value={form.xtc_threshold ?? ""}
                  oninput={(e) => (form.xtc_threshold = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">Mirostat</span>{#if tooltip("--mirostat")}<span class="info-icon" title={tooltip("--mirostat")}>ℹ</span>{/if}</div>
                <input type="number" placeholder="auto (0)" value={form.mirostat ?? ""}
                  oninput={(e) => (form.mirostat = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">Mirostat LR</span>{#if tooltip("--mirostat-lr")}<span class="info-icon" title={tooltip("--mirostat-lr")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (0.10)" value={form.mirostat_lr ?? ""}
                  oninput={(e) => (form.mirostat_lr = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">Mirostat Entropy</span>{#if tooltip("--mirostat-ent")}<span class="info-icon" title={tooltip("--mirostat-ent")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (5.00)" value={form.mirostat_ent ?? ""}
                  oninput={(e) => (form.mirostat_ent = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">DRY Multiplier</span>{#if tooltip("--dry-multiplier")}<span class="info-icon" title={tooltip("--dry-multiplier")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (0.00)" value={form.dry_multiplier ?? ""}
                  oninput={(e) => (form.dry_multiplier = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">DRY Base</span>{#if tooltip("--dry-base")}<span class="info-icon" title={tooltip("--dry-base")}>ℹ</span>{/if}</div>
                <input type="number" step="0.01" placeholder="auto (1.75)" value={form.dry_base ?? ""}
                  oninput={(e) => (form.dry_base = e.currentTarget.value ? parseFloat(e.currentTarget.value) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">DRY Allowed Length</span>{#if tooltip("--dry-allowed-length")}<span class="info-icon" title={tooltip("--dry-allowed-length")}>ℹ</span>{/if}</div>
                <input type="number" placeholder="auto (2)" value={form.dry_allowed_length ?? ""}
                  oninput={(e) => (form.dry_allowed_length = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
              <div class="field"><div class="label-row"><span class="field-label">Max Tokens</span>{#if tooltip("--predict")}<span class="info-icon" title={tooltip("--predict")}>ℹ</span>{/if}</div>
                <input type="number" placeholder="auto (-1)" value={form.predict ?? ""}
                  oninput={(e) => (form.predict = e.currentTarget.value ? parseInt(e.currentTarget.value, 10) : null)} /></div>
              <label class="checkbox-field advanced-flag">
                <input type="checkbox" checked={form.context_shift ?? false}
                  onchange={(e) => (form.context_shift = e.currentTarget.checked)} />
                <span>Context Shift</span>
                {#if tooltip("--context-shift")}<span class="info-icon" title={tooltip("--context-shift")}>ℹ</span>{/if}
              </label>
            </div>

            <h3 class="advanced-subhead">Reasoning</h3>
            <div class="advanced-grid">
              <div class="field"><div class="label-row"><span class="field-label">Reasoning Format</span>{#if tooltip("--reasoning-format")}<span class="info-icon" title={tooltip("--reasoning-format")}>ℹ</span>{/if}</div>
                <select value={form.reasoning_format ?? ""} onchange={(e) => (form.reasoning_format = e.currentTarget.value || null)}>
                  <option value="">auto</option>
                  <option value="none">none</option>
                  <option value="deepseek">deepseek</option>
                  <option value="deepseek-legacy">deepseek-legacy</option>
                </select></div>
            </div>
          {/if}

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
