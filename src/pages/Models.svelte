<script>
  /**
   * Models page — vertical split: HuggingFace browser (left) + local library
   * (right). Owns the download event listeners that update the downloads map
   * and refresh the local registry on completion.
   *
   * CRITICAL: the previous Browse.svelte had a broken onMount whose listener
   * cleanup (`u.then((fn) => fn())`) could leak and a self-triggering `$effect`
   * that crashed AppShell. Here listeners are awaited individually and the
   * cleanup unlisten functions are collected correctly.
   */
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import HfBrowser from "../components/HfBrowser.svelte";
  import LocalLibrary from "../components/LocalLibrary.svelte";
  import { hfDownloads, setHfDownload, models, refreshModels, pushError } from "../lib/stores.svelte.js";

  onMount(() => {
    /** @type {Array<() => void>} */
    const unlistenFns = [];
    const registrations = [
      listen("download-progress", (e) => {
        const p = /** @type {any} */ (e.payload);
        // Scan the shared downloads map for the entry matching this download id.
        // The id is stored on each entry by handleDownload() in HfBrowser.svelte,
        // so we don't need a separate lookup map.
        for (const [key, entry] of Object.entries(hfDownloads.map)) {
          if (entry.id === p.id) {
            setHfDownload(key, {
              ...entry,
              downloaded: p.downloaded_bytes,
              total: p.total_bytes ?? null,
              status: "active",
            });
            break;
          }
        }
      }),
      listen("download-completed", (e) => {
        const p = /** @type {any} */ (e.payload);
        const dlKey = `${p.repo_id}/${p.filename}`;
        setHfDownload(dlKey, { id: p.id, downloaded: 0, total: null, status: "completed" });
        refreshModels(); // show in the right pane + Loader dropdown
      }),
      listen("download-failed", (e) => {
        const p = /** @type {any} */ (e.payload);
        const dlKey = `${p.repo_id}/${p.filename}`;
        setHfDownload(dlKey, { id: p.id, downloaded: 0, total: null, status: "failed", message: p.message });
        pushError(`Download failed (${p.filename}): ${p.message}`, "warning");
      }),
    ];
    Promise.all(registrations).then((fns) => unlistenFns.push(...fns));
    return () => unlistenFns.forEach((fn) => fn());
  });
</script>

<div class="models-page">
  <h1>Models</h1>
  <div class="split">
    <div class="pane left"><HfBrowser /></div>
    <div class="pane right"><LocalLibrary /></div>
  </div>
</div>

<style>
  .models-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  h1 { font-size: 1.2rem; margin: 0 0 0.5rem; }
  .split {
    display: flex; gap: 1rem; flex: 1; min-height: 0;
  }
  .pane {
    flex: 1; min-width: 0; display: flex; flex-direction: column;
    background: var(--bg-base);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 0.75rem;
    overflow: hidden;
  }
</style>
