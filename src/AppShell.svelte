<script>
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import Loader from "./pages/Loader.svelte";
  import Settings from "./pages/Settings.svelte";
  import NavRail from "./components/NavRail.svelte";
  import {
    errors,
    dismissError,
    clearErrors,
    initAll,
    navigate,
    page,
    processes,
    proxy,
    refreshModels,
    refreshProcesses,
    refreshProxy,
    refreshHardware,
    hardware,
  } from "./lib/stores.svelte.js";

  let railCollapsed = $state(false);

  function toggleRail() {
    railCollapsed = !railCollapsed;
  }

  let runningCount = $derived(processes.list.length);
  let proxyPort = $derived(proxy.data?.master_port ?? "—");

  onMount(async () => {
    await initAll();

    await listen("process-terminated", () => {
      refreshProcesses();
      refreshProxy();
    });
    await listen("hardware-updated", () => {
      refreshHardware();
    });
    await listen("registry-updated", () => {
      refreshModels();
    });
  });
</script>

<div class="shell" class:collapsed={railCollapsed}>
  <header class="topbar">
    <button class="rail-toggle" onclick={toggleRail} title="Toggle navigation">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2">
        <line x1="3" y1="6" x2="21" y2="6" />
        <line x1="3" y1="12" x2="21" y2="12" />
        <line x1="3" y1="18" x2="21" y2="18" />
      </svg>
    </button>
    <span class="app-title">OmniLauncher</span>
    <div class="status-summary">
      {#if runningCount > 0}
        <span class="badge ok">{runningCount} running</span>
      {/if}
      <span class="badge muted">proxy :{proxyPort}</span>
    </div>
  </header>

  <NavRail collapsed={railCollapsed} currentPage={page.current} {navigate} />

  <main class="content">
    {#if errors.items.length > 0}
      <div class="error-queue">
        {#each errors.items as err, i (err.timestamp)}
          <div class="error-row" class:severity-warning={err.severity === "warning"}>
            <span class="error-sev">{err.severity === "warning" ? "⚠" : "✕"}</span>
            <span class="error-msg">{err.message}</span>
            <button class="error-dismiss" onclick={() => dismissError(i)}>✕</button>
          </div>
        {/each}
        {#if errors.items.length > 1}
          <button class="error-clear-all" onclick={() => clearErrors()}>Clear all</button>
        {/if}
      </div>
    {/if}

    {#if page.current === "loader"}
      <Loader />
      {:else if page.current === "settings"}
      <Settings />
    {/if}
  </main>

  <footer class="footer">
    <span>OmniLauncher 0.1.0</span>
    {#if hardware.data}
      <span class="badge muted">
        {hardware.data.gpu_present ? "GPU mode" : "CPU-only"}
      </span>
    {/if}
  </footer>
</div>
