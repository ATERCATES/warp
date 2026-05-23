use super::{
    kill_session_cmd, list_sessions_cmd, new_session_cmd, parse_sessions, shell_escape,
    CONTROL_SESSION_NAME, SESSIONS_FORMAT,
};

fn line(parts: &[&str]) -> String {
    parts.join("|")
}

#[test]
fn list_sessions_cmd_uses_canonical_format() {
    assert_eq!(
        list_sessions_cmd(),
        format!("list-sessions -F '{SESSIONS_FORMAT}'")
    );
}

#[test]
fn new_session_cmd_uses_unquoted_simple_name() {
    assert_eq!(new_session_cmd("work", None), "new-session -d -s work");
}

#[test]
fn new_session_cmd_quotes_name_with_spaces() {
    assert_eq!(
        new_session_cmd("my work", None),
        "new-session -d -s 'my work'"
    );
}

#[test]
fn new_session_cmd_includes_quoted_command_when_provided() {
    assert_eq!(
        new_session_cmd("work", Some("htop --tree")),
        "new-session -d -s work 'htop --tree'"
    );
}

#[test]
fn new_session_cmd_skips_empty_command() {
    assert_eq!(
        new_session_cmd("work", Some("")),
        "new-session -d -s work"
    );
}

#[test]
fn kill_session_cmd_quotes_name_with_special_chars() {
    assert_eq!(kill_session_cmd("a b"), "kill-session -t 'a b'");
}

#[test]
fn shell_escape_passes_alphanumeric_through_unchanged() {
    assert_eq!(shell_escape("simple_name-1.2"), "simple_name-1.2");
}

#[test]
fn shell_escape_quotes_value_with_spaces() {
    assert_eq!(shell_escape("foo bar"), "'foo bar'");
}

#[test]
fn shell_escape_breaks_out_single_quote_with_posix_idiom() {
    // POSIX sh: 'X' '\'' 'Y' concatenates to literal "X'Y".
    assert_eq!(shell_escape("O'Brien"), "'O'\\''Brien'");
}

#[test]
fn shell_escape_empty_string_is_quoted_empty() {
    assert_eq!(shell_escape(""), "''");
}

#[test]
fn parse_sessions_skips_control_session_by_name() {
    let lines = vec![
        line(&["$0", CONTROL_SESSION_NAME, "1700000000", "1", "bash"]),
        line(&["$1", "work", "1700000100", "0", "zsh"]),
    ];
    let parsed = parse_sessions(&lines, None);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].session_id, "$1");
    assert_eq!(parsed[0].name, "work");
    assert_eq!(parsed[0].current_command, "zsh");
}

#[test]
fn parse_sessions_filters_excluded_session_id() {
    let lines = vec![
        line(&["$0", "main", "1700000000", "1", "bash"]),
        line(&["$1", "other", "1700000100", "0", "zsh"]),
    ];
    let parsed = parse_sessions(&lines, Some("$0"));
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].session_id, "$1");
}

#[test]
fn parse_sessions_drops_malformed_rows_without_panicking() {
    let lines = vec![
        "only-one-field".into(),
        line(&["$1", "name", "not-a-number", "1", "bash"]),
        line(&["$2", "good", "1700000000", "1", "bash"]),
    ];
    let parsed = parse_sessions(&lines, None);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].session_id, "$2");
}

#[test]
fn parse_sessions_treats_missing_current_command_as_empty() {
    let lines = vec![line(&["$0", "main", "1700000000", "1"])];
    let parsed = parse_sessions(&lines, None);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].current_command, "");
}
