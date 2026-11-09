# 2026-10-30: AI cleanup model pins (0.11.0, Windows download catalogue)

Pinned downloads for the local AI cleanup (llama.cpp, `murmur_llm`) models the
Windows app can download from Settings → AI cleanup (D31 downloader, engine
`llm`). Values come from the Hugging Face API, not from downloading the files:
the revision from `https://huggingface.co/api/models/<repo>/revision/main`
(`sha`), size and SHA-256 from the tree listing at that revision
(`/api/models/<repo>/tree/<revision>`, `lfs.oid` = SHA-256 of the file). A
`HEAD` on the resolve URL agrees (`x-linked-size`, `x-linked-etag`) and
redirects to `us.aws.cdn.hf.co`, which the downloader's redirect policy
already allows (`*.hf.co`). The downloader re-hashes every byte, so a wrong
pin fails closed. Both repos are Apache-2.0 (no attribution required).

URLs are `https://huggingface.co/<repo>/resolve/<revision>/<file>`.

| repo | revision | file | size | SHA-256 |
|---|---|---|---|---|
| `Qwen/Qwen2.5-0.5B-Instruct-GGUF` | `9217f5db79a29953eb74d5343926648285ec7e67` | `qwen2.5-0.5b-instruct-q4_k_m.gguf` | 491400032 | `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` |
| `Qwen/Qwen2.5-1.5B-Instruct-GGUF` | `91cad51170dc346986eccefdc2dd33a9da36ead9` | `qwen2.5-1.5b-instruct-q4_k_m.gguf` | 1117320736 | `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e` |

The 0.5B file is saved under the name the app already looks for when no
cleanup model is set (`DEFAULT_LLM_FILE`, D11), so a download fixes a
"model file not found" status in place. More GGUFs (a benchmark task will
recommend them) are one `CatalogEntry` each in
`apps/murmur-windows/src/model_download.rs` plus a row here.
