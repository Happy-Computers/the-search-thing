use std::fmt::{Display, Formatter};
use std::path::Path;
use std::time::Duration;

use fff_search::{
    FFFMode, FilePicker, FilePickerOptions, FuzzySearchOptions, GrepMode, GrepSearchOptions,
    PaginationArgs, QueryParser, ScanProgress, SharedFilePicker, SharedFrecency,
};

#[derive(Debug)]
pub enum SearchError {
    FilePicker(fff_search::Error),
    Io(std::io::Error),
}

impl Display for SearchError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FilePicker(error) => Display::fmt(error, formatter),
            Self::Io(error) => Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for SearchError {}

impl From<fff_search::Error> for SearchError {
    fn from(error: fff_search::Error) -> Self {
        Self::FilePicker(error)
    }
}

impl From<std::io::Error> for SearchError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

struct Picker {
    shared: SharedFilePicker,
    frecency: SharedFrecency,
}

pub struct SearchService {
    root: Picker,
    extract_cache: Picker,
}

#[derive(Debug)]
pub struct FileSearchResults {
    pub items: Vec<FileSearchItem>,
    pub total_matched: u32,
}

#[derive(Debug)]
pub struct FileSearchItem {
    pub relative_path: String,
    pub file_name: String,
}

#[derive(Debug)]
pub struct ContentSearchResults {
    pub items: Vec<ContentSearchItem>,
    pub total_matched: u32,
}

#[derive(Debug)]
pub struct ContentSearchItem {
    pub relative_path: String,
    pub line_number: u64,
    pub line_content: String,
}

impl SearchService {
    pub fn new(
        root_path: impl AsRef<Path>,
        extract_cache_path: impl AsRef<Path>,
    ) -> Result<Self, SearchError> {
        std::fs::create_dir_all(extract_cache_path.as_ref())?;

        Ok(Self {
            root: Picker::new(root_path)?,
            extract_cache: Picker::new(extract_cache_path)?,
        })
    }

    pub fn wait_for_scan(&self, timeout: Duration) -> bool {
        let root_ready = self.root.wait_for_scan(timeout);
        let extract_ready = self.extract_cache.wait_for_scan(timeout);
        root_ready && extract_ready
    }

    pub fn scan_progress(&self) -> Result<ScanProgress, SearchError> {
        self.root.scan_progress()
    }

    pub fn trigger_rescan(&self) -> Result<(), SearchError> {
        self.root.trigger_rescan()?;
        self.extract_cache.trigger_rescan()
    }

    pub fn file_search(
        &self,
        raw_query: &str,
        limit: u32,
    ) -> Result<FileSearchResults, SearchError> {
        self.root.file_search(raw_query, limit)
    }

    pub fn content_search(
        &self,
        raw_query: &str,
        mode: GrepMode,
        limit: u32,
    ) -> Result<ContentSearchResults, SearchError> {
        let native = self.root.content_search(raw_query, mode, limit)?;
        let extracted = self.extract_cache.content_search(raw_query, mode, limit)?;
        let total_matched = native
            .total_matched
            .checked_add(extracted.total_matched)
            .expect("combined FFF result count must fit in u32");

        let mut items = native.items;
        items.extend(extracted.items.into_iter().map(remap_extract_path));
        items.truncate(limit as usize);

        Ok(ContentSearchResults {
            items,
            total_matched,
        })
    }
}

impl Picker {
    fn new(base_path: impl AsRef<Path>) -> Result<Self, SearchError> {
        let shared = SharedFilePicker::default();
        let frecency = SharedFrecency::default();
        let options = FilePickerOptions {
            base_path: base_path.as_ref().to_string_lossy().into_owned(),
            enable_mmap_cache: false,
            enable_content_indexing: false,
            mode: FFFMode::Ai,
            cache_budget: None,
            watch: true,
            follow_symlinks: false,
            enable_fs_root_scanning: false,
            enable_home_dir_scanning: false,
        };

        FilePicker::new_with_shared_state(shared.clone(), frecency.clone(), options)?;

        Ok(Self { shared, frecency })
    }

    fn wait_for_scan(&self, timeout: Duration) -> bool {
        self.shared.wait_for_scan(timeout)
    }

    fn scan_progress(&self) -> Result<ScanProgress, SearchError> {
        let guard = self.shared.read()?;
        let picker = guard.as_ref().ok_or(fff_search::Error::FilePickerMissing)?;

        Ok(picker.get_scan_progress())
    }

    fn trigger_rescan(&self) -> Result<(), SearchError> {
        self.shared
            .trigger_full_rescan_async(&self.frecency)
            .map_err(SearchError::from)
    }

    fn file_search(&self, raw_query: &str, limit: u32) -> Result<FileSearchResults, SearchError> {
        let query = QueryParser::default().parse(raw_query);
        let guard = self.shared.read()?;
        let picker = guard.as_ref().ok_or(fff_search::Error::FilePickerMissing)?;
        let result = picker.fuzzy_search(
            &query,
            None,
            FuzzySearchOptions {
                pagination: PaginationArgs {
                    offset: 0,
                    limit: limit as usize,
                },
                ..Default::default()
            },
        );
        let items = result
            .items
            .into_iter()
            .map(|file| FileSearchItem {
                relative_path: file.relative_path(picker),
                file_name: file.file_name(picker),
            })
            .collect();

        Ok(FileSearchResults {
            items,
            total_matched: result
                .total_matched
                .try_into()
                .expect("FFF count must fit in u32"),
        })
    }

    fn content_search(
        &self,
        raw_query: &str,
        mode: GrepMode,
        limit: u32,
    ) -> Result<ContentSearchResults, SearchError> {
        let query = QueryParser::default().parse(raw_query);
        let guard = self.shared.read()?;
        let picker = guard.as_ref().ok_or(fff_search::Error::FilePickerMissing)?;
        let result = picker.grep(
            &query,
            &GrepSearchOptions {
                mode,
                page_limit: limit as usize,
                ..Default::default()
            },
        );
        let mut items = Vec::with_capacity(result.matches.len());

        for matched in result.matches {
            assert!(matched.file_index < result.files.len());
            let file = result.files[matched.file_index];
            items.push(ContentSearchItem {
                relative_path: file.relative_path(picker),
                line_number: matched.line_number,
                line_content: matched.line_content,
            });
        }

        Ok(ContentSearchResults {
            total_matched: items.len().try_into().expect("FFF count must fit in u32"),
            items,
        })
    }
}

fn remap_extract_path(mut item: ContentSearchItem) -> ContentSearchItem {
    if let Some(original) = item.relative_path.strip_suffix(".txt") {
        item.relative_path = original.to_owned();
    }
    item
}
