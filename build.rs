use std::path::PathBuf;

fn main() {
    let dir: PathBuf = ["tree-sitter-solidity", "src"].iter().collect();
    cc::Build::new()
        .flag_if_supported("-Wno-unused-but-set-variable")
        .include(&dir)
        .file(dir.join("parser.c"))
        .compile("tree-sitter-solidity");
}
