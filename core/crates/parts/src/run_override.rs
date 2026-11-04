//! A package's run command replaced from the environment. `ARENA_RUN_<PACKAGE>` (the package
//! name upper-cased, `-` as `_`) holds a command line that `/bin/sh -c` runs in place of the
//! manifest's `run`. It is for a part that runs on another machine, for example
//! `ssh <host> -- <command>`, whose stdin and stdout carry the protocol as they would locally.

/// The variable that overrides `package`'s run command: `isaac-sim` is `ARENA_RUN_ISAAC_SIM`.
pub fn var_name(package: &str) -> String {
    format!("ARENA_RUN_{}", package.to_uppercase().replace('-', "_"))
}

/// The override set for `package` in arenad's environment, if it is set and not blank.
pub fn from_env(package: &str) -> Option<String> {
    std::env::var(var_name(package))
        .ok()
        .filter(|line| !line.trim().is_empty())
}

/// The program and arguments that run `line`.
pub fn argv(line: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), line.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_is_named_after_the_package() {
        assert_eq!(var_name("isaac-sim"), "ARENA_RUN_ISAAC_SIM");
        assert_eq!(var_name("factory"), "ARENA_RUN_FACTORY");
    }

    #[test]
    fn a_blank_override_is_no_override() {
        assert_eq!(from_env("no-such-package-for-run-override-test"), None);
    }

    #[test]
    fn the_shell_runs_the_line() {
        assert_eq!(argv("ssh h -- cmd"), ["/bin/sh", "-c", "ssh h -- cmd"]);
    }
}
