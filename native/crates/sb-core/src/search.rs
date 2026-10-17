//! Command palette matching: every word of the query must appear; a match at the start scores 3, after a space 2,
//! anywhere else 1. An empty query matches everything with 1.

pub fn score(text: &str, query: &str) -> u32 {
    let q = query.trim();
    if q.is_empty() {
        return 1;
    }
    let t = text.to_lowercase();
    let mut total = 0;
    for w in q.to_lowercase().split_whitespace() {
        let Some(i) = t.find(w) else { return 0 };
        total += if i == 0 {
            3
        } else if t[..i].chars().last().is_some_and(char::is_whitespace) {
            2
        } else {
            1
        };
    }
    total
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn scores() {
        assert_eq!(score("Folder audit Studio", ""), 1);
        assert_eq!(score("Folder audit Studio", "fold"), 3);
        assert_eq!(score("Folder audit Studio", "aud"), 2);
        assert_eq!(score("Folder audit Studio", "dio"), 1);
        assert_eq!(score("Folder audit Studio", "fold studio"), 5);
        assert_eq!(score("Folder audit Studio", "fold windows"), 0);
    }
}
