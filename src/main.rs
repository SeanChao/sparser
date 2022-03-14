use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use tree_sitter::{Language, Node, Parser, Query, QueryCapture, QueryCursor};

extern "C" {
    fn tree_sitter_solidity() -> Language;
}

fn main() {
    let mut parser = Parser::new();
    let language = unsafe { tree_sitter_solidity() };
    parser.set_language(language).unwrap();
    let code = fs::read_to_string("./example.sol").unwrap();
    let parsed = parser.parse(&code, None).unwrap();

    let root = parsed.root_node();
    let func_comm_query_string = fs::read_to_string("./query/func_comment.sexp").unwrap();
    let fc_query = Query::new(language, &func_comm_query_string).unwrap();
    let mut fc_qc = QueryCursor::new();
    let matches = fc_qc.matches(&fc_query, root, code.as_bytes());
    let re = Regex::new(r"\s+").unwrap();
    let mut func_comments: HashMap<String, String> = HashMap::new();
    let mut dup_funcs = HashSet::new(); // duplicated function names are ignore for simplicity
    for m in matches {
        // match a function name with its comment
        let mut comment = "".to_string();
        let mut name = "";
        for capture in m.captures {
            let capture_name = &fc_query.capture_names()[capture.index as usize];
            match capture_name.as_str() {
                "name" => {
                    name = capture.node.utf8_text(&code.as_bytes()).unwrap_or("");
                    if dup_funcs.contains(name) {
                        continue;
                    }
                    if func_comments.contains_key(name) {
                        dup_funcs.insert(name.to_string());
                        func_comments.remove(name);
                    }
                }
                "comment" => {
                    let com = capture
                        .node
                        .utf8_text(&code.as_bytes())
                        .unwrap_or("")
                        .replace("//", "")
                        .replace("/*", "")
                        .replace("*/", "")
                        .replace("\r?\n", " ");
                    let com = re.replace_all(&com, " ").to_string().trim().to_string() + " ";
                    comment.push_str(&com);
                }
                unhandled => {
                    println!("unhandled match: {}", unhandled);
                }
            }
        }
        print!("name: {}", name);
        println!(" | comment: {}", comment);
        func_comments.insert(name.to_string(), comment);
    }

    // find all function calls
    let query_string = fs::read_to_string("query/func_call.sexp").unwrap();
    let query = Query::new(language, &query_string).unwrap();
    let mut query_cursor = QueryCursor::new();
    let matches = query_cursor.matches(&query, root, code.as_bytes());
    let mut calling_pairs = HashSet::new();
    for m in matches {
        for capture in m.captures {
            let capture_name = &query.capture_names()[capture.index as usize];
            match capture_name.as_str() {
                "func_name" => {
                    let func_name = get_node_text(capture.node, &code);
                    if func_comments.contains_key(func_name.as_str()) {
                        // find caller
                        let mut node = capture.node;
                        while node.parent().is_some() {
                            let parent = node.parent().unwrap();
                            let kind = parent.kind();
                            if kind == "function_definition" {
                                let identifier_node =
                                    parent.child_by_field_name("function_name").unwrap();
                                let caller_name = get_node_text(identifier_node, &code);
                                println!("  caller found: {}", caller_name);
                                calling_pairs.insert((caller_name, func_name.clone()));
                            }
                            node = parent;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // debug
    // for (func_name, comment) in &func_comments {
    //     println!("{}\t| {}", func_name, comment);
    // }
    // for (caller, callee) in &calling_pairs {
    //     println!("{} -> {}", caller, callee);
    // }
    // generate dataset
    for (caller, callee) in &calling_pairs {
        match (func_comments.get(caller), func_comments.get(callee)) {
            (Some(caller_comment), Some(callee_comment)) => {
                println!("{} -> {}", caller, callee);
                println!("{}", caller_comment);
                println!("{}", callee_comment);
            }
            _ => {}
        }
    }
}

#[allow(dead_code)]
fn print_node_text(capture: &QueryCapture, query: &Query, code: &str) {
    let start = capture.node.start_position();
    let end = capture.node.end_position();
    let capture_name = &query.capture_names()[capture.index as usize];
    if end.row == start.row {
        println!(
            "    capture: {}, start: {}, text: {:?}",
            capture_name,
            start,
            capture.node.utf8_text(&code.as_bytes()).unwrap_or("")
        );
    } else {
        let start_byte = capture.node.start_byte();
        let end_byte = capture.node.end_byte();
        let text = &code.as_bytes()[start_byte..end_byte];
        let text = String::from_utf8(text.to_vec()).unwrap();
        println!(
            "    capture: {}, start: {}, end: {}, text: {:?}",
            capture_name, start, end, text
        );
    }
}

fn get_node_text(node: Node, code: &str) -> String {
    node.utf8_text(code.as_bytes()).unwrap_or("").to_string()
}
