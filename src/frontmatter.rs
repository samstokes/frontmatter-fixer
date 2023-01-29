use eyre::Context;
use std::io::Write;

const RULE_LENGTH: usize = "---\n".len();

pub fn parse(s: &str) -> (Option<serde_yaml::Result<serde_yaml::Value>>, &str) {
    let (raw_frontmatter, content) = parse_raw(s);
    let frontmatter = raw_frontmatter.map(serde_yaml::from_str);
    (frontmatter, content)
}

pub fn write<W: Write>(
    mut writer: W,
    frontmatter: Option<&serde_yaml::Value>,
    content: &str,
) -> eyre::Result<()> {
    if let Some(frontmatter) = frontmatter {
        writer.write_all(b"---\n")?;
        serde_yaml::to_writer(&mut writer, frontmatter)
            .context("couldn't serialize frontmatter")?;
        writer.write_all(b"---\n")?;
    }
    writer.write_all(content.as_bytes())?;
    Ok(())
}

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
    fn parses_example_raw() {
        let (yfm, content) = parse_raw(EXAMPLE);
        assert_eq!(Some("hello: world\n"), yfm);
        assert_eq!("# Title\n", content);
    }

    #[test]
    fn parses_empty_yfm_raw() {
        let (yfm, content) = parse_raw(EXAMPLE_EMPTY_YFM);
        assert_eq!(Some(""), yfm);
        assert_eq!("# Title\n", content);
    }

    #[test]
    fn parses_only_yfm_raw() {
        let (yfm, content) = parse_raw(EXAMPLE_ONLY_YFM);
        assert_eq!(Some("hello: world\n"), yfm);
        assert_eq!("", content);
    }

    #[test]
    fn parses_no_yfm_raw() {
        let (yfm, content) = parse_raw(EXAMPLE_NO_YFM);
        assert_eq!(None, yfm);
        assert_eq!("", content);
    }

    #[test]
    fn parses_example() {
        let (yfm, content) = parse(EXAMPLE);
        let yfm = yfm.expect("should be present").expect("should parse");

        let mut expected = serde_yaml::Mapping::new();
        expected.insert("hello".into(), "world".into());
        let expected = serde_yaml::Value::Mapping(expected);

        assert_eq!(expected, yfm);
        assert_eq!("# Title\n", content);
    }

    #[test]
    fn parses_empty_yfm() {
        let (yfm, content) = parse(EXAMPLE_EMPTY_YFM);
        let _ = yfm
            .expect("should be present")
            .expect_err("should not parse empty string");
        assert_eq!("# Title\n", content);
    }

    #[test]
    fn parses_only_yfm() {
        let (yfm, content) = parse(EXAMPLE_ONLY_YFM);
        let yfm = yfm.expect("should be present").expect("should parse");

        let mut expected = serde_yaml::Mapping::new();
        expected.insert("hello".into(), "world".into());
        let expected = serde_yaml::Value::Mapping(expected);

        assert_eq!(expected, yfm);
        assert_eq!("", content);
    }

    #[test]
    fn parses_no_yfm() {
        let (yfm, content) = parse(EXAMPLE_NO_YFM);
        assert!(yfm.is_none());
        assert_eq!("", content);
    }
}
