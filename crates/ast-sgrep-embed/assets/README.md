# Vendored static-embedding assets

Placeholder weights for the native static embedding engine
(`src/static_engine.rs`), baked into the binary with `include_bytes!`.
Phase 3 replaces these borrowed weights with our own distilled table
behind the same engine.

## Provenance

- Upstream: `minishlab/potion-code-16M-v2` (Minish Lab, Model2Vec)
- License: MIT
- Revision: `e9d2a44ca6a05ac6685f3b23709ea57eb7352d5b`
- Source files (HuggingFace, pinned revision):
  - `model.safetensors` — 32,490,072 bytes,
    sha256 `75cf7a6c2171b230ad19b1e7d8e0b1aee86da5a02af8e7cacedd9921d227623c`
  - `tokenizer.json` — 1,024,340 bytes,
    sha256 `107bbdcbad4bff1d299b7a4c3a2fb17c52890688b7dd0e4c9deab79d3c4f3d45`

## Derived files (produced by `convert_potion.py`)

- `potion-code-16m-v2.f16.bin` — raw `embeddings` tensor bytes,
  63,457 x 256 f16 little-endian, row-major by token id (32,489,984 bytes,
  sha256 `fff14ae49c5343920aaafbeb9914bce7c831da550828e9f0217a387289b10421`).
- `potion-code-16m-v2.vocab.bin` — WordPiece vocab sorted by byte order:
  `[u32 n][u32 offsets (n+1)][u32 ids (n)][token bytes]` (971,153 bytes,
  sha256 `29190a78cda9a47a515f66c48a3d27e8eadd58003d3641bf92926afb4a09c0b2`).

Tokenizer config from `tokenizer.json`: BertNormalizer (clean_text,
handle_chinese_chars, lowercase, strip_accents follows lowercase),
BertPreTokenizer, WordPiece model (`##` continuations, `[UNK]` id 1,
max 100 chars per word). `config.json`: normalize=true, float16.
