use super::IndexStore;
use crate::Result;

impl IndexStore {
    pub fn begin_file_tx(&self) -> Result<()> {
        let depth = self.file_tx_depth.get();
        if depth == 0 {
            self.file_tx_poisoned.set(false);
            if self.conn.is_autocommit() {
                // Was unconditionally OFF; now the profile decides.
                self.begin_owned_transaction(&format!(
                    "PRAGMA synchronous = {}",
                    self.durability.write_pragma()
                ))?;
                self.file_tx_owns.set(true);
            } else {
                // Bulk (or other) transaction owns the write set.
                self.file_tx_owns.set(false);
            }
        }
        self.file_tx_depth.set(depth + 1);
        Ok(())
    }
    pub fn commit_file_tx(&self) -> Result<()> {
        self.end_file_tx(true)
    }
    pub fn rollback_file_tx(&self) -> Result<()> {
        self.end_file_tx(false)
    }
    fn restore_synchronous(&self) -> Result<()> {
        self.conn.execute_batch(&format!(
            "PRAGMA synchronous = {}; PRAGMA cache_size = -16384",
            self.durability.steady_pragma()
        ))?;
        Ok(())
    }
    /// Apply write-mode pragmas and acquire a transaction as one admission.
    /// If acquisition fails after (for example) FastUnsafe selected
    /// `synchronous=OFF`, restore the steady profile before returning.
    fn begin_owned_transaction(&self, setup: &str) -> Result<()> {
        let start = (|| -> Result<()> {
            self.conn.execute_batch(setup)?;
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            Ok(())
        })();
        let Err(start_error) = start else {
            return Ok(());
        };

        if !self.conn.is_autocommit() {
            if let Err(rollback_error) = self.conn.execute_batch("ROLLBACK") {
                return Err(crate::StoreError::Other(format!(
                    "transaction admission failed ({start_error}); cleanup ROLLBACK failed: {rollback_error}"
                )));
            }
        }
        // Fail visible if cleanup itself cannot restore a safe steady profile.
        self.restore_synchronous()?;
        Err(start_error)
    }
    fn execute_transaction_end(&self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql)?;
        Ok(())
    }
    /// End a transaction owned by this store and restore its steady-state
    /// connection settings. A failed COMMIT is followed by a best-effort
    /// ROLLBACK before any error is returned, so callers never inherit an open
    /// transaction merely because commit failed.
    fn finish_owned_transaction(&self, commit: bool) -> Result<()> {
        let mut tx_error = self
            .execute_transaction_end(if commit { "COMMIT" } else { "ROLLBACK" })
            .err();

        if commit && tx_error.is_some() && !self.conn.is_autocommit() {
            let commit_error = tx_error.take().expect("checked above");
            tx_error = match self.execute_transaction_end("ROLLBACK") {
                Ok(()) => Some(commit_error),
                Err(rollback_error) => Some(crate::StoreError::Other(format!(
                    "COMMIT failed: {commit_error}; cleanup ROLLBACK failed: {rollback_error}"
                ))),
            };
        }

        if !self.conn.is_autocommit() {
            let cleanup_error = crate::StoreError::Other(
                "transaction cleanup failed: SQLite transaction remains active".into(),
            );
            return Err(match tx_error {
                Some(error) => crate::StoreError::Other(format!("{error}; {cleanup_error}")),
                None => cleanup_error,
            });
        }

        // Preserve the existing fail-visible policy: a stuck write-batch mode
        // is operationally more dangerous than the transaction error that led
        // to cleanup, so restoration errors take precedence.
        self.restore_synchronous()?;
        match tx_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn end_file_tx(&self, commit: bool) -> Result<()> {
        let depth = self.file_tx_depth.get();
        if depth == 0 {
            return Ok(());
        }
        if !commit {
            self.file_tx_poisoned.set(true);
        }
        if depth > 1 {
            // Nested end: never COMMIT/ROLLBACK the outer transaction here.
            self.file_tx_depth.set(depth - 1);
            return Ok(());
        }
        let owns = self.file_tx_owns.get();
        let poisoned = self.file_tx_poisoned.get();
        let transaction_result = owns.then(|| self.finish_owned_transaction(commit && !poisoned));
        // Clear bookkeeping before propagating any transaction/restore error so
        // a failed end cannot leave stale state that confuses the next begin.
        self.file_tx_depth.set(0);
        self.file_tx_owns.set(false);
        self.file_tx_poisoned.set(false);
        if let Some(result) = transaction_result {
            result?;
        }
        if poisoned && commit {
            return Err(crate::StoreError::Other(
                "file_tx commit refused: nested file_tx rolled back".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn with_file_tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.begin_file_tx()?;
        match f() {
            Ok(v) => {
                // Nested rollback poisons the write set. Do not return Ok(v) after
                // rolling back: callers would treat a failed nested write as success
                // (e.g. file_id that no longer exists). Match commit_file_tx refusal.
                if self.file_tx_poisoned.get() {
                    self.rollback_file_tx()?;
                    return Err(crate::StoreError::Other(
                        "file_tx commit refused: nested file_tx rolled back".into(),
                    ));
                }
                self.commit_file_tx()?;
                Ok(v)
            }
            Err(e) => {
                self.rollback_file_tx()?;
                Err(e)
            }
        }
    }
    pub(crate) fn meta_u64(&self, key: &str) -> Result<u64> {
        Ok(self
            .get_meta(key)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }
    pub(crate) fn bump_meta_u64(&self, key: &str, delta: usize) -> Result<()> {
        if delta == 0 {
            return Ok(());
        }
        let total = self.meta_u64(key)?.saturating_add(delta as u64);
        self.set_meta(key, &total.to_string())
    }
    pub fn begin_bulk_tx(&self) -> Result<()> {
        if self.bulk_tx_active.get() {
            return Ok(());
        }
        if self.conn.is_autocommit() {
            // Was unconditionally OFF; now the profile decides.
            self.begin_owned_transaction(&format!(
                "PRAGMA temp_store = MEMORY; PRAGMA cache_size = -131072; PRAGMA mmap_size = 536870912; \
                 PRAGMA synchronous = {}",
                self.durability.write_pragma()
            ))?;
            self.bulk_tx_owns.set(true);
        } else {
            self.bulk_tx_owns.set(false);
        }
        self.bulk_tx_active.set(true);
        Ok(())
    }
    pub fn commit_bulk_tx(&self) -> Result<()> {
        self.end_bulk_tx(true)
    }
    pub fn rollback_bulk_tx(&self) -> Result<()> {
        self.end_bulk_tx(false)
    }
    /// Finish a bulk write started with [`begin_bulk_tx`].
    ///
    /// On `Ok`, commits. On `Err`, rolls back and **prefers** the rollback/restore
    /// error so a stuck FastUnsafe `synchronous=OFF` mode cannot hide behind the
    /// original write failure (no `let _ = rollback`).
    pub fn apply_bulk_write_result(&self, write_result: Result<()>) -> Result<()> {
        match write_result {
            Ok(()) => self.commit_bulk_tx(),
            Err(e) => match self.rollback_bulk_tx() {
                Ok(()) => Err(e),
                Err(rb) => Err(rb),
            },
        }
    }
    fn end_bulk_tx(&self, commit: bool) -> Result<()> {
        if !self.bulk_tx_active.get() {
            return Ok(());
        }
        let owns = self.bulk_tx_owns.get();
        let transaction_result = owns.then(|| self.finish_owned_transaction(commit));
        self.bulk_tx_active.set(false);
        self.bulk_tx_owns.set(false);
        self.file_tx_depth.set(0);
        self.file_tx_owns.set(false);
        self.file_tx_poisoned.set(false);
        match transaction_result {
            Some(result) => result,
            None => Ok(()),
        }
    }
}
