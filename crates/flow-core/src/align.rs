//! Word-level alignment (Levenshtein with backtrace), labelled like Wispr's
//! `parseAlignmentSteps`: C = correct, S = substitution, I = insertion
//! (word only in the edited text), D = deletion (word only in the original).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    C,
    S,
    I,
    D,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub op: Op,
    /// Word in the original (pasted) text; `None` for insertions.
    pub from: Option<String>,
    /// Word in the edited text; `None` for deletions.
    pub to: Option<String>,
}

/// Normalized comparison key: lowercase, surrounding punctuation trimmed.
pub fn norm_word(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn tokens(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

/// Align with exact (case-sensitive) word equality, so a casing fix like
/// `wispr -> Wispr` shows up as a substitution.
pub fn align_words(a: &str, b: &str) -> Vec<Step> {
    let (a, b) = (tokens(a), tokens(b));
    let key = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let ak: Vec<String> = a.iter().map(|w| key(w)).collect();
    let bk: Vec<String> = b.iter().map(|w| key(w)).collect();
    let dp = table(&ak, &bk);
    let (mut i, mut j) = (a.len(), b.len());
    let mut out = Vec::new();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && ak[i - 1] == bk[j - 1] && dp[i][j] == dp[i - 1][j - 1] {
            out.push(Step {
                op: Op::C,
                from: Some(a[i - 1].into()),
                to: Some(b[j - 1].into()),
            });
            i -= 1;
            j -= 1;
        } else if i > 0 && j > 0 && dp[i][j] == dp[i - 1][j - 1] + 1 {
            out.push(Step {
                op: Op::S,
                from: Some(a[i - 1].into()),
                to: Some(b[j - 1].into()),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && dp[i][j] == dp[i][j - 1] + 1 {
            out.push(Step {
                op: Op::I,
                from: None,
                to: Some(b[j - 1].into()),
            });
            j -= 1;
        } else {
            out.push(Step {
                op: Op::D,
                from: Some(a[i - 1].into()),
                to: None,
            });
            i -= 1;
        }
    }
    out.reverse();
    out
}

fn table(a: &[String], b: &[String]) -> Vec<Vec<usize>> {
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let sub = dp[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]);
            dp[i][j] = sub.min(dp[i - 1][j] + 1).min(dp[i][j - 1] + 1);
        }
    }
    dp
}

/// Word edit distance on normalized words (case/punctuation-insensitive).
pub fn word_edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<String> = tokens(a)
        .into_iter()
        .map(norm_word)
        .filter(|w| !w.is_empty())
        .collect();
    let b: Vec<String> = tokens(b)
        .into_iter()
        .map(norm_word)
        .filter(|w| !w.is_empty())
        .collect();
    table(&a, &b)[a.len()][b.len()]
}

/// Edit distance divided by the longer word count (0 = same, 1 = disjoint).
pub fn divergence_ratio(a: &str, b: &str) -> f64 {
    let n = tokens(a).len().max(tokens(b).len());
    if n == 0 {
        return 0.0;
    }
    word_edit_distance(a, b) as f64 / n as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use Op::*;

    fn ops(a: &str, b: &str) -> Vec<Op> {
        align_words(a, b).into_iter().map(|s| s.op).collect()
    }

    #[test]
    fn identical_is_all_correct() {
        assert_eq!(ops("a b c", "a b c"), vec![C, C, C]);
    }

    #[test]
    fn single_substitution() {
        assert_eq!(ops("ask whisper flow", "ask Wispr flow"), vec![C, S, C]);
    }

    #[test]
    fn insertion_and_deletion() {
        assert_eq!(ops("a c", "a b c"), vec![C, I, C]);
        assert_eq!(ops("a b c", "a c"), vec![C, D, C]);
    }

    #[test]
    fn steps_carry_words() {
        let s = align_words("hello world", "hello there");
        assert_eq!(s[1].from.as_deref(), Some("world"));
        assert_eq!(s[1].to.as_deref(), Some("there"));
    }

    #[test]
    fn edit_distance_counts_word_edits() {
        assert_eq!(word_edit_distance("a b c", "a x c d"), 2);
        assert_eq!(word_edit_distance("", "a b"), 2);
    }

    #[test]
    fn divergence_ratio_is_normalized() {
        assert_eq!(divergence_ratio("a b c d", "a b c d"), 0.0);
        assert!((divergence_ratio("a b c d", "a b x d") - 0.25).abs() < 1e-9);
        assert_eq!(divergence_ratio("", ""), 0.0);
        assert_eq!(divergence_ratio("", "x"), 1.0);
    }

    #[test]
    fn divergence_ignores_case_and_punctuation() {
        assert_eq!(divergence_ratio("hello, world", "Hello world."), 0.0);
    }
}
