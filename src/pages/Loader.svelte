<script>
  import ModelCard from "../components/ModelCard.svelte";
</script>

<div class="loader-page">
  <h1>Loader</h1>
  <p class="page-desc">
    Configure and launch your chat and embedding models. Each card manages an
    independent backend — point external tools at the master port and the proxy
    routes by path.
  </p>

  <div class="cards-grid">
    <ModelCard role="chat" />
    <ModelCard role="embedding" />
  </div>
</div>
