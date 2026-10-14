"""Convert pinned potion-code-16M-v2 artifacts into the vendored blobs used
by src/static_engine.rs, and regenerate the accent table section.

Usage: place model.safetensors + tokenizer.json (revision
e9d2a44ca6a05ac6685f3b23709ea57eb7352d5b, hashes in README.md) in a
directory, then: python3 convert_potion.py <src_dir> <assets_dir>
"""
import hashlib
import json
import os
import struct
import sys
import unicodedata


def main() -> None:
    src, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    data = open(os.path.join(src, "model.safetensors"), "rb").read()
    hlen = struct.unpack("<Q", data[:8])[0]
    hdr = json.loads(data[8 : 8 + hlen])
    t = hdr["embeddings"]
    assert t["dtype"] == "F16" and t["shape"] == [63457, 256], t
    off = 8 + hlen
    a, b = t["data_offsets"]
    blob = data[off + a : off + b]
    assert len(blob) == 63457 * 256 * 2
    open(os.path.join(out, "potion-code-16m-v2.f16.bin"), "wb").write(blob)
    print("weights", len(blob), hashlib.sha256(blob).hexdigest())

    vocab = json.load(open(os.path.join(src, "tokenizer.json")))["model"]["vocab"]
    items = sorted(vocab.items(), key=lambda kv: kv[0].encode())
    concat = b"".join(s.encode() for s, _ in items)
    offsets, pos = [], 0
    for s, _ in items:
        offsets.append(pos)
        pos += len(s.encode())
    offsets.append(pos)
    vb = (
        struct.pack("<I", len(items))
        + struct.pack(f"<{len(offsets)}I", *offsets)
        + struct.pack(f"<{len(items)}I", *[i for _, i in items])
        + concat
    )
    open(os.path.join(out, "potion-code-16m-v2.vocab.bin"), "wb").write(vb)
    print("vocab", len(vb), hashlib.sha256(vb).hexdigest())

    # Accent table: BMP chars whose NFD is base + combining marks (Mn) only.
    entries = []
    for cp in range(0x10000):
        ch = chr(cp)
        d = unicodedata.normalize("NFD", ch)
        if len(d) > 1 and d[0] != ch and all(
            unicodedata.category(c) == "Mn" for c in d[1:]
        ):
            entries.append((cp, ord(d[0])))
    lines = []
    for i in range(0, len(entries), 8):
        chunk = ", ".join(f"(0x{x:04x},0x{y:04x})" for x, y in entries[i : i + 8])
        lines.append("    " + chunk + ",")
    body = (
        "//! Generated: Unicode chars whose NFD decomposition is a base char followed\n"
        "//! only by combining marks (category Mn), mapped to that base char. Used to\n"
        "//! strip accents the way BERT's normalizer does (NFD + drop Mn) without a\n"
        "//! unicode-normalization dependency. Regenerate with\n"
        "//! `assets/convert_potion.py` (table section) if Unicode data changes.\n"
        "/// Sorted by codepoint; binary-searchable.\n"
        "pub static ACCENT_BASE: &[(u32, u32)] = &[\n"
        + "\n".join(lines)
        + "\n];\n"
    )
    print("accent entries", len(entries), "-- write body to src/accent_table.rs")


if __name__ == "__main__":
    main()
