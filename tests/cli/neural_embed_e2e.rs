//! Mock-free neural embedding E2E using a pinned, pre-provisioned ONNX model.

use ast_sgrep_core::store::IndexStore;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MODEL_REVISION: &str = "751bff37182d3f1213fa05d7196b954e230abad9";
const MODEL_REPO_DIR: &str = "models--Xenova--all-MiniLM-L6-v2";

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn pinned_cache() -> PathBuf {
    let configured = std::env::var_os("ASGREP_NEURAL_E2E_CACHE_DIR")
        .map(PathBuf::from)
        .expect(
            "ASGREP_NEURAL_E2E_CACHE_DIR must name the cache created by \
             scripts/fetch-neural-e2e-model",
        );
    let cache = if configured.is_absolute() {
        configured
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(configured)
    };
    let repo = cache.join(MODEL_REPO_DIR);
    assert_eq!(
        fs::read_to_string(repo.join("refs/main"))
            .expect("pinned neural model cache must contain refs/main"),
        MODEL_REVISION,
        "neural E2E refuses an unpinned model revision"
    );
    for file in [
        "onnx/model_quantized.onnx",
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ] {
        assert!(
            repo.join("snapshots")
                .join(MODEL_REVISION)
                .join(file)
                .is_file(),
            "pinned neural model cache is incomplete: missing {file}"
        );
    }
    cache
}

fn run(bin: &Path, cache: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env("NO_COLOR", "1")
        .env("ASGREP_NEURAL_EMBED", "1")
        .env("ASGREP_NEURAL_MODEL", "all-minilm-l6-v2-q")
        .env("ASGREP_NEURAL_CACHE_DIR", cache)
        .env("ASGREP_NEURAL_INTRA_THREADS", "1")
        // Any cache miss must fail instead of silently downloading a moving model.
        .env("HF_ENDPOINT", "http://127.0.0.1:9")
        .env_remove("HF_HOME")
        .env_remove("ASGREP_NEURAL_FALLBACK")
        .env_remove("ASGREP_SEMANTIC_ONLY")
        .output()
        .expect("run feature-gated asgrep")
}

fn success_json(output: &Output, command: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{command} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{command} stdout is not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(value["ok"], true, "{command} response: {value}");
    assert_eq!(value["command"], command);
    value
}

#[test]
fn real_model_indexes_and_searches_embedding_hits() {
    let cache = pinned_cache();
    let fixture = tempfile::tempdir().expect("fixture tempdir");
    let index_dir = tempfile::tempdir().expect("index tempdir");
    fs::write(
        fixture.path().join("credentials.rs"),
        "/// Renew an expired access credential and rotate its token.\n\
         pub fn renew_expired_credential(account: &mut Account) {\n\
             account.rotate_access_token();\n\
         }\n",
    )
    .expect("write real source fixture");

    let bin = asgrep_bin();
    let index_path = index_dir.path().join("neural.db");
    let index = index_path.to_str().expect("index path utf8");
    let root = fixture.path().to_str().expect("fixture path utf8");

    let indexed = success_json(
        &run(
            &bin,
            &cache,
            &[
                "--json",
                "--neural-embed",
                "--index-path",
                index,
                "index",
                root,
            ],
        ),
        "index",
    );
    assert_eq!(indexed["files_indexed"], 1, "index response: {indexed}");

    let status = success_json(
        &run(
            &bin,
            &cache,
            &["--json", "--index-path", index, "status", root],
        ),
        "status",
    );
    assert_eq!(status["embed_backend"], "neural", "status: {status}");
    assert_eq!(status["embed_dim"], 384, "status: {status}");
    assert!(
        status["semantic_chunk_count"].as_u64().unwrap_or(0) > 0,
        "neural index must contain semantic chunks: {status}"
    );

    let store = IndexStore::open(fixture.path(), Some(&index_path)).expect("open real index");
    assert_eq!(
        store.get_meta("embed_model").expect("read model metadata"),
        Some("neural:all-minilm-l6-v2-q".to_owned())
    );
    drop(store);

    let searched = success_json(
        &run(
            &bin,
            &cache,
            &[
                "--json",
                "--neural-embed",
                "--index-path",
                index,
                "--limit",
                "8",
                "semantic",
                "--",
                "renew an expired authentication credential",
                root,
            ],
        ),
        "semantic",
    );
    let hits = searched["hits"].as_array().expect("semantic hits array");
    assert!(
        !hits.is_empty(),
        "real neural search returned no hits: {searched}"
    );
    assert!(
        hits.iter().any(|hit| {
            hit["kind"].as_str() == Some("embed")
                && hit["symbol"].as_str() == Some("renew_expired_credential")
        }),
        "real neural search must return the fixture symbol as an embed hit: {searched}"
    );

    println!("index={indexed}");
    println!("status={status}");
    println!("search={searched}");
}
