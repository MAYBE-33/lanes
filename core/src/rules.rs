//! Resolving an audio session to a channel.
//!
//! # Why executable name, and not something cleverer
//!
//! Process IDs change on every launch, so they cannot be the key. Window titles
//! change constantly. Session display names are set by the app and are often
//! absent or wrong. The executable name is the one thing that is stable across
//! restarts, survives reboots, and is recognisable to the person editing the
//! config by hand.
//!
//! # Matching order
//!
//! More specific rules win. A rule with a `path_contains` qualifier beats one
//! without, and an exact name beats a wildcard. Otherwise the first rule in the
//! file wins, so hand-editing behaves predictably: move a rule up to give it
//! priority.

use crate::config::{Config, Rule};

/// What a session resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution<'a> {
    /// Matched a rule; this is its channel.
    Channel { id: &'a str, trim: Option<f32> },
    /// Explicitly ignored — never managed, never shown.
    Ignored,
    /// No rule matched. Belongs in "To be routed".
    Unassigned,
}

/// Case-insensitive glob supporting `*`.
///
/// Written out rather than pulling in a glob crate: the patterns here are
/// executable names, the syntax is one metacharacter, and a dependency whose
/// behaviour differs subtly from what a user expects when hand-editing a config
/// is worse than twenty lines that do exactly what they say.
fn matches_glob(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let value = value.to_lowercase();

    if !pattern.contains('*') {
        return pattern == value;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    let mut position = 0usize;

    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        // A pattern not starting with '*' must match at the very beginning.
        if index == 0 {
            if !value.starts_with(part) {
                return false;
            }
            position = part.len();
            continue;
        }

        match value[position..].find(part) {
            Some(found) => position += found + part.len(),
            None => return false,
        }
    }

    // A pattern not ending in '*' must reach the end of the value.
    if let Some(last) = parts.last() {
        if !last.is_empty() && !value.ends_with(last) {
            return false;
        }
    }

    true
}

fn specificity(rule: &Rule) -> u8 {
    let mut score = 0;
    if rule.path_contains.is_some() {
        score += 2;
    }
    if !rule.pattern.contains('*') {
        score += 1;
    }
    score
}

/// The rule that decides where an application goes, if any does.
///
/// Separate from [`resolve`] so diagnostics can say *which* rule matched, not
/// only which channel won - "why is this app in Media" is usually answered by
/// the pattern, especially once wildcards are involved.
pub fn best_rule<'a>(
    config: &'a Config,
    executable: &str,
    full_path: &str,
) -> Option<&'a crate::config::Rule> {
    let path_lower = full_path.to_lowercase();

    config
        .rules
        .iter()
        .filter(|rule| matches_glob(&rule.pattern, executable))
        .filter(|rule| match &rule.path_contains {
            Some(fragment) => path_lower.contains(&fragment.to_lowercase()),
            None => true,
        })
        // max_by_key returns the LAST maximum; rules earlier in the file should
        // win ties, so the iterator is reversed first.
        .rev()
        .max_by_key(|rule| specificity(rule))
}

/// Whether an application is on the ignore list.
pub fn is_ignored(config: &Config, executable: &str) -> bool {
    config
        .ignored
        .iter()
        .any(|pattern| matches_glob(pattern, executable))
}

