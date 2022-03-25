use clap::Parser as ArgsParser;
use linya::Progress;
use log::{debug, error};
use rayon::prelude::*;
use sparser::{append_jsonl_to_file, get_node_text, JsonSample, FUNC_CALL_ID_MASK};
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
    let language = match lang {
        TargetLanguage::Python => tree_sitter_python::language(),
    };

    run_preprocessing(&data_dir, &out_file, language).await;
}

async fn run_preprocessing(data_dir: &str, out_file: &str, language: Language) {
    let (tx, mut rx) = mpsc::channel(10);
    let data_dir = data_dir.to_string();
    let input_th = tokio::spawn(async move { read_input_data(data_dir.as_str(), tx).await });
    let parent = Path::new(out_file).parent();
    fs::create_dir_all(parent.unwrap()).unwrap();
    let file = File::create(out_file).unwrap();
    let file = Arc::new(Mutex::new(file));

    let mut processing_threads = Vec::new();
    while let Some(sample_group) = rx.recv().await {
        let file = file.clone();
        processing_threads.push(tokio::spawn(async move {
            let samples = process_grouped_samples(&sample_group, language).await;
            debug!("recv {}", samples.len());
            let samples: Vec<(Vec<String>, Vec<String>, Vec<String>, Vec<String>, bool)> = samples
                .into_par_iter()
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
            append_jsonl_to_file(&samples, file.lock().await.deref_mut()).unwrap();
        }));
    }
    // wait for all to finish
    input_th.await.unwrap();
    for t in processing_threads {
        t.await.unwrap();
    }
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

async fn process_grouped_samples(
    sample_group: &Vec<JsonSample>,
    // parser: &mut Parser,
    language: Language,
) -> Vec<(JsonSample, JsonSample, bool)> {
    let res: Vec<Vec<(JsonSample, JsonSample, bool)>> = sample_group
        .par_iter()
        .map(|sample| {
            let mut all_samples = Vec::new();
            // find all function calls in this sample
            let code = &sample.code;
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(language).unwrap();
            let root = parser.parse(code, None).unwrap();
            // let mut other_funcs = sample_group
            //     .clone()
            //     .into_iter()
            //     .map(|e| (e.func_name.clone(), e))
            //     .collect::<BTreeMap<String, JsonSample>>();
            let mut other_funcs = sample_group
                .iter()
                .map(|e| (e.func_name.as_str(), e))
                .collect::<BTreeMap<&str, &JsonSample>>();
            other_funcs.retain(|k, _v| *k != &sample.func_name);
            let callees = find_function_calls(language, code, root.root_node(), |func_name| {
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
