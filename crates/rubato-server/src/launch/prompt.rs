//! The prompt a launched session receives, and the branch slug.

use std::collections::BTreeMap;

/// The prompt the launched session receives. Shipped with the skill; a copy is embedded.
pub const PROMPT_TEMPLATE: &str = include_str!("../../../../skills/rubato/prompt.md");

/// Slug from a title: kebab-case, at most four words.
pub fn slugify(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| {
            !matches!(
                w.as_str(),
                "a" | "an" | "the" | "and" | "of" | "to" | "for" | "task"
            )
        })
        .take(4)
        .collect();
    if words.is_empty() {
        "work".into()
    } else {
        words.join("-")
    }
}

pub fn render_prompt(template: &str, vars: &BTreeMap<&str, String>) -> String {
    let mut out = template.to_owned();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slugify("Task 1: rubato-protocol — types, serde, JSON Schema export"),
            "1-rubato-protocol-types"
        );
        assert_eq!(slugify("Add the map view"), "add-map-view");
        assert_eq!(slugify(""), "work");
    }

    #[test]
    fn prompt_renders_placeholders() {
        let mut v = BTreeMap::new();
        v.insert("issue", "o/r#1".to_string());
        assert_eq!(
            render_prompt("on {issue} and {issue}", &v),
            "on o/r#1 and o/r#1"
        );
        assert!(PROMPT_TEMPLATE.contains("{plan_path}"));
    }
}
