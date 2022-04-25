use clap::Parser as ArgsParser;
use futures::StreamExt;
use linya::Progress;
use log::{debug, error};
use rayon::prelude::*;
use sparser::{append_jsonl_to_file, get_node_text, CallJsonSample, JsonSample, FUNC_CALL_ID_MASK};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufRead};
use std::ops::DerefMut;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::Mutex;
use tree_sitter::{Language, Node, Query, QueryCursor};
use walkdir::{DirEntry, WalkDir};

#[derive(ArgsParser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[clap(short = 'd', long)]
    data: String,
    #[clap(short = 'o', long, default_value = "output")]
    out: String,
    #[clap(short = 'l', long)]
    lang: TargetLanguage,
    #[clap(short = 't', long, default_value_t=num_cpus::get())]
    threads: usize,
}

#[derive(Debug, Clone, Copy)]
enum TargetLanguage {
    Python,
    Javascript,
    Java,
    Go,
    Php,
    Ruby,
}

impl FromStr for TargetLanguage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "go" => Ok(TargetLanguage::Go),
            "javascript" => Ok(TargetLanguage::Javascript),
            "java" => Ok(TargetLanguage::Java),
            "php" => Ok(TargetLanguage::Php),
            "python" => Ok(TargetLanguage::Python),
            "ruby" => Ok(TargetLanguage::Ruby),
            _ => Err(format!("Unknown language: {}", s)),
        }
    }
}

lazy_static::lazy_static! {
    static ref PROGRESS: Mutex<Progress> = Mutex::new(Progress::new());
}

#[tokio::main]
async fn main() {
    simple_logger::init_with_env().unwrap();
    let args = Args::parse();
    let data_dir = args.data;
    let out_file = args.out;
    let lang = args.lang;
    let num_threads = args.threads;
    run_preprocessing(&data_dir, &out_file, lang, num_threads).await;
}

async fn run_preprocessing(
    data_dir: &str,
    out_file: &str,
    language: TargetLanguage,
    num_threads: usize,
) {
    let (tx, mut rx) = mpsc::channel(10);
    let data_dir = data_dir.to_string();
    let input_th = tokio::spawn(async move { read_input_data(data_dir.as_str(), tx).await });
    let parent = Path::new(out_file).parent();
    fs::create_dir_all(parent.unwrap()).unwrap();
    let file = File::create(out_file).unwrap();
    let file = Arc::new(Mutex::new(file));

    // let mut processing_threads = Vec::new();
    let rx_stream = async_stream::stream! {
        while let Some(item) = rx.recv().await {
            yield item;
        }
    };
    let generated_samples = rx_stream
        .map(|sample_group: Vec<JsonSample>| async move {
            let samples = process_grouped_samples(&sample_group, language).await;
            let samples: Vec<CallJsonSample> = samples
                .into_par_iter()
                .map(|(caller, callee, label)| {
                    let (caller_code, caller_code_tokens) = match label {
                        true => {
                            let tokens = caller
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
                                .collect::<Vec<String>>();
                            let re =
                                regex::Regex::new(&format!(r"\b{}\b", &callee.func_name)).unwrap();
                            let code = re.replace_all(&caller.code, FUNC_CALL_ID_MASK).to_string();
                            (code, tokens)
                        }
                        false => (caller.code.clone(), caller.code_tokens.clone()),
                    };
                    CallJsonSample {
                        caller_code,
                        caller_comm: caller.docstring.clone(),
                        callee_code: callee.code.clone(),
                        callee_comm: callee.docstring.clone(),
                        label,
                        caller_code_tokens: caller_code_tokens,
                        caller_comm_tokens: caller.docstring_tokens.clone(),
                        callee_code_tokens: callee.code_tokens.clone(),
                        callee_comm_tokens: callee.docstring_tokens.clone(),
                    }
                })
                .collect();
            samples
        })
        .buffer_unordered(num_threads);
    generated_samples
        .for_each(|samples| {
            let file = file.clone();
            async move {
                append_jsonl_to_file(&samples, file.lock().await.deref_mut()).unwrap();
            }
        })
        .await;
    input_th.await.unwrap();
}

async fn read_input_data(data_dir: &str, tx: Sender<Vec<JsonSample>>) {
    let paths: Vec<DirEntry> = WalkDir::new(data_dir)
        .into_iter()
        .map(|e| e.unwrap())
        .collect();
    let files: Vec<DirEntry> = paths
        .into_iter()
        .filter(|e| e.file_type().is_file())
        .collect();

    let files_bar = PROGRESS.lock().await.bar(files.len(), "Files");
    let mut input_threads = Vec::new();
    for (idx, entry) in files.into_iter().enumerate() {
        let file_path = entry.into_path();
        // info!("{}/{} {}", idx + 1, len, file_path.to_str().unwrap());
        if file_path.is_file() {
            let tx = tx.clone();
            let file_path = file_path.clone();
            let input_data_thread = tokio::spawn(async move {
                let bar = PROGRESS.lock().await.bar(
                    read_lines(&file_path).unwrap().count(),
                    &format!("[IN] #{} {}", idx, file_path.to_str().unwrap()),
                );
                if let Ok(lines) = read_lines(&file_path) {
                    let mut sample_group_identifier = String::new();
                    let mut cur_group_samples = Vec::new();
                    for line in lines {
                        if let Ok(line) = line {
                            if line.len() == 0 {
                                continue;
                            }
                            if let Ok(mut json_sample) = serde_json::from_str::<JsonSample>(&line) {
                                json_sample.func_name =
                                    json_sample.func_name.split('.').last().unwrap().to_string();
                                if json_sample.repo != sample_group_identifier
                                    && cur_group_samples.len() > 0
                                {
                                    debug!("sent {} samples", cur_group_samples.len());
                                    match tx.send(cur_group_samples).await {
                                        Ok(_) => {}
                                        Err(e) => error!("tx error {:?}", e.source()),
                                    }
                                    // reset
                                    cur_group_samples = Vec::new();
                                    sample_group_identifier = json_sample.repo.clone();
                                }
                                cur_group_samples.push(json_sample);
                            }
                        }
                        PROGRESS.lock().await.inc_and_draw(&bar, 1);
                    }
                    if !cur_group_samples.is_empty() {
                        debug!("sent {} samples", cur_group_samples.len());
                        tx.send(cur_group_samples).await.unwrap();
                    }
                }
            });
            input_threads.push(input_data_thread);
        }
        PROGRESS.lock().await.inc_and_draw(&files_bar, 1);
    }

    for input_thread in input_threads {
        input_thread.await.unwrap();
    }
}

