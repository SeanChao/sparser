use clap::Parser as ArgsParser;
use log::{debug, info};
use sparser::{get_node_text, save_data_gen, JsonSample, FUNC_CALL_ID_MASK};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
use std::str::FromStr;
use tree_sitter::{Language, Node, Query, QueryCursor};
use walkdir::{DirEntry, WalkDir};

#[derive(ArgsParser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[clap(short = 'd', long)]
    data: String,
    #[clap(short = 'o', long, default_value = "output")]
    out_dir: String,
    #[clap(short = 'l', long)]
    lang: TargetLanguage,
}

#[derive(Debug, Clone, Copy)]
enum TargetLanguage {
    Python,
}

impl FromStr for TargetLanguage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "python" => Ok(TargetLanguage::Python),
            _ => Err(format!("Unknown language: {}", s)),
        }
    }
}

fn main() {
    simple_logger::init_with_env().unwrap();
    let args = Args::parse();
    let data_dir = args.data;
    let out_dir = args.out_dir;
    let lang = args.lang;
    let language = match lang {
        TargetLanguage::Python => tree_sitter_python::language(),
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(language).unwrap();

    let paths: Vec<DirEntry> = WalkDir::new(data_dir)
        .into_iter()
        .map(|e| e.unwrap())
        .collect();
    let paths_len = paths.len();
    let mut num_neg_samples_needed = 0;
    let mut all_samples = Vec::new();
    for (idx, entry) in paths.iter().enumerate() {
        info!("{}/{}", idx + 1, paths_len);
        let file_path = entry.path();
        if file_path.is_file() {
            if let Ok(lines) = read_lines(file_path) {
                debug!("{}", file_path.display());
                let mut sample_group_identifier = String::new();
                let mut cur_code_file_samples = Vec::new();
                let mut grouped_samples = BTreeSet::new();
                for line in lines {
                    if let Ok(line) = line {
                        if line.len() == 0 {
                            continue;
                        }
                        if let Ok(mut json_sample) = serde_json::from_str::<JsonSample>(&line) {
                            json_sample.func_name =
                                json_sample.func_name.split('.').last().unwrap().to_string();
                            if json_sample.repo != sample_group_identifier {
                                // flush current file path's samples
                                grouped_samples.insert(cur_code_file_samples);
                                // reset
                                cur_code_file_samples = Vec::new();
                                sample_group_identifier = json_sample.repo.clone();
                            }
                            cur_code_file_samples.push(json_sample);
                        }
                    }
                }
                if !cur_code_file_samples.is_empty() {
                    grouped_samples.insert(cur_code_file_samples);
                }
                for group in &grouped_samples {
                    for sample in group {
                        // find all function calls in this sample
                        let code = &sample.code;
                        let root = parser.parse(code, None).unwrap();
                        let mut other_funcs = group
                            .clone()
                            .into_iter()
                            .map(|e| (e.func_name.clone(), e))
                            .collect::<BTreeMap<String, JsonSample>>();
                        other_funcs.retain(|k, _v| k != &sample.func_name);
                        let callees =
                            find_function_calls(language, code, root.root_node(), |func_name| {
                                other_funcs.contains_key(func_name)
                            });
                        let mut non_callees = other_funcs.clone();
                        non_callees.retain(|k, _v| !callees.contains(k));

                        // generate a (caller, callee) pair
                        for callee in &callees {
                            let callee_sample = other_funcs.get(callee).unwrap();
                            let sample = (sample.clone(), callee_sample.clone(), true);
                            all_samples.push(sample);
                            num_neg_samples_needed += 1;
                        }
                        // generate a (caller, non-callee) pair
                        for (_, non_callee) in &non_callees {
                            if num_neg_samples_needed < 0 {
                                break;
                            }
                            let sample = (sample.clone(), non_callee.clone(), false);
                            all_samples.push(sample);
                            num_neg_samples_needed -= 1;
                        }
                    }
                }
            }
        }
    }
    info!("Collected {} samples", all_samples.len());
    info!("#imbalance samples: {}", num_neg_samples_needed);
    let all_samples: Vec<(Vec<String>, Vec<String>, Vec<String>, Vec<String>, bool)> = all_samples
        .into_iter()
        .map(|(caller, callee, label)| {
            let caller_code_tokens = match label {
                true => caller
                    .code_tokens
                    .clone()
                    .into_iter()
                    .map(|t| {
                        if &t == &callee.func_name {
                            FUNC_CALL_ID_MASK.to_string()
                        } else {
                            t
                        }
                    })
                    .collect::<Vec<String>>(),
                false => caller.code_tokens.clone(),
            };
            (
                caller_code_tokens,
                caller.docstring_tokens.clone(),
                callee.code_tokens.clone(),
                callee.docstring_tokens.clone(),
                label,
            )
        })
        .collect();
    save_data_gen(&out_dir, &all_samples);
}

// The output is wrapped in a Result to allow matching on errors
// Returns an Iterator to the Reader of the lines of the file.
fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}

const PYTHON_SEXP_FUNC_CALL: &str = "
(call
  function: (attribute attribute: (identifier) @function.method))
(call
  function: (identifier) @function)";

// use sparser::main::{get_node_text};

fn find_function_calls<F>(
    language: Language,
    code: &str,
    root: Node,
    func_validate_fn: F,
) -> HashSet<String>
where
    F: Fn(&str) -> bool,
{
    let query_string = PYTHON_SEXP_FUNC_CALL;
    let query = Query::new(language, &query_string).unwrap();
    let mut query_cursor = QueryCursor::new();
    let matches = query_cursor.matches(&query, root, code.as_bytes());
    let mut callees = HashSet::new();
    for m in matches {
        for capture in m.captures {
            let capture_name = &query.capture_names()[capture.index as usize];
            match capture_name.as_str() {
                "function" | "function.method" => {
                    let func_name = get_node_text(capture.node, &code);
                    if func_validate_fn(func_name.as_str()) {
                        callees.insert(func_name);
                    }
                }
                _ => {
                    println!("\tunknown capture_name: {}", capture_name);
                }
            }
        }
    }
    callees
}
