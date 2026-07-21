use crate::fixture::sample_root;
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;
pub struct CliSession {
    pub _temp: TempDir,
    pub root: PathBuf,
    pub index_path: PathBuf,
    pub bin: PathBuf,
}
impl CliSession {
    pub fn sample(bin: PathBuf) -> Self {
        let temp = TempDir::new().expect("tempdir");
        let session = Self {
            root: sample_root(),
            index_path: temp.path().join("index.db"),
            bin,
            _temp: temp,
        };
        session.index().expect("index sample fixture");
        session
    }
    pub fn search_json(&self, query: &str, extra: &[&str]) -> Value {
        let mut args = vec!["--index-path", self.index_path.to_str().unwrap(), "--json"];
        args.extend(extra);
        if !query.is_empty() {
            args.push(query);
        }
        args.push(self.root.to_str().unwrap());
        serde_json::from_slice(&self.run_success(&args).stdout).expect("search json")
    }
    pub fn run_success(&self, args: &[&str]) -> Output {
        let out = self.run(args).expect("run command");
        assert!(
            out.status.success(),
            "expected success, stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
    pub fn run_failure(&self, args: &[&str]) -> Output {
        let out = self.run(args).expect("run command");
        assert!(
            !out.status.success(),
            "expected failure, stdout: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        out
    }
    pub fn run(&self, args: &[&str]) -> Result<Output, String> {
        Command::new(&self.bin)
            .args(args)
            .output()
            .map_err(|e| e.to_string())
    }
    fn index(&self) -> Result<Output, String> {
        self.run(&[
            "--index-path",
            self.index_path.to_str().unwrap(),
            "index",
            self.root.to_str().unwrap(),
        ])
    }
}
