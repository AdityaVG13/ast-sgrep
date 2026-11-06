use super::IndexStore;
use crate::Result;

impl IndexStore {
    pub fn index_data_version(&self) -> Result<i64> {
        // The meta row is authoritative for both same-connection and foreign
        // commits. No memo: the bundle header or one meta read is enough.
        Ok(self
            .get_meta("index_data_version")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    /// Indexed-content and lexicon generations used by long-lived search caches.
    pub(crate) fn search_data_versions(&self) -> Result<(i64, i64)> {
        Ok(self.conn.query_row(
            "SELECT
               COALESCE(MAX(CASE WHEN key = 'index_data_version' THEN CAST(value AS INTEGER) END), 0),
               COALESCE(MAX(CASE WHEN key = 'lexicon_data_version' THEN CAST(value AS INTEGER) END), 0)
             FROM meta
             WHERE key IN ('index_data_version', 'lexicon_data_version')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }
    /// True when the index still stores the unversioned `"semantic"` backend.
    /// Search refuses that meta; indexing must rewrite every chunk before
    /// promoting to the current `"semantic-v2"` identity.
    pub fn needs_legacy_semantic_rewrite(&self) -> Result<bool> {
        Ok(self.get_meta("embed_backend")?.as_deref() == Some("semantic"))
    }
    /// Start a proven-complete semantic rewrite inside the caller's bulk
    /// transaction. Old vectors are removed before the first upsert so that
    /// its resolved provider/model can establish the store-wide identity;
    /// every later upsert must match it or the bulk transaction rolls back.
    /// Embed-cache rows for any other model are dead weight after the
    /// rewrite (the cache is keyed by model id) and are purged here.
    pub(crate) fn reset_semantic_index_for_rewrite(&self, keep_model_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM semantic_chunks", [])?;
        self.conn.execute("DELETE FROM embeddings", [])?;
        self.conn.execute(
            "DELETE FROM embed_cache WHERE model_id != ?1",
            rusqlite::params![keep_model_id],
        )?;
        for key in [
            "embed_backend",
            "embed_backend_pref",
            "embed_model",
            "embed_dim",
        ] {
            self.delete_meta(key)?;
        }
        crate::semantic_ann::drop_semantic_ivf(self)?;
        self.bump_semantic_data_version()
    }
    pub(crate) fn bump_index_data_version(&self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('index_data_version', '1')              ON CONFLICT(key) DO UPDATE SET value =              CAST(COALESCE(meta.value, '0') AS INTEGER) + 1",
            [],
        )?;
        self.line_corpus.borrow_mut().take();
        self.bundle.borrow_mut().take();
        self.bundle_file_stamp.set(None);
        self.lazy_ivf.borrow_mut().take();
        self.line_corpus_disabled.set(false);
        Ok(())
    }
    /// Monotonic counter bumped on every semantic_chunks mutation (insert or delete).
    /// Used by SemanticCache and the IVF fingerprint to detect delete+re-add
    /// collisions where max_id is reused but chunk content/vectors differ.
    pub fn semantic_data_version(&self) -> Result<i64> {
        Ok(self
            .get_meta("semantic_data_version")?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0))
    }
    pub fn bump_semantic_data_version(&self) -> Result<()> {
        let v = self.semantic_data_version()?.saturating_add(1);
        self.set_meta("semantic_data_version", &v.to_string())
    }
}
