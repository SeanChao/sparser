use std::path::PathBuf;

fn main() {
    let dir: PathBuf = ["tree-sitter-solidity", "src"].iter().collect();
    cc::Build::new()
        .flag_if_supported("-Wno-unused-but-set-variable")
        .include(&dir)
        .file(dir.join("parser.c"))
        .compile("tree-sitter-solidity");
    build_tree_sitter_php();
}

fn build_tree_sitter_php() {
    let src_dir: PathBuf = ["tree-sitter-php", "src"].iter().collect();

    let mut c_config = cc::Build::new();
    c_config.include(&src_dir);
    c_config
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs");
    let parser_path = src_dir.join("parser.c");
    c_config.file(&parser_path);

    println!("cargo:rerun-if-changed={}", parser_path.to_str().unwrap());
    c_config.compile("tree-sitter-php");

    // If your language uses an external scanner written in C++,
    // then include this block of code:

    let mut cpp_config = cc::Build::new();
    cpp_config.cpp(true);
    cpp_config.include(&src_dir);
    cpp_config
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable");
    let scanner_path = src_dir.join("scanner.cc");
    cpp_config.file(&scanner_path);
    println!("cargo:rerun-if-changed={}", scanner_path.to_str().unwrap());
    cpp_config.compile("scanner");
}
