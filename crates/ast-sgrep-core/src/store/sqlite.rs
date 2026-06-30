use std::path::Path;

use rusqlite::Connection;

use super::index_db_path;
use super::schema::init_schema;
use crate::Result;

/// SQLite-backed index store.
pub struct IndexStore {
    pub(crate) conn: Connection,
    pub(crate) root: std::path::PathBuf,
    pub(crate) db_path: std::path::PathBuf,
}

impl IndexStore {
    pub fn open(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        let db_path = index_db_path(root, index_path);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        let store = Self {
            conn,
            root: root.to_path_buf(),
            db_path,
        };
        init_schema(&store.conn)?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}
