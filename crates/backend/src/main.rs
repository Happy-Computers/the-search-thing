use std::env;
use std::path::PathBuf;
use std::time::Duration;

use fff_search::GrepMode;

pub mod search;

const DEFAULT_LIMIT: u32 = 20;
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        eprintln!();
        print_usage();
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args();
    let _program = arguments.next();
    let root = arguments
        .next()
        .ok_or_else(|| "missing root path".to_owned())?;
    let command = arguments
        .next()
        .ok_or_else(|| "missing command".to_owned())?;
    let root_path = PathBuf::from(root);
    let extract_cache_path = env::var_os("EXTRACT_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root_path.join(".data").join("extracted"));
    let service = search::SearchService::new(&root_path, &extract_cache_path)
        .map_err(|error| error.to_string())?;

    match command.as_str() {
        "files" => run_file_search(&service, arguments.collect()),
        "grep" => run_content_search(&service, arguments.collect()),
        "progress" => run_progress(&service),
        "rescan" => run_rescan(&service),
        _ => Err(format!("unknown command: {command}")),
    }
}

fn run_file_search(service: &search::SearchService, arguments: Vec<String>) -> Result<(), String> {
    let query = required_argument(&arguments, 0, "query")?;
    let limit = optional_limit(&arguments, 1)?;
    wait_for_scan(service)?;

    let result = service
        .file_search(query, limit)
        .map_err(|error| error.to_string())?;
    println!("total matched: {}", result.total_matched);
    for item in result.items {
        println!("{}\t{}", item.file_name, item.relative_path);
    }

    Ok(())
}

fn run_content_search(
    service: &search::SearchService,
    arguments: Vec<String>,
) -> Result<(), String> {
    let query = required_argument(&arguments, 0, "query")?;
    let mode = optional_mode(&arguments, 1)?;
    let limit = optional_limit(&arguments, 2)?;
    wait_for_scan(service)?;

    let result = service
        .content_search(query, mode, limit)
        .map_err(|error| error.to_string())?;
    println!("total matched: {}", result.total_matched);
    for item in result.items {
        println!(
            "{}:{}\t{}",
            item.relative_path, item.line_number, item.line_content
        );
    }

    Ok(())
}

fn run_progress(service: &search::SearchService) -> Result<(), String> {
    let progress = service.scan_progress().map_err(|error| error.to_string())?;
    println!("scanned files: {}", progress.scanned_files_count);
    println!("scanning: {}", progress.is_scanning);
    println!("watcher ready: {}", progress.is_watcher_ready);
    println!("warmup complete: {}", progress.is_warmup_complete);
    Ok(())
}

fn run_rescan(service: &search::SearchService) -> Result<(), String> {
    service
        .trigger_rescan()
        .map_err(|error| error.to_string())?;
    println!("rescan started");
    Ok(())
}

fn wait_for_scan(service: &search::SearchService) -> Result<(), String> {
    if service.wait_for_scan(SCAN_TIMEOUT) {
        return Ok(());
    }

    Err(format!(
        "initial scan did not finish within {} seconds",
        SCAN_TIMEOUT.as_secs()
    ))
}

fn required_argument<'a>(
    arguments: &'a [String],
    index: usize,
    name: &str,
) -> Result<&'a str, String> {
    arguments
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {name}"))
}

fn optional_limit(arguments: &[String], index: usize) -> Result<u32, String> {
    let Some(value) = arguments.get(index) else {
        return Ok(DEFAULT_LIMIT);
    };

    value
        .parse::<u32>()
        .map_err(|error| format!("invalid limit {value:?}: {error}"))
}

fn optional_mode(arguments: &[String], index: usize) -> Result<GrepMode, String> {
    let Some(value) = arguments.get(index) else {
        return Ok(GrepMode::PlainText);
    };

    match value.as_str() {
        "plain" => Ok(GrepMode::PlainText),
        "regex" => Ok(GrepMode::Regex),
        "fuzzy" => Ok(GrepMode::Fuzzy),
        _ => Err(format!(
            "invalid grep mode {value:?}; use plain, regex, or fuzzy"
        )),
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  backend <root> files <query> [limit]");
    eprintln!("  backend <root> grep <query> [plain|regex|fuzzy] [limit]");
    eprintln!("  backend <root> progress");
    eprintln!("  backend <root> rescan");
    eprintln!();
    eprintln!("EXTRACT_CACHE_DIR overrides <root>/.data/extracted");
}
