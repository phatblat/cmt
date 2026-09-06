/// Leading directories that name a layout, not a component; they are dropped
/// before the first path segment is read as the scope.
const ROOTS: [&str; 9] = [
    "src", "lib", "app", "pkg", "crates", "packages", "docs", "tests", "test",
];

/// The component every path lives under, or `None` when they disagree or a
/// path sits directly in a root (`src/main.rs` has no scope).
pub fn scope(paths: &[&str]) -> Option<String> {
    let mut common: Option<&str> = None;
    for path in paths {
        let mut parts = path.split('/');
        let mut first = parts.next()?;
        if ROOTS.contains(&first) {
            first = parts.next()?;
        }
        // A scope is a directory: there has to be something below it.
        parts.next()?;
        match common {
            None => common = Some(first),
            Some(seen) if seen == first => {}
            Some(_) => return None,
        }
    }
    common.map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_component_under_a_root() {
        assert_eq!(
            scope(&["src/parser/lex.rs", "src/parser/ast.rs"]),
            Some("parser".into())
        );
    }

    #[test]
    fn workspace_crate_is_the_scope() {
        assert_eq!(scope(&["crates/core/src/x.rs"]), Some("core".into()));
    }

    #[test]
    fn file_directly_in_root_has_no_scope() {
        assert_eq!(scope(&["src/main.rs"]), None);
    }

    #[test]
    fn disagreeing_components_have_no_scope() {
        assert_eq!(scope(&["src/a/x.rs", "src/b/y.rs"]), None);
    }

    #[test]
    fn docs_subdirectory_is_a_scope() {
        assert_eq!(scope(&["docs/guide/a.md"]), Some("guide".into()));
    }

    #[test]
    fn empty_input_has_no_scope() {
        assert_eq!(scope(&[]), None);
    }
}