macro_rules! get_tree_sitter_language {
    ($lang: expr) => {
        match $lang {
            TargetLanguage::Python => tree_sitter_python::language(),
            TargetLanguage::Javascript => tree_sitter_javascript::language(),
            TargetLanguage::Go => tree_sitter_go::language(),
            TargetLanguage::Java => tree_sitter_java::language(),
            TargetLanguage::Ruby => tree_sitter_ruby::language(),
            TargetLanguage::Php => unsafe { tree_sitter_php() },
        }
    };
}

async fn process_grouped_samples(
    sample_group: &Vec<JsonSample>,
    lang: TargetLanguage,
) -> Vec<(JsonSample, JsonSample, bool)> {
    let res: Vec<Vec<(JsonSample, JsonSample, bool)>> = sample_group
        .par_iter()
        .map(|sample| {
            let mut all_samples = Vec::new();
            // find all function calls in this sample
            let code = &sample.code;
            let parser_lang = get_tree_sitter_language!(lang);
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(parser_lang).unwrap();
            let root = parser.parse(code, None).unwrap();
            let mut other_funcs = sample_group
                .iter()
                .map(|e| (e.func_name.as_str(), e))
                .collect::<BTreeMap<&str, &JsonSample>>();
            other_funcs.retain(|k, _v| *k != &sample.func_name);
            let callees = find_function_calls(lang, code, root.root_node(), |func_name| {
                other_funcs.contains_key(func_name)
            });
            let mut non_callees = other_funcs.clone();
            non_callees.retain(|k, _v| !callees.contains(k.to_owned()));

            // generate a (caller, callee) pair
            for callee in &callees {
                let callee_sample = *other_funcs.get(callee.as_str()).unwrap();
                let sample = (sample.clone(), callee_sample.clone(), true);
                all_samples.push(sample);
            }
            let mut neg_samples_needed = callees.len();
            // generate a (caller, non-callee) pair
            for (_, non_callee) in non_callees {
                if neg_samples_needed == 0 {
                    break;
                }
                let sample = (sample.clone(), non_callee.clone(), false);
                all_samples.push(sample);
                neg_samples_needed -= 1;
            }
            all_samples
        })
        .collect();

    res.into_iter()
        .flatten()
        .collect::<Vec<(JsonSample, JsonSample, bool)>>()
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

const JAVASCRIPT_SEXP_FUNC_CALL: &str = "
(call_expression
  function: (identifier) @function)
(call_expression
  function: (member_expression
    property: (property_identifier) @function.method))
";
const JAVA_SEXP_FUNC_CALL: &str = "(method_declaration
  name: (identifier) @function.method)
(method_invocation
  name: (identifier) @function.method)
";
const GO_SEXP_FUNC_CALL: &str = "
(call_expression
  function: (identifier) @function)
(call_expression
  function: (selector_expression
    field: (field_identifier) @function.method))";

const RUBY_SEXP_FUNC_CALL: &str = "
(call
  method: [(identifier) (constant)] @function.method)";
const PHP_SEXP_FUNC_CALL: &str = "
(member_call_expression
  name: (name) @function.method)
(function_call_expression
  function: (qualified_name (name)) @function)
";

fn find_function_calls<F>(
    language: TargetLanguage,
    code: &str,
    root: Node,
    func_validate_fn: F,
) -> HashSet<String>
where
    F: Fn(&str) -> bool,
{
    let query_string = match language {
        TargetLanguage::Python => PYTHON_SEXP_FUNC_CALL,
        TargetLanguage::Javascript => JAVASCRIPT_SEXP_FUNC_CALL,
        TargetLanguage::Java => JAVA_SEXP_FUNC_CALL,
        TargetLanguage::Go => GO_SEXP_FUNC_CALL,
        TargetLanguage::Ruby => RUBY_SEXP_FUNC_CALL,
        TargetLanguage::Php => PHP_SEXP_FUNC_CALL,
    };
    let language = get_tree_sitter_language!(language);
    let query = Query::new(language, &query_string).unwrap();
    let mut query_cursor = QueryCursor::new();
    let matches = query_cursor.matches(&query, root, |_| code.as_bytes());
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

extern "C" {
    fn tree_sitter_php() -> Language;
}

// fn get_tree_sitter_language(lang: TargetLanguage) -> tree_sitter_python::tree_sitter::language() {
//     // let lan = tree_sitter_python::language();
//     match lang {
//         TargetLanguage::Python => tree_sitter_python::language(),
//         // TargetLanguage::Javascript => tree_sitter_javascript::language(),
//         // TargetLanguage::Go => tree_sitter_go::language(),
//         // TargetLanguage::Java => tree_sitter_java::language(),
//         // TargetLanguage::Ruby => tree_sitter_ruby::language(),
//         // TargetLanguage::Php => unsafe { tree_sitter_php() },
//         _ => panic!(),
//     }
// }
