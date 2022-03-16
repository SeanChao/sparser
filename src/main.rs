use clap::Parser as ArgsParser;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use tree_sitter::{Language, Node, Parser, Query, QueryCapture, QueryCursor};
use walkdir::{DirEntry, WalkDir};

extern "C" {
    fn tree_sitter_solidity() -> Language;
}

#[derive(ArgsParser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[clap(short = 'd', long)]
    data: String,
    #[clap(short = 't', long, default_value = "func_call")]
    task: String,
    #[clap(short = 'o', long)]
    out_dir: String,
}

#[derive(Serialize, Deserialize, Clone)]
enum DataSample {
    FuncCall(String, String),
    FuncCallComm(String, String, String, String),
    /// function src and function comment
    FuncComm(String, String),
}

fn process_func_call_comm(code: &str, parser: &mut Parser, language: Language) -> Vec<DataSample> {
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
    unimplemented!()
}

fn process_func_call(code: &str, parser: &mut Parser, language: Language) -> Vec<DataSample> {
    let parsed = parser.parse(&code, None).unwrap();

    let root = parsed.root_node();
    let func_body_query_string = fs::read_to_string("./query/func_body.sexp").unwrap();
    let fc_query = Query::new(language, &func_body_query_string).unwrap();
    let mut fc_qc = QueryCursor::new();
    let matches = fc_qc.matches(&fc_query, root, code.as_bytes());
    let re = Regex::new(r"\s+").unwrap();
    let mut func_src_map: HashMap<String, String> = HashMap::new();
    let mut dup_funcs = HashSet::new(); // duplicated function names are ignore for simplicity
    for m in matches {
        // match a function name with its comment
        let mut name = "";
        let mut func_body = "".to_string();
        for capture in m.captures {
            let capture_name = &fc_query.capture_names()[capture.index as usize];
            match capture_name.as_str() {
                "name" => {
                    name = capture.node.utf8_text(&code.as_bytes()).unwrap_or("");
                    if dup_funcs.contains(name) {
                        continue;
                    }
                    if func_src_map.contains_key(name) {
                        dup_funcs.insert(name.to_string());
                        func_src_map.remove(name);
                    }
                }
                "func_body" => {
                    let body = capture.node.utf8_text(&code.as_bytes()).unwrap_or("");
                    let body = re.replace_all(&body, " ").to_string().trim().to_string() + " ";
                    func_body = body;
                }
                unhandled => {
                    println!("unhandled match: {}", unhandled);
                }
            }
        }
        func_src_map.insert(name.to_string(), func_body);
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
                    if func_src_map.contains_key(func_name.as_str()) {
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
    // generate dataset
    let mut samples = Vec::new();
    for (caller, callee) in &calling_pairs {
        match (func_src_map.get(caller), func_src_map.get(callee)) {
            (Some(caller_code), Some(callee_code)) => {
                println!("{} -> {}", caller, callee);
                println!("{}", caller_code);
                println!("{}", callee_code);
                samples.push(DataSample::FuncCall(
                    caller_code.to_string(),
                    callee_code.to_string(),
                ))
            }
            _ => {}
        }
    }
    samples
}

fn process_func_comm(code: &str, parser: &mut Parser, language: Language) -> Vec<DataSample> {
    let parsed = parser.parse(&code, None).unwrap();

    let root = parsed.root_node();
    let func_comm_query_string = fs::read_to_string("./query/func_comment.sexp").unwrap();
    let fc_query = Query::new(language, &func_comm_query_string).unwrap();
    let mut fc_qc = QueryCursor::new();
    let matches = fc_qc.matches(&fc_query, root, code.as_bytes());
    let re = Regex::new(r"\s+").unwrap();
    let mut func_comments: HashMap<String, String> = HashMap::new();
    let mut func_code: HashMap<String, String> = HashMap::new();
    let mut dup_funcs = HashSet::new(); // duplicated function names are ignore for simplicity
    for m in matches {
        // match a function name with its comment
        let mut comment = "".to_string();
        let mut name = "";
        let mut src = "".to_string();
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
                        .trim()
                        .to_string();
                    let com = com.strip_prefix("*").unwrap_or(&com);
                    let com = re.replace_all(&com, " ").to_string().trim().to_string() + "\n";
                    comment.push_str(&com);
                }
                "func_src" => {
                    let body = get_node_text(capture.node, &code);
                    src = body;
                }
                _unhandled => {}
            }
        }
        func_comments.insert(name.to_string(), comment);
        func_code.insert(name.to_string(), src);
    }

    // generate dataset
    let mut samples = Vec::new();
    for (name, comment) in &func_comments {
        if comment.len() == 0 {
            continue;
        }
        if let Some(src) = func_code.get(name) {
            samples.push(DataSample::FuncComm(src.to_string(), comment.to_string()));
        }
    }
    samples
}

