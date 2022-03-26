cargo install tree-sitter-cli
git submodule add https://github.com/JoranHonig/tree-sitter-solidity.git
(cd tree-sitter-solidity && tree-sitter generate)
git submodule add https://github.com/tree-sitter/tree-sitter-php.git
(cd tree-sitter-php && tree-sitter generate)