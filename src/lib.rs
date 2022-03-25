use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
};
use tree_sitter::{Node, Query, QueryCapture};

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct JsonSample {
    pub func_name: String,
    // pub path: String,
    pub repo: String,
    pub original_string: String,
    pub code: String,
    pub code_tokens: Vec<String>,
    pub docstring: String,
    pub docstring_tokens: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Eq, PartialEq, Hash)]
pub enum DataSample {
    FuncCall(String, String),
    FuncCallComm(String, String, String, String, bool),
    /// function src and function comment
    FuncComm(String, String),
}

pub const FUNC_CALL_ID_MASK: &str = "<masked_func_id>";

pub fn write_to_json(samples: &Vec<DataSample>, file_path: &str) {
    println!("Writing to {}", file_path);
    let mut file = File::create(file_path).unwrap();
    for sample in samples {
        // writer.write_fmt();
        let json_string = match sample {
            DataSample::FuncComm(src, com) => serde_json::to_string(&(src, com)).unwrap(),
            DataSample::FuncCallComm(caller_src, caller_com, callee_src, callee_com, label) => {
                serde_json::to_string(&(caller_src, caller_com, callee_src, callee_com, label))
                    .unwrap()
            }
            _ => todo!(),
        } + "\n";
        file.write(json_string.as_bytes()).unwrap();
    }
}

pub fn split_array<T: Clone>(
    arr: &Vec<T>,
    proportion0: usize,
    proportion1: usize,
) -> (Vec<T>, Vec<T>) {
    let sum = proportion0 + proportion1;
    let size0 = (proportion0 as f64 / sum as f64 * arr.len() as f64).ceil() as usize;
    let arr0 = arr[0..size0].to_vec();
    let arr1 = arr[size0..].to_vec();
    return (arr0, arr1);
}

pub fn save_dataset(path_prefix: &str, samples: &Vec<DataSample>) {
    fs::create_dir_all(path_prefix).unwrap();
    write_to_json(samples, &format!("{}/all.jsonl", path_prefix));
    // split into train:val:test = 8:1:1
    let (train_samples, other_samples) = split_array(samples, 8, 2);
    let (val_samples, test_samples) = split_array(&other_samples, 1, 1);
    write_to_json(&train_samples, &format!("{}/train.jsonl", path_prefix));
    write_to_json(&val_samples, &format!("{}/val.jsonl", path_prefix));
    write_to_json(&test_samples, &format!("{}/test.jsonl", path_prefix));
}

pub fn append_jsonl_to_file<T: Serialize>(
    samples: &Vec<T>,
    file: &mut File,
) -> std::io::Result<()> {
    for sample in samples {
        let json_string = serde_json::to_string(sample).unwrap() + "\n";
        file.write(json_string.as_bytes())?;
    }
    Ok(())
}

pub fn write_to_json_gen<T: Serialize>(samples: &Vec<T>, file_path: &str) {
    println!("Writing to {}", file_path);
    let mut file = File::create(file_path).unwrap();
    for sample in samples {
        let json_string = serde_json::to_string(sample).unwrap() + "\n";
        file.write(json_string.as_bytes()).unwrap();
    }
}

pub fn save_data_gen<T: Serialize + Clone>(path_prefix: &str, samples: &Vec<T>) {
    fs::create_dir_all(path_prefix).unwrap();
    write_to_json_gen(samples, &format!("{}/all.jsonl", path_prefix));
    // split into train:val:test = 8:1:1
    let (train_samples, other_samples) = split_array(samples, 8, 2);
    let (val_samples, test_samples) = split_array(&other_samples, 1, 1);
    write_to_json_gen(&train_samples, &format!("{}/train.jsonl", path_prefix));
    write_to_json_gen(&val_samples, &format!("{}/val.jsonl", path_prefix));
    write_to_json_gen(&test_samples, &format!("{}/test.jsonl", path_prefix));
}

#[allow(dead_code)]
pub fn print_node_text(capture: &QueryCapture, query: &Query, code: &str) {
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

pub fn get_node_text(node: Node, code: &str) -> String {
    node.utf8_text(code.as_bytes()).unwrap_or("").to_string()
}
