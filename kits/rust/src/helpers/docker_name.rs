//! Names Docker accepts for containers, volumes and network aliases.

/// A container or volume name: `[A-Za-z0-9][A-Za-z0-9_.-]*`, at most 63 characters.
pub fn valid_docker_name(s: &str) -> bool {
    s.len() <= 63
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
