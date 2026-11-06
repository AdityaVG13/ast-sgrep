use super::super::sql::SCHEMA_DDL;
use super::{IndexStore, SCHEMA_VERSION};
use crate::Result;
use rusqlite::Connection;

/// Add `files.depth_truncated` when missing (idempotent: fresh dbs already
/// have it via SCHEMA_DDL; pre-16 dbs get the DEFAULT-0 column here).
fn ensure_depth_truncated_column(conn: &Connection) -> Result<()> {
    let existing: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(files)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    if !existing.iter().any(|name| name == "depth_truncated") {
        conn.execute(
            "ALTER TABLE files ADD COLUMN depth_truncated INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn ensure_semantic_field_vector_columns(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(semantic_chunks)")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for column in [
        "vector_name",
        "vector_docs",
        "vector_body",
        "vector_graph",
        "vector_tests_examples",
    ] {
        if !existing.iter().any(|name| name == column) {
            conn.execute(
                &format!("ALTER TABLE semantic_chunks ADD COLUMN {column} BLOB"),
                [],
            )?;
        }
    }
    Ok(())
}

fn ensure_scip_facts_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scip_facts (
            file_id INTEGER NOT NULL, line_no INTEGER NOT NULL, name TEXT NOT NULL,
            is_def INTEGER NOT NULL, PRIMARY KEY (file_id, line_no, name, is_def),
            FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);
         CREATE INDEX IF NOT EXISTS idx_scip_facts_file_id ON scip_facts(file_id);",
    )?;
    Ok(())
}

