# Models Directory

Drop your `.gguf` model files in this directory. OmniLauncher discovers them
automatically on startup — no manual registration needed.

## Organization

```
models/
├── my-chat-model.gguf              # Chat models — auto-tagged via chat_template
├── my-embedding-model.gguf         # Embedding models — auto-tagged via pooling_type
└── multimodal/
    └── my-vision-projector.gguf    # Vision projectors — excluded from chat scan
```

## Auto-Detection

Models are automatically tagged as `chat` or `embedding` based on their GGUF
header metadata:

- **Embedding**: has `{arch}.pooling_type` in the header
- **Chat**: has `tokenizer.chat_template` but no pooling_type
- **Untagged**: neither found — appears in both dropdowns

You can override the tag in the UI after the model is discovered.

## Supported Formats

Any `.gguf` file compatible with `llama-server` (llama.cpp b9893+) is supported.
Common architectures: Llama, Qwen, Mistral, Gemma, Phi, DeepSeek, and more.
