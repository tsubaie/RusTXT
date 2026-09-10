use regex::{Regex, RegexBuilder};
use std::ops::Range;

#[derive(Default)]
pub struct Search {
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub matches: Vec<Range<usize>>,
    pub error: Option<String>,
}

impl Search {
    pub fn expression(&self) -> Result<Regex, String> {
        let pattern = if self.regex {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        let pattern = if self.whole_word {
            format!(r"\b(?:{pattern})\b")
        } else {
            pattern
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| e.to_string())
    }
    pub fn refresh(&mut self, text: &str) {
        self.matches.clear();
        self.error = None;
        if self.query.is_empty() {
            return;
        }
        match self.expression() {
            Ok(regex) => self.matches = regex.find_iter(text).map(|m| m.range()).collect(),
            Err(error) => self.error = Some(error),
        }
    }
    pub fn next(&self, selection: Range<usize>, backwards: bool) -> Option<Range<usize>> {
        if backwards {
            self.matches
                .iter()
                .rev()
                .find(|m| m.start < selection.start)
                .or(self.matches.last())
                .cloned()
        } else {
            self.matches
                .iter()
                .find(|m| m.start >= selection.end && **m != selection)
                .or(self.matches.first())
                .cloned()
        }
    }
    pub fn replacement_for(&self, text: &str, range: Range<usize>) -> String {
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
    pub fn replace_all(&self, text: &str) -> Result<String, String> {
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
        search.query = "(hi)".into();
        search.regex = true;
        search.replacement = "$1!".into();
        assert_eq!(search.replace_all("hi hi").unwrap(), "hi! hi!");
        search.regex = false;
        search.query = "hi".into();
        assert_eq!(search.replace_all("hi").unwrap(), "$1!");
    }
}