/// Resolve one session.
pub fn resolve<'a>(config: &'a Config, executable: &str, full_path: &str) -> Resolution<'a> {
    if is_ignored(config, executable) {
        return Resolution::Ignored;
    }

    match best_rule(config, executable, full_path) {
        Some(rule) => match config.channel(&rule.channel) {
            Some(channel) => Resolution::Channel {
                id: &channel.id,
                trim: rule.trim,
            },
            // A rule naming a channel that no longer exists is a config error,
            // not a reason to crash or to route audio somewhere arbitrary.
            // Treating it as unassigned surfaces it in "To be routed" where the
            // user can see something is wrong.
            None => {
                eprintln!(
                    "warning: rule '{}' points at unknown channel '{}'; treating as unassigned",
                    rule.pattern, rule.channel
                );
                Resolution::Unassigned
            }
        },
        None => Resolution::Unassigned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rule;

    fn config_with(rules: Vec<Rule>) -> Config {
        Config {
            rules,
            ..Config::default()
        }
    }

    fn rule(pattern: &str, channel: &str) -> Rule {
        Rule {
            pattern: pattern.into(),
            channel: channel.into(),
            path_contains: None,
            trim: None,
        }
    }

    #[test]
    fn nothing_matches_by_default() {
        let c = Config::default();
        assert_eq!(
            resolve(&c, "Spotify.exe", r"C:\spotify\Spotify.exe"),
            Resolution::Unassigned
        );
    }

    #[test]
    fn exact_name_matches_case_insensitively() {
        let c = config_with(vec![rule("spotify.exe", "media")]);
        assert!(matches!(
            resolve(&c, "Spotify.exe", r"C:\x\Spotify.exe"),
            Resolution::Channel { id: "media", .. }
        ));
    }

    #[test]
    fn wildcards_work() {
        let c = config_with(vec![rule("chrome*.exe", "media")]);
        assert!(matches!(
            resolve(&c, "chrome_proxy.exe", r"C:\x\chrome_proxy.exe"),
            Resolution::Channel { id: "media", .. }
        ));
        assert_eq!(
            resolve(&c, "notchrome.exe", r"C:\x\notchrome.exe"),
            Resolution::Unassigned
        );
    }

    #[test]
    fn leading_wildcard_matches_a_suffix() {
        let c = config_with(vec![rule("*helper.exe", "aux")]);
        assert!(matches!(
            resolve(&c, "steamwebhelper.exe", r"C:\x\steamwebhelper.exe"),
            Resolution::Channel { id: "aux", .. }
        ));
    }

    #[test]
    fn path_qualifier_disambiguates_same_named_executables() {
        let mut work = rule("game.exe", "game");
        work.path_contains = Some(r"\SteamLibrary\".into());

        let c = config_with(vec![rule("game.exe", "aux"), work]);

        // The qualified rule is more specific, so it wins despite being second.
        assert!(matches!(
            resolve(&c, "game.exe", r"D:\SteamLibrary\game.exe"),
            Resolution::Channel { id: "game", .. }
        ));
        // A different install falls through to the general rule.
        assert!(matches!(
            resolve(&c, "game.exe", r"C:\Elsewhere\game.exe"),
            Resolution::Channel { id: "aux", .. }
        ));
    }

    #[test]
    fn exact_beats_wildcard() {
        let c = config_with(vec![rule("*.exe", "aux"), rule("discord.exe", "chat")]);
        assert!(matches!(
            resolve(&c, "Discord.exe", r"C:\x\Discord.exe"),
            Resolution::Channel { id: "chat", .. }
        ));
    }

    #[test]
    fn earlier_rule_wins_a_tie() {
        let c = config_with(vec![
            rule("discord.exe", "chat"),
            rule("discord.exe", "aux"),
        ]);
        assert!(matches!(
            resolve(&c, "Discord.exe", r"C:\x\Discord.exe"),
            Resolution::Channel { id: "chat", .. }
        ));
    }

    #[test]
    fn ignored_apps_are_never_managed() {
        let c = Config {
            ignored: vec!["launcher.exe".into()],
            rules: vec![rule("launcher.exe", "aux")],
            ..Config::default()
        };
        assert_eq!(
            resolve(&c, "launcher.exe", r"C:\x\launcher.exe"),
            Resolution::Ignored
        );
    }

    #[test]
    fn rule_pointing_at_a_missing_channel_is_unassigned_not_a_crash() {
        let c = config_with(vec![rule("spotify.exe", "nonexistent")]);
        assert_eq!(
            resolve(&c, "Spotify.exe", r"C:\x\Spotify.exe"),
            Resolution::Unassigned
        );
    }
}
