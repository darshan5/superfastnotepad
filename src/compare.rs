//! File compare: line diff, alignment and in-line word differences.
//!
//! The two documents are shown side by side, with spacer rows that keep
//! matching lines level.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use similar::{Algorithm, ChangeTag, DiffOp};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct CompareOptions {
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub ignore_empty_lines: bool,
    /// Highlight changed words inside changed lines.
    pub word_diff: bool,
    /// Re-compare automatically after edits.
    pub auto: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            ignore_whitespace: false,
            ignore_case: false,
            ignore_empty_lines: false,
            word_diff: true,
            auto: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Same,
    /// Only in the left document.
    Removed,
    /// Only in the right document.
    Added,
    /// Present on both sides, but different.
    Changed,
}

/// One difference, in line numbers of each document. An empty range means
/// "nothing on this side": the lines go before `range.start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub left: Range<usize>,
    pub right: Range<usize>,
}

/// What one editor view needs to draw its side of a compare.
#[derive(Clone, Debug)]
pub struct SideDecor {
    /// Unique per computation (layout cache key).
    pub generation: u64,
    /// Document version this was computed for.
    pub version: u64,
    pub kinds: Vec<LineKind>,
    /// Spacer rows drawn before each line; index `n` = after the last line.
    pub spacers_before: Vec<u32>,
    /// Changed character ranges (char offsets in the line).
    pub inline: HashMap<usize, Vec<(usize, usize)>>,
    /// Runs of changed lines `(first, last_exclusive, kind)` for scrollbar marks.
    pub runs: Vec<(usize, usize, LineKind)>,
}

impl SideDecor {
    /// Whether this decoration still matches the document.
    pub fn valid_for(&self, version: u64, lines: usize) -> bool {
        self.version == version && self.kinds.len() == lines
    }
}

#[derive(Clone, Debug)]
pub struct CompareResult {
    pub hunks: Vec<Hunk>,
    pub left: SideDecor,
    pub right: SideDecor,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

/// Inputs larger than this are refused (diffing would freeze the UI).
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// In-line word highlighting is skipped for longer lines.
const MAX_INLINE_LINE: usize = 32 * 1024;

static GENERATION: AtomicU64 = AtomicU64::new(1);

fn normalize(line: &str, o: &CompareOptions) -> String {
    let s: String = if o.ignore_whitespace {
        line.chars().filter(|c| !c.is_whitespace()).collect()
    } else {
        line.to_string()
    };
    if o.ignore_case { s.to_lowercase() } else { s }
}

/// Changed char ranges of `old` and `new` from a word-level diff.
fn inline_ranges(
    old: &str,
    new: &str,
    o: &CompareOptions,
) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    if old.len() > MAX_INLINE_LINE || new.len() > MAX_INLINE_LINE {
        return (Vec::new(), Vec::new());
    }
    let (a, b) = if o.ignore_case {
        (old.to_lowercase(), new.to_lowercase())
    } else {
        (old.to_string(), new.to_string())
    };
    // Case folding can change lengths (e.g. 'İ'); then positions wouldn't map back.
    if a.chars().count() != old.chars().count() || b.chars().count() != new.chars().count() {
        return (Vec::new(), Vec::new());
    }
    let mut lo = Vec::new();
    let mut ro = Vec::new();
    let (mut li, mut ri) = (0usize, 0usize);
    for (tag, piece) in similar::utils::diff_unicode_words(Algorithm::Myers, a.as_str(), b.as_str())
    {
        let n = piece.chars().count();
        let ws_only = o.ignore_whitespace && piece.trim().is_empty();
        match tag {
            ChangeTag::Equal => {
                li += n;
                ri += n;
            }
            ChangeTag::Delete => {
                if !ws_only {
                    push_merged(&mut lo, li, li + n);
                }
                li += n;
            }
            ChangeTag::Insert => {
                if !ws_only {
                    push_merged(&mut ro, ri, ri + n);
                }
                ri += n;
            }
        }
    }
    (lo, ro)
}

fn push_merged(v: &mut Vec<(usize, usize)>, s: usize, e: usize) {
    if let Some(last) = v.last_mut()
        && last.1 == s
    {
        last.1 = e;
        return;
    }
    v.push((s, e));
}

