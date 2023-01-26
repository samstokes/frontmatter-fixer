const RULE_LENGTH: usize = "---\n".len();

pub fn parse_raw(s: &str) -> (Option<&str>, &str) {
    // first line must begin frontmatter if present
    let mut rules = s.match_indices("---\n");
    if let Some((0, _)) = rules.next() {
        let start = RULE_LENGTH;
        if let Some((close, _)) = rules.next() {
            assert!(start <= close);

            let content_start = close + RULE_LENGTH;
            assert!(content_start <= s.len());

            return (Some(&s[start..close]), &s[content_start..]);
        }
        // otherwise frontmatter never closed
    }
    // otherwise frontmatter never started
    (None, s)
}

#[cfg(test)]
mod test {
    use super::*;

    const EXAMPLE: &'_ str = "\
---
hello: world
---
# Title
";

    const EXAMPLE_EMPTY_YFM: &'_ str = "\
---
---
# Title
";

    const EXAMPLE_ONLY_YFM: &'_ str = "\
---
hello: world
---
";

    const EXAMPLE_NO_YFM: &'_ str = "";

    #[test]
    fn parses_example() {
        let (yfm, content) = parse_raw(EXAMPLE);
        assert_eq!(Some("hello: world\n"), yfm);
        assert_eq!("# Title\n", content);
    }

    #[test]
    fn parses_empty_yfm() {
        let (yfm, content) = parse_raw(EXAMPLE_EMPTY_YFM);
        assert_eq!(Some(""), yfm);
        assert_eq!("# Title\n", content);
    }

    #[test]
    fn parses_only_yfm() {
        let (yfm, content) = parse_raw(EXAMPLE_ONLY_YFM);
        assert_eq!(Some("hello: world\n"), yfm);
        assert_eq!("", content);
    }

    #[test]
    fn parses_no_yfm() {
        let (yfm, content) = parse_raw(EXAMPLE_NO_YFM);
        assert_eq!(None, yfm);
        assert_eq!("", content);
    }
}
