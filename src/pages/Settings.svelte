<script>
  import * as cmd from "../lib/commands.js";
  import {
    pushError,
    hardware,
    refreshHardware,
    refreshModels,
    settings,
  } from "../lib/stores.svelte.js";

  let modelsDir = $state("");
  let multimodalDir = $state("");
  let masterPort = $state(52715);
  let autoIncrement = $state(true);

  let saving = $state(false);
  let saved = $state(false);
  let rescanning = $state(false);
  let rescanResult = $state(null);

  $effect(() => {
    if (settings.data) {
      modelsDir = settings.data.models_directory ?? "";
      multimodalDir = settings.data.multimodal_directory ?? "";
      masterPort = settings.data.master_port;
      autoIncrement = settings.data.auto_port_increment;
    }
  });

  async function handleSave() {
    saving = true;
    saved = false;
    try {
      await cmd.saveAppSettings({
        models_directory: modelsDir || null,
        multimodal_directory: multimodalDir || null,
        master_port: masterPort,
        auto_port_increment: autoIncrement,
      });
      settings.data = await cmd.getAppSettings();
      saved = true;
      setTimeout(() => (saved = false), 2000);
    } catch (e) {
      pushError(String(e), "error");
    } finally {
      saving = false;
    }
  }

  async function handleRescan() {
    rescanning = true;
    rescanResult = null;
    try {
      await cmd.rescanHardware();
      await cmd.resyncRegistry();
      await Promise.all([refreshHardware(), refreshModels()]);
      rescanResult = "Hardware + registry rescan complete";
    } catch (e) {
      pushError(String(e), "warning");
    } finally {
      rescanning = false;
    }
  }
</script>

<div class="settings-page">
  <h1>Settings</h1>

  <section class="settings-section">
    <h2>Paths</h2>
    <div class="field">
      <label for="models-dir">Models Directory</label>
      <input id="models-dir" type="text" bind:value={modelsDir} placeholder="/path/to/models" />
    </div>
    <div class="field">
      <label for="multimodal-dir">Multimodal Directory</label>
      <input id="multimodal-dir" type="text" bind:value={multimodalDir} placeholder="/path/to/multimodal" />
    </div>
  </section>

  <section class="settings-section">
    <h2>Network</h2>
    <div class="field-row">
      <label class="checkbox-field">
        <input type="checkbox" bind:checked={autoIncrement} />
        <span>Auto Port Increment</span>
      </label>
    </div>
    <div class="field">
      <label for="master-port">Master Port</label>
      <input id="master-port" type="number" bind:value={masterPort} min="1024" max="65535" />
      <p class="hint">Takes effect on next app restart. The proxy binds at startup.</p>
    </div>
  </section>

  <section class="settings-section">
    <h2>Hardware</h2>
    {#if hardware.data}
      <div class="hw-info">
        <div class="hw-row"><span>GPU</span><strong>{hardware.data.gpu_name}</strong></div>
        <div class="hw-row"><span>VRAM</span><strong>{hardware.data.total_vram_mb.toLocaleString()} MiB</strong></div>
        <div class="hw-row"><span>System RAM</span><strong>{hardware.data.total_system_ram_mb.toLocaleString()} MiB</strong></div>
        <div class="hw-row"><span>CPU</span><strong>{hardware.data.cpu_physical_cores} phys / {hardware.data.cpu_logical_threads} logical</strong></div>
        <div class="hw-row"><span>Last scanned</span><strong>{hardware.data.last_scanned_at}</strong></div>
        <div class="hw-row">
          <span>Mode</span>
          {#if hardware.data.gpu_present}
            <span class="badge ok">GPU · --fit armed</span>
          {:else}
            <span class="badge cpu">CPU-only · safety valve</span>
          {/if}
        </div>
      </div>
    {:else}
      <p class="muted">No hardware profile yet.</p>
    {/if}
    <button class="btn" disabled={rescanning} onclick={handleRescan}>
      {rescanning ? "Scanning…" : "Manual Rescan"}
    </button>
    {#if rescanResult}
      <p class="rescan-result">{rescanResult}</p>
    {/if}
  </section>

  <div class="settings-actions">
    <button class="btn save" disabled={saving} onclick={handleSave}>
      {saving ? "Saving…" : "Save Settings"}
    </button>
    {#if saved}<span class="badge ok">Saved</span>{/if}
  </div>
</div>