fn runs_of(kinds: &[LineKind]) -> Vec<(usize, usize, LineKind)> {
    let mut out: Vec<(usize, usize, LineKind)> = Vec::new();
    for (i, &k) in kinds.iter().enumerate() {
        if k == LineKind::Same {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.1 == i
            && last.2 == k
        {
            last.1 = i + 1;
            continue;
        }
        out.push((i, i + 1, k));
    }
    out
}

/// Compares two documents given as lines (without terminators).
pub fn compare(
    left: &[String],
    right: &[String],
    left_version: u64,
    right_version: u64,
    o: &CompareOptions,
) -> Result<CompareResult, String> {
    let bytes: usize = left.iter().chain(right).map(String::len).sum();
    if bytes > MAX_BYTES {
        return Err(format!(
            "The documents are too large to compare ({} MB, the limit is {} MB).",
            bytes / 1_000_000,
            MAX_BYTES / 1_000_000
        ));
    }
    let norm_l: Vec<String> = left.iter().map(|l| normalize(l, o)).collect();
    let norm_r: Vec<String> = right.iter().map(|l| normalize(l, o)).collect();
    // Indices of the lines that take part in the diff.
    let keep = |norm: &[String]| -> Vec<usize> {
        (0..norm.len())
            .filter(|&i| !o.ignore_empty_lines || !norm[i].trim().is_empty())
            .collect()
    };
    let (map_l, map_r) = (keep(&norm_l), keep(&norm_r));
    let seq_l: Vec<&str> = map_l.iter().map(|&i| norm_l[i].as_str()).collect();
    let seq_r: Vec<&str> = map_r.iter().map(|&i| norm_r[i].as_str()).collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    let ops =
        similar::capture_diff_slices_deadline(Algorithm::Patience, &seq_l, &seq_r, Some(deadline));

    // Filtered index -> start line in the original document.
    let bound = |map: &[usize], total: usize, k: usize| if k < map.len() { map[k] } else { total };
    let mut kinds_l = vec![LineKind::Same; left.len()];
    let mut kinds_r = vec![LineKind::Same; right.len()];
    let mut sp_l = vec![0u32; left.len() + 1];
    let mut sp_r = vec![0u32; right.len() + 1];
    let mut inline_l = HashMap::new();
    let mut inline_r = HashMap::new();
    let mut hunks = Vec::new();
    let (mut added, mut removed, mut changed) = (0, 0, 0);
    let (mut cur_l, mut cur_r) = (0usize, 0usize);
    let n_ops = ops.len();
    for (oi, op) in ops.iter().enumerate() {
        let (a0, a_len, b0, b_len) = match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => (old_index, len, new_index, len),
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index, old_len, new_index, 0),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index, 0, new_index, new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (old_index, old_len, new_index, new_len),
        };
        // Segment in original lines; the last op extends to the end of each document.
        let last = oi + 1 == n_ops;
        let l_end = if last {
            left.len()
        } else {
            bound(&map_l, left.len(), a0 + a_len)
        };
        let r_end = if last {
            right.len()
        } else {
            bound(&map_r, right.len(), b0 + b_len)
        };
        let (l_rng, r_rng) = (cur_l..l_end.max(cur_l), cur_r..r_end.max(cur_r));
        let counts = |k: &[LineKind], r: &Range<usize>, norm: &[String]| {
            r.clone()
                .filter(|&i| {
                    !(o.ignore_empty_lines && norm[i].trim().is_empty()) && k[i] != LineKind::Same
                })
                .count()
        };
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete { .. } => {
                for i in l_rng.clone() {
                    if !(o.ignore_empty_lines && norm_l[i].trim().is_empty()) {
                        kinds_l[i] = LineKind::Removed;
                    }
                }
                removed += counts(&kinds_l, &l_rng, &norm_l);
            }
            DiffOp::Insert { .. } => {
                for i in r_rng.clone() {
                    if !(o.ignore_empty_lines && norm_r[i].trim().is_empty()) {
                        kinds_r[i] = LineKind::Added;
                    }
                }
                added += counts(&kinds_r, &r_rng, &norm_r);
            }
            DiffOp::Replace { .. } => {
                let pairs = l_rng.len().min(r_rng.len());
                for k in 0..l_rng.len() {
                    kinds_l[l_rng.start + k] = if k < pairs {
                        LineKind::Changed
                    } else {
                        LineKind::Removed
                    };
                }
                for k in 0..r_rng.len() {
                    kinds_r[r_rng.start + k] = if k < pairs {
                        LineKind::Changed
                    } else {
                        LineKind::Added
                    };
                }
                changed += pairs;
                removed += l_rng.len() - pairs;
                added += r_rng.len() - pairs;
                if o.word_diff {
                    for k in 0..pairs {
                        let (li, ri) = (l_rng.start + k, r_rng.start + k);
                        let (lr, rr) = inline_ranges(&left[li], &right[ri], o);
                        if !lr.is_empty() {
                            inline_l.insert(li, lr);
                        }
                        if !rr.is_empty() {
                            inline_r.insert(ri, rr);
                        }
                    }
                }
            }
        }
        if !matches!(op, DiffOp::Equal { .. }) {
            hunks.push(Hunk {
                left: l_rng.clone(),
                right: r_rng.clone(),
            });
        }
        // Keep both sides level: pad the shorter segment after its last line.
        let (ll, rl) = (l_rng.len(), r_rng.len());
        if ll > rl {
            sp_r[r_rng.end] += u32::try_from(ll - rl).unwrap_or(u32::MAX);
        } else if rl > ll {
            sp_l[l_rng.end] += u32::try_from(rl - ll).unwrap_or(u32::MAX);
        }
        cur_l = l_rng.end;
        cur_r = r_rng.end;
    }
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    Ok(CompareResult {
        hunks,
        left: SideDecor {
            generation,
            version: left_version,
            runs: runs_of(&kinds_l),
            kinds: kinds_l,
            spacers_before: sp_l,
            inline: inline_l,
        },
        right: SideDecor {
            generation: generation + 1_000_000_000,
            version: right_version,
            runs: runs_of(&kinds_r),
            kinds: kinds_r,
            spacers_before: sp_r,
            inline: inline_r,
        },
        added,
        removed,
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use LineKind::{Added, Changed, Removed, Same};

    fn lines(s: &str) -> Vec<String> {
        s.split('\n').map(str::to_string).collect()
    }

    #[test]
    fn identical_documents() {
        let a = lines("a\nb\nc");
        let r = compare(&a, &a, 1, 1, &CompareOptions::default()).unwrap();
        assert!(r.hunks.is_empty());
        assert!(r.left.kinds.iter().all(|k| *k == Same));
        assert!(r.left.spacers_before.iter().all(|&s| s == 0));
    }

    #[test]
    fn added_removed_changed_and_alignment() {
        let a = lines("one\ntwo\nthree\nfour");
        let b = lines("one\nTWO\nthree\nextra 1\nextra 2\nfour");
        let r = compare(&a, &b, 1, 1, &CompareOptions::default()).unwrap();
        assert_eq!(r.left.kinds, vec![Same, Changed, Same, Same]);
        assert_eq!(r.right.kinds, vec![Same, Changed, Same, Added, Added, Same]);
        // Two spacer rows on the left before "four" keep it level with the right.
        assert_eq!(r.left.spacers_before[3], 2);
        assert_eq!(
            r.hunks,
            vec![
                Hunk {
                    left: 1..2,
                    right: 1..2
                },
                Hunk {
                    left: 3..3,
                    right: 3..5
                }
            ]
        );
        assert_eq!((r.added, r.removed, r.changed), (2, 0, 1));

        let r2 = compare(&b, &a, 1, 1, &CompareOptions::default()).unwrap();
        assert_eq!(r2.left.kinds[3], Removed);
        assert_eq!(r2.right.spacers_before[3], 2);
    }

    #[test]
    fn word_level_ranges() {
        let a = lines("let total = price * qty;");
        let b = lines("let total = cost * qty;");
        let r = compare(&a, &b, 1, 1, &CompareOptions::default()).unwrap();
        assert_eq!(r.left.inline.get(&0), Some(&vec![(12, 17)]));
        assert_eq!(r.right.inline.get(&0), Some(&vec![(12, 16)]));
    }

    #[test]
    fn ignore_options() {
        let a = lines("Hello  World\n\nend");
        let b = lines("hello world\nend");
        let strict = compare(&a, &b, 1, 1, &CompareOptions::default()).unwrap();
        assert!(!strict.hunks.is_empty());
        let loose = CompareOptions {
            ignore_case: true,
            ignore_whitespace: true,
            ignore_empty_lines: true,
            ..Default::default()
        };
        let r = compare(&a, &b, 1, 1, &loose).unwrap();
        assert!(r.hunks.is_empty(), "{:?}", r.hunks);
        assert!(r.left.kinds.iter().all(|k| *k == Same));
        // Lines stay aligned even though the left has an extra (ignored) blank line.
        assert_eq!(r.right.spacers_before.iter().sum::<u32>(), 1);
    }

    #[test]
    fn insertion_at_start_and_end() {
        let a = lines("x");
        let b = lines("new\nx\ntail");
        let r = compare(&a, &b, 1, 1, &CompareOptions::default()).unwrap();
        assert_eq!(r.left.spacers_before, vec![1, 1]);
        assert_eq!(r.right.kinds, vec![Added, Same, Added]);
        assert_eq!(r.left.runs.len(), 0);
        assert_eq!(r.right.runs, vec![(0, 1, Added), (2, 3, Added)]);
    }
}
