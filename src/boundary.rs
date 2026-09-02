use std::path::{Component, Path};

use anyhow::{Result, bail};

pub fn has_wildcards(pattern: &str) -> bool {
    pattern.bytes().any(|byte| matches!(byte, b'*' | b'?'))
}

pub fn validate_pattern(pattern: &str) -> Result<()> {
    if pattern == "*" {
        return Ok(());
    }
    let path = Path::new(pattern);
    if !path.is_absolute() || pattern.contains('\0') || pattern.contains('\\') {
        bail!("boundary patterns must be '*' or absolute paths without NUL or backslashes");
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!("boundary patterns cannot contain '.' or '..' components");
    }
    Ok(())
}

pub fn validate_program_pattern(pattern: &str) -> Result<()> {
    if pattern.contains('/') {
        return validate_pattern(pattern);
    }
    if pattern.is_empty()
        || pattern.contains('\0')
        || pattern.contains('\\')
        || matches!(pattern, "." | "..")
    {
        bail!(
            "program patterns must be basenames, '*', or absolute paths without NUL or backslashes"
        );
    }
    Ok(())
}

pub fn program_name_matches(pattern: &str, name: &str) -> bool {
    !pattern.contains('/') && wildcard_match(pattern.as_bytes(), name.as_bytes())
}

pub fn path_allowed(
    path: &Path,
    includes: &[String],
    excludes: &[String],
    directory: bool,
) -> bool {
    let Some(value) = path.to_str() else {
        return false;
    };
    let included = best_match(value, includes, directory);
    let excluded = best_match(value, excludes, directory);
    match (included, excluded) {
        (Some(include), Some(exclude)) => include >= exclude,
        (Some(_), None) => true,
        _ => false,
    }
}

pub fn pattern_may_match_below(pattern: &str, directory: &Path) -> bool {
    if pattern == "*" {
        return false;
    }
    let prefix = pattern
        .find(['*', '?'])
        .map_or(pattern, |index| &pattern[..index]);
    let prefix = Path::new(prefix.trim_end_matches('/'));
    prefix.starts_with(directory) || directory.starts_with(prefix)
}

fn best_match(value: &str, patterns: &[String], directory: bool) -> Option<usize> {
    patterns
        .iter()
        .filter(|pattern| pattern_matches(pattern, value, directory))
        .map(|pattern| {
            pattern
                .bytes()
                .filter(|byte| !matches!(byte, b'*' | b'?'))
                .count()
        })
        .max()
}

fn pattern_matches(pattern: &str, value: &str, directory: bool) -> bool {
    if pattern == "*" {
        return true;
    }
    if !directory {
        return wildcard_match(pattern.as_bytes(), value.as_bytes())
            || Path::new(value).file_name().is_some_and(|name| {
                name.to_str()
                    .is_some_and(|name| program_name_matches(pattern, name))
            });
    }
    let path = Path::new(value);
    path.ancestors().any(|ancestor| {
        ancestor
            .to_str()
            .is_some_and(|candidate| wildcard_match(pattern.as_bytes(), candidate.as_bytes()))
    })
}

fn wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    let (mut p, mut v, mut star, mut retry) = (0usize, 0usize, None, 0usize);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = v;
        } else if let Some(index) = star {
            p = index + 1;
            retry += 1;
            v = retry;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specific_rules_win_and_includes_win_ties() {
        assert!(path_allowed(
            Path::new("/home/moe/code/app"),
            &["/home/*/code".into()],
            &["*".into()],
            true
        ));
        assert!(!path_allowed(
            Path::new("/home/moe/code/private/key"),
            &["/home/moe/code".into()],
            &["/home/moe/code/private".into()],
            true
        ));
        assert!(path_allowed(
            Path::new("/home/moe/code/private/key"),
            &["/home/moe/code/private/*".into()],
            &["/home/moe/code/private/*".into()],
            true
        ));
    }

    #[test]
    fn wildcard_program_rules_match_paths_and_basenames() {
        assert!(path_allowed(
            Path::new("/usr/bin/git"),
            &["/usr/bin/git*".into()],
            &["*".into()],
            false
        ));
        assert!(!path_allowed(
            Path::new("/usr/bin/rm"),
            &["*".into()],
            &["rm".into()],
            false
        ));
        assert!(path_allowed(
            Path::new("/usr/bin/rm"),
            &["rm".into()],
            &["*".into()],
            false
        ));
        assert!(!path_allowed(
            Path::new("/usr/bin/bash"),
            &["/usr/bin/git*".into()],
            &["*".into()],
            false
        ));
    }
}
