use super::{should_skip_dir, should_skip_file};
use std::path::Path;

#[test]
fn skips_path_escape_noise_directory() {
    assert!(should_skip_dir(Path::new("~")));
    assert!(!should_skip_dir(Path::new("src")));
}

#[test]
fn indexes_swift_source_files() {
    assert!(!should_skip_file(Path::new("Sources/App/Main.swift")));
}

#[test]
fn indexes_c_cpp_kotlin_php_source_files() {
    assert!(!should_skip_file(Path::new("src/main.c")));
    assert!(!should_skip_file(Path::new("include/app.h")));
    assert!(!should_skip_file(Path::new("src/main.cpp")));
    assert!(!should_skip_file(Path::new("src/Main.kt")));
    assert!(!should_skip_file(Path::new("src/index.php")));
}
