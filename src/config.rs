use std::path::PathBuf;

/// Runtime settings, read from environment variables. The defaults make a
/// plain `cargo run` from the repo work without any setup.
pub struct Config {
    /// `SMERT_BIND`: address the HTTP server listens on.
    pub bind:     String,
    /// `SMERT_DATA_DIR`: folder holding the SQLite database.
    pub data_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            bind:     std::env::var("SMERT_BIND").unwrap_or_else(|_| "0.0.0.0:1212".into()),
            data_dir: std::env::var_os("SMERT_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| "./data".into()),
        }
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("smert.db")
    }
}