fn main() {
    let mut parser = Parser::new();
    let language = unsafe { tree_sitter_solidity() };
    parser.set_language(language).unwrap();
    let args = Args::parse();
    let data_dir = args.data;
    let task = args.task;
    let out_dir = args.out_dir.strip_suffix("/").unwrap_or(&args.out_dir);
    let task_fp = match task.as_str() {
        "func_call" => process_func_call,
        "func_call_comm" => process_func_call_comm,
        "func_comm" => process_func_comm,
        &_ => panic!("unknown task"),
    };

    let mut all_samples = Vec::new();
    let paths: Vec<DirEntry> = WalkDir::new(data_dir)
        .into_iter()
        .map(|e| e.unwrap())
        .collect();
    let paths_len = paths.len();
    for (idx, entry) in paths.iter().enumerate() {
        print!("\x1b[K\r{}/{}", idx + 1, paths_len);
        let file_path = entry.path();
        if file_path.is_file() {
            match fs::read_to_string(file_path) {
                Ok(src) => {
                    let mut file_samples = task_fp(&src, &mut parser, language);
                    all_samples.append(&mut file_samples);
                }
                Err(e) => {
                    eprintln!("{} NOT FOUND: {}", file_path.to_str().unwrap(), e);
                }
            }
        }
    }
    println!();
    save_dataset(out_dir, &all_samples);
}

fn write_to_json(samples: &Vec<DataSample>, file_path: &str) {
    println!("Writing to {}", file_path);
    let mut file = File::create(file_path).unwrap();
    // let mut writer = BufWriter::new(file);
    for sample in samples {
        // writer.write_fmt();
        let json_string = match sample {
            DataSample::FuncComm(src, com) => serde_json::to_string(&(src, com)).unwrap() + "\n",
            _ => unimplemented!(),
        };
        file.write(json_string.as_bytes()).unwrap();
    }
}

fn split_array<T: Clone>(arr: &Vec<T>, proportion0: usize, proportion1: usize) -> (Vec<T>, Vec<T>) {
    let sum = proportion0 + proportion1;
    let size0 = (proportion0 as f64 / sum as f64 * arr.len() as f64).ceil() as usize;
    let arr0 = arr[0..size0].to_vec();
    let arr1 = arr[size0..].to_vec();
    return (arr0, arr1);
}

fn save_dataset(path_prefix: &str, samples: &Vec<DataSample>) {
    fs::create_dir_all(path_prefix).unwrap();
    write_to_json(samples, &format!("{}/all.jsonl", path_prefix));
    // split into train:val:test = 8:1:1
    let (train_samples, other_samples) = split_array(samples, 8, 2);
    let (val_samples, test_samples) = split_array(&other_samples, 1, 1);
    write_to_json(&train_samples, &format!("{}/train.jsonl", path_prefix));
    write_to_json(&val_samples, &format!("{}/val.jsonl", path_prefix));
    write_to_json(&test_samples, &format!("{}/test.jsonl", path_prefix));
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
