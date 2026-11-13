use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReadingProgressSource {
    pub path: String,
    pub size_bytes: u64,
    pub modified_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ReadingProgress {
    pub schema_version: u32,
    pub comicbook_id: i64,
    pub source: ReadingProgressSource,
    pub total_pages: usize,
    pub last_page_index: Option<usize>,
    pub visited_page_indexes: Vec<usize>,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

impl Default for ReadingProgress {
    fn default() -> Self {
        Self {
            schema_version: 1,
            comicbook_id: 0,
            source: ReadingProgressSource::default(),
            total_pages: 0,
            last_page_index: None,
            visited_page_indexes: Vec::new(),
            completed_at: None,
            updated_at: String::new(),
        }
    }
}

impl ReadingProgress {
    pub fn normalize(&mut self) {
        self.visited_page_indexes
            .retain(|index| *index < self.total_pages);
        self.visited_page_indexes.sort_unstable();
        self.visited_page_indexes.dedup();
        self.last_page_index = self
            .last_page_index
            .filter(|index| *index < self.total_pages);
        if self.total_pages == 0 || self.visited_page_indexes.len() < self.total_pages {
            self.completed_at = None;
        }
    }

    pub fn is_complete(&self) -> bool {
        self.total_pages > 0 && self.visited_page_indexes.len() == self.total_pages
    }
}