impl IndexStore {
    pub(super) fn init_schema(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(crate::StoreError::schema_newer_than_binary(
                version,
                SCHEMA_VERSION,
            ));
        }
        if self.read_only {
            return self.init_schema_readonly(version);
        }
        if version >= SCHEMA_VERSION {
            // Probe core tables even when user_version is current (a639).
            let core: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('files','lines','meta','symbols')",
                [],
                |r| r.get(0),
            )?;
            if core >= 4 {
                return Ok(());
            }
            // Corrupt/partial schema with current user_version — rebuild.
        }
        // A legacy semantic sidecar is only a derived acceleration structure.
        // Remove it before migration; if the DB transaction then rolls back,
        // searches safely fall back rather than observing stale ANN contents.
        if version < 12 {
            crate::semantic_ivf::invalidate_semantic_ivf(&self.db_path)?;
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let drops_fts_shadows = version < 18;
        let migration = (|| -> Result<()> {
            self.conn.execute_batch(SCHEMA_DDL)?;
            if version < 12 {
                ensure_semantic_field_vector_columns(&self.conn)?;
            }
            if version < 11 {
                ensure_scip_facts_table(&self.conn)?;
            }
            if version < 13 {
                // gauntlet-r11: backfill the lower() expression indexes for
                // existing indexes. SCHEMA_DDL above already carries them via
                // IF NOT EXISTS, but only a version bump guarantees the DDL
                // re-runs on stores that never re-open through a rebuild.
            }
            if version < 15 {
                // The php `::` re-key changed the meaning of `call:`
                // signature rows without bumping the
                // schema version, so a pre-rekey index still holds `call:bar`
                // rows the current binary would serve AUTHORITATIVELY for
                // `bar($$$A)` patterns. Signature rows have no row-level
                // migration path (their semantics are binary-defined), so
                // discard them and defeat both unchanged-file fast paths —
                // the content-hash prefix defeats the hash fast path and
                // dropping `mtime_identity_root` defeats the mtime fast path
                // (same mechanism as the schema-10 semantic re-render) — so
                // the next indexing pass re-extracts every file under the
                // current keying. Read-only opens cannot migrate; they refuse
                // loudly in `init_schema_readonly` instead.
                //
                // The hash prefix + mtime wipe defeat only TWO of the THREE
                // skip layers. The per-file structure
                // fingerprints (`body:<rel>` written by
                // `commit_prepared_files`, `struct:<rel>` written by
                // `upsert_file_material`) route a fingerprint-unchanged file
                // to `refresh_lines_only`, which rewrites the PLAIN content
                // hash WITHOUT reparsing — so a surviving fingerprint
                // re-legitimizes the hash on the very next pass and
                // `pattern_nodes` stays permanently empty (the loud gate's
                // own `asgrep reindex` remedy never rebuilt the rows; ident/
                // decl search silently answered `[]` and the codemod
                // honesty gate flipped to a silent ok:true zero-edit).
                // Discard both fingerprints here (same mechanism as the
                // schema-7/10 blocks) so the next pass takes the full
                // extract+upsert path and rebuilds the rows.
                self.conn.execute_batch(
                    "DELETE FROM pattern_nodes;
                     DELETE FROM meta WHERE key = 'mtime_identity_root';
                     DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%';
                     UPDATE files SET content_hash = 'schema15-rekey:' || content_hash
                       WHERE content_hash NOT LIKE 'schema15-rekey:%';",
                )?;
            }
            if version < 16 {
                // The extraction depth budget needs a per-file truncation
                // flag. Purely additive — pre-16 rows were
                // produced by the unbounded (complete) walk, so DEFAULT 0 is
                // truthful and no unchanged-file fast path needs defeating;
                // only re-extracted files can ever carry the flag. Guarded:
                // fresh (version-0) dbs just ran SCHEMA_DDL, which already
                // created the column.
                ensure_depth_truncated_column(&self.conn)?;
            }
            if version < 3 {
                self.conn.execute_batch(
                    "INSERT INTO lines_trigram(rowid, content) SELECT rowid, content FROM lines;",
                )?;
            }
            // Schema 8: backfill the unstemmed code field for older indexes.
            if version < 8 {
                self.conn.execute_batch(
                    "DELETE FROM lines_code_fts;
                 INSERT INTO lines_code_fts(rowid, content, file_id, line_no)
                   SELECT rowid, content, file_id, line_no FROM lines;",
                )?;
            }
            // Schema 6 (main): idx_symbols_name_lower arrives via SCHEMA_DDL above.
            // Schema 7: force re-embed under parent-mapped child layout (e2hc.6).
            if version < 7 {
                self.conn.execute_batch(
                    "DELETE FROM semantic_chunks;
                 DELETE FROM embeddings;
                 DELETE FROM embed_cache;
                 DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%'
                   OR key IN ('embed_backend', 'embed_model', 'embed_dim');",
                )?;
            }
            // Schema 10 changed chunk rendering and added per-field vectors.
            // All semantic state is derived, so rebuild it rather than mixing
            // legacy primary vectors or cache metadata with the new layout.
            if version < 10 {
                self.conn.execute_batch(
                    "DELETE FROM semantic_chunks;
                     DELETE FROM embeddings;
                     DELETE FROM embed_cache;
                     DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%'
                       OR key IN ('embed_backend', 'embed_model', 'embed_dim');
                     UPDATE files SET content_hash = 'semantic-layout-v3:' || content_hash
                       WHERE content_hash NOT LIKE 'semantic-layout-v3:%';",
                )?;
            }
            // Schema 17: chunking now always writes a whole-body parent
            // chunk beside extracted child fragments. Pre-17 stores can
            // represent a function by fragments alone, and neither the
            // mtime shortcut nor the content-hash path can observe a
            // chunk-derivation change, so rebuild derived semantic state
            // and defeat both fast paths (hash re-key + identity drop).
            if version < 17 {
                self.conn.execute_batch(
                    "DELETE FROM semantic_chunks;
                     DELETE FROM embeddings;
                     DELETE FROM embed_cache;
                     DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%'
                       OR key IN ('embed_backend', 'embed_model', 'embed_dim', 'mtime_identity_root');
                     UPDATE files SET content_hash = 'parent-chunk-v17:' || content_hash
                       WHERE content_hash NOT LIKE 'parent-chunk-v17:%';",
                )?;
            }
            // Schema 18: the two line analyzers become FTS5 external-content
            // tables bound to `lines`. Postings and ranking are identical
            // (columns read through the content table by rowid); the
            // duplicated `_content` shadow stores (~39 MB at repo scale)
            // disappear. Migration drops the legacy tables, recreates them
            // from the new DDL, and rebuilds postings from `lines` in-db --
            // no source re-read. Insert binds the same content value as the
            // `lines` row, so supplied-text tokenization cannot diverge.
            if version < 18 {
                self.conn.execute_batch(
                    "DROP TABLE IF EXISTS lines_fts; DROP TABLE IF EXISTS lines_code_fts;",
                )?;
                self.conn.execute_batch(SCHEMA_DDL)?;
                self.conn.execute_batch(
                    "INSERT INTO lines_fts(lines_fts) VALUES('rebuild');\
                     INSERT INTO lines_code_fts(lines_code_fts) VALUES('rebuild');\
                     DROP INDEX IF EXISTS idx_pattern_nodes_file;",
                )?;
            }
            self.conn
                .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
            Ok(())
        })();
        match migration {
            Ok(()) => self.conn.execute_batch("COMMIT")?,
            Err(error) => {
                if let Err(rollback_error) = self.conn.execute_batch("ROLLBACK") {
                    return Err(crate::StoreError::Other(format!(
                        "schema migration failed ({error}); rollback also failed: {rollback_error}"
                    )));
                }
                return Err(error);
            }
        }
        if drops_fts_shadows {
            // DROP frees pages to the freelist only; return the file space
            // once for the schema-18 shadow-content reclaim.
            let _ = self.conn.execute_batch("VACUUM");
        }
        Ok(())
    }

    fn init_schema_readonly(&self, version: i64) -> Result<()> {
        if version < SCHEMA_VERSION {
            // A below-current stamp means the on-disk rows may predate a
            // schema migration (14→15: the php `::` call re-key; 15→16: the
            // additive per-file depth-truncation flag the cached-lane
            // refusal keys on). A read-only open cannot
            // migrate, and the exact `call:`/`decl:` lane serves rows
            // AUTHORITATIVELY, so refuse with a rebuild instruction instead of
            // silently answering from stale rows. A writable open (asgrep
            // index / reindex) migrates in place; doctor's peek reports the
            // same mismatch with the same recovery.
            return Err(crate::StoreError::Other(format!(
                "index schema version {version} is older than supported version {SCHEMA_VERSION}; its rows predate a schema migration and must not be served as authoritative. Rebuild with: asgrep reindex {} --json",
                self.root.display()
            )));
        }
        let core: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('files','lines','meta','symbols')",
            [],
            |r| r.get(0),
        )?;
        if core < 4 {
            return Err(crate::StoreError::EmptyIndex(format!(
                "index is empty for {}; run: asgrep index {} --json",
                self.root.display(),
                self.root.display()
            )));
        }
        Ok(())
    }
}