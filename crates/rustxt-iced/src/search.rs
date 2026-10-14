use regex::{Regex, RegexBuilder};
use std::ops::Range;

/// Matches kept per refresh; the count label reads "10 000+" beyond this.
pub const MAX_MATCHES: usize = 10_000;
const SIZE_LIMIT: usize = 1024 * 1024;

#[derive(Default)]
pub struct Search {
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub matches: Vec<Range<usize>>,
    /// More than `MAX_MATCHES` matched; only the first `MAX_MATCHES` are kept.
    pub truncated: bool,
    /// Index into `matches` of the match equal to the editor selection.
    pub current: Option<usize>,
    pub error: Option<String>,
    compiled: Option<Compiled>,
    #[cfg(test)]
    compile_count: usize,
}

struct Compiled {
    key: (String, bool, bool, bool),
    result: Result<Regex, String>,
}

impl Search {
    /// The compiled pattern, rebuilt only when the query or options changed.
    pub fn expression(&mut self) -> Result<Regex, String> {
        let key = (
            self.query.clone(),
            self.case_sensitive,
            self.whole_word,
            self.regex,
        );
        if self.compiled.as_ref().map(|c| &c.key) != Some(&key) {
            let result = self.compile();
            self.compiled = Some(Compiled { key, result });
            #[cfg(test)]
            {
                self.compile_count += 1;
            }
        }
        self.compiled
            .as_ref()
            .map(|c| c.result.clone())
            .expect("cached")
    }
    fn compile(&self) -> Result<Regex, String> {
        let build = |pattern: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(!self.case_sensitive)
                .multi_line(true)
                .size_limit(SIZE_LIMIT)
                .build()
                .map_err(|e| e.to_string())
        };
        let pattern = if self.regex {
            // Validate the user's pattern on its own first: wrapping an
            // unbalanced pattern such as `a)|(b` could otherwise compile
            // into something they never wrote.
            build(&self.query)?;
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        if self.whole_word {
            build(&format!(r"\b(?:{pattern})\b"))
        } else {
            build(&pattern)
        }
    }
    pub fn refresh(&mut self, text: &str) {
        self.matches.clear();
        self.truncated = false;
        self.current = None;
        self.error = None;
        if self.query.is_empty() {
            return;
        }
        match self.expression() {
            Ok(regex) => {
                self.matches = regex
                    .find_iter(text)
                    .map(|m| m.range())
                    .take(MAX_MATCHES + 1)
                    .collect();
                if self.matches.len() > MAX_MATCHES {
                    self.matches.truncate(MAX_MATCHES);
                    self.truncated = true;
                }
            }
            Err(error) => self.error = Some(error),
        }
    }
    /// Record which match, if any, the editor selection sits on.
    pub fn locate(&mut self, selection: &Range<usize>) {
        self.current = self
            .matches
            .binary_search_by(|m| {
                m.start
                    .cmp(&selection.start)
                    .then(m.end.cmp(&selection.end))
            })
            .ok();
    }
    /// Count label for the find bar: "3 of 12", "12", "No results" or "10 000+".
    pub fn count_label(&self) -> String {
        if self.error.is_some() {
            return "!".into();
        }
        if self.query.is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            return "No results".into();
        }
        let total = if self.truncated {
            format!("{}+", group_thousands(MAX_MATCHES))
        } else {
            group_thousands(self.matches.len())
        };
        match self.current {
            Some(index) => format!("{} of {total}", index + 1),
            None => total,
        }
    }
    pub fn next(&self, selection: Range<usize>, backwards: bool) -> Option<Range<usize>> {
        if backwards {
            let before = self.matches.partition_point(|m| m.start < selection.start);
            self.matches[..before]
                .last()
                .or(self.matches.last())
                .cloned()
        } else {
            let from = self.matches.partition_point(|m| m.start < selection.end);
            self.matches[from..]
                .iter()
                .find(|m| **m != selection)
                .or(self.matches.first())
                .cloned()
        }
    }
    pub fn replacement_for(&mut self, text: &str, range: Range<usize>) -> String {
        if !self.regex {
            return self.replacement.clone();
        }
        let Ok(regex) = self.expression() else {
            return self.replacement.clone();
        };
        let Some(captures) = regex.captures_at(text, range.start) else {
            return self.replacement.clone();
        };
        let mut result = String::new();
        captures.expand(&self.replacement, &mut result);
        result
    }
    pub fn replace_all(&mut self, text: &str) -> Result<String, String> {
        let regex = self.expression()?;
        if self.regex {
            Ok(regex
                .replace_all(text, self.replacement.as_str())
                .into_owned())
        } else {
            Ok(regex
                .replace_all(text, regex::NoExpand(&self.replacement))
                .into_owned())
        }
    }
}

fn group_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_whole_words_regex_groups_and_wrapping() {
        let mut search = Search {
            query: "مرحبا".into(),
            whole_word: true,
            ..Search::default()
        };
        search.refresh("مرحبا مرحباب hi مرحبا");
        assert_eq!(search.matches.len(), 2);
        assert_eq!(
            search.next(search.matches[1].clone(), false),
            Some(search.matches[0].clone())
        );
        assert_eq!(
            search.next(search.matches[0].clone(), true),
            Some(search.matches[1].clone())
        );
        search.query = "(hi)".into();
        search.regex = true;
        search.replacement = "$1!".into();
        assert_eq!(search.replace_all("hi hi").unwrap(), "hi! hi!");
        search.regex = false;
        search.query = "hi".into();
        assert_eq!(search.replace_all("hi").unwrap(), "$1!");
    }
    #[test]
    fn compiled_pattern_is_cached_until_the_query_or_options_change() {
        let mut search = Search {
            query: "a".into(),
            ..Search::default()
        };
        search.expression().unwrap();
        search.expression().unwrap();
        search.refresh("a");
        assert_eq!(search.compile_count, 1);
        search.case_sensitive = true;
        search.expression().unwrap();
        assert_eq!(search.compile_count, 2);
        search.query = "b".into();
        search.refresh("b");
        assert_eq!(search.compile_count, 3);
    }
    #[test]
    fn whole_word_regex_reports_the_users_own_pattern_error() {
        let mut search = Search {
            query: "a)|(b".into(),
            regex: true,
            whole_word: true,
            ..Search::default()
        };
        search.refresh("a b");
        assert!(search.error.is_some(), "unbalanced pattern must not match");
        assert!(search.matches.is_empty());
        search.query = "a|b".into();
        search.refresh("a b ab");
        assert_eq!(search.matches.len(), 2);
    }
    #[test]
    fn match_count_is_capped_and_current_match_is_tracked() {
        let mut search = Search {
            query: "x".into(),
            ..Search::default()
        };
        let text = "x".repeat(MAX_MATCHES + 5);
        search.refresh(&text);
        assert_eq!(search.matches.len(), MAX_MATCHES);
        assert!(search.truncated);
        assert_eq!(search.count_label(), "10 000+");
        search.locate(&(2..3));
        assert_eq!(search.current, Some(2));
        assert_eq!(search.count_label(), "3 of 10 000+");
        search.refresh("x x x");
        search.locate(&(1..2));
        assert_eq!(search.current, None);
        assert_eq!(search.count_label(), "3");
        search.locate(&(4..5));
        assert_eq!(search.count_label(), "3 of 3");
        search.query.clear();
        search.refresh("x");
        assert_eq!(search.count_label(), "");
        assert_eq!(group_thousands(1234567), "1 234 567");
    }
}
