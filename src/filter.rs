//! Line filter ("log viewer"): show only the lines that match (or don't
//! match) a text or regex. Matching runs on a background thread; the editor
//! then lays out just those lines.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::document::DocId;
use crate::search::{SearchMode, SearchOptions, build_regex};

/// What the user asked to filter by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterQuery {
    pub text: String,
    pub regex: bool,
    pub match_case: bool,
    /// Hide the matching lines instead of showing only them.
    pub exclude: bool,
}

/// Result of a filter run: the lines to show, out of how many.
#[derive(Clone, Debug)]
pub struct FilterResult {
    pub lines: Arc<Vec<usize>>,
    pub total: usize,
}

/// A background filter run: (document, version, query, result).
pub type FilterJob = (
    DocId,
    u64,
    FilterQuery,
    Receiver<Result<FilterResult, String>>,
);

#[derive(Default)]
pub struct FilterState {
    pub open: bool,
    pub focus: bool,
    pub query: FilterQuery,
    /// (document, version, query) the current filter was computed for.
    pub applied: Option<(DocId, u64, FilterQuery)>,
    pub job: Option<FilterJob>,
    pub last: Option<FilterResult>,
    pub error: Option<String>,
    /// When the query or document last changed (for debouncing).
    pub changed_at: f64,
}

/// Line numbers (0-based) of `text` that pass the filter.
pub fn filter_lines(text: &str, q: &FilterQuery) -> Result<FilterResult, String> {
    let opts = SearchOptions {
        mode: if q.regex {
            SearchMode::Regex
        } else {
            SearchMode::Normal
        },
        match_case: q.match_case,
        ..Default::default()
    };
    let re = build_regex(&q.text, &opts)?;
    let mut lines = Vec::new();
    let mut total = 0;
    for (i, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let hit = re.is_match(line).map_err(|e| e.to_string())?;
        if hit != q.exclude {
            lines.push(i);
        }
        total = i + 1;
    }
    Ok(FilterResult {
        lines: Arc::new(lines),
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str, regex: bool, exclude: bool) -> FilterQuery {
        FilterQuery {
            text: text.into(),
            regex,
            match_case: false,
            exclude,
        }
    }

    #[test]
    fn filters_lines() {
        let log = "10:00 INFO start\r\n10:01 ERROR disk full\n10:02 warn slow\n10:03 error retry";
        let r = filter_lines(log, &q("error", false, false)).unwrap();
        assert_eq!(*r.lines, vec![1, 3]);
        assert_eq!(r.total, 4);
        let r = filter_lines(log, &q("error", false, true)).unwrap();
        assert_eq!(*r.lines, vec![0, 2]);
        let r = filter_lines(log, &q(r"^10:0[23]", true, false)).unwrap();
        assert_eq!(*r.lines, vec![2, 3]);
        let mut case = q("ERROR", false, false);
        case.match_case = true;
        assert_eq!(*filter_lines(log, &case).unwrap().lines, vec![1]);
        // `$` matches at the end of each line, CR LF or LF.
        let r = filter_lines(log, &q(r"start$", true, false)).unwrap();
        assert_eq!(*r.lines, vec![0]);
        assert!(filter_lines(log, &q("(", true, false)).is_err());
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore = "benchmark: BENCH_FILE=path cargo test --release filter_bench -- --ignored --nocapture"]
    fn filter_bench() {
        let text =
            std::fs::read_to_string(std::env::var("BENCH_FILE").expect("BENCH_FILE")).unwrap();
        for (q, regex) in [
            ("worker-3]", false),
            (r"status=(4|5)\d\d", true),
            (r"dur=\d{3}ms$", true),
        ] {
            let query = super::FilterQuery {
                text: q.into(),
                regex,
                ..Default::default()
            };
            let t = std::time::Instant::now();
            let r = super::filter_lines(&text, &query).unwrap();
            println!(
                "{q:<22} {:>7} of {} lines in {:?}",
                r.lines.len(),
                r.total,
                t.elapsed()
            );
        }
    }
}
