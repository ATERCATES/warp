use super::{classify_ssh_error, extract_capabilities, BEGIN_MARKER, END_MARKER};
use crate::terminal::remote_sessions::types::HostError;

fn wrap_payload(json: &str) -> String {
    format!("preamble noise\n{BEGIN_MARKER}\n{json}\n{END_MARKER}\nsuffix noise\n")
}

#[test]
fn extract_capabilities_parses_well_formed_payload() {
    let payload = wrap_payload(
        r#"{
            "tmux_bin": "/usr/bin/tmux",
            "tmux_version": "3.3a",
            "tmux_supported": true,
            "passthrough": true,
            "shell_integration": true,
            "os": "linux",
            "pkg": "apt",
            "root_access": "can_run_sudo"
        }"#,
    );
    let caps = extract_capabilities(&payload).expect("well-formed payload");
    assert_eq!(caps.tmux_version, "3.3a");
    assert!(caps.tmux_supported);
    assert_eq!(caps.os, "linux");
}

#[test]
fn extract_capabilities_missing_begin_marker_returns_probe_malformed() {
    let err = extract_capabilities("no markers here").unwrap_err();
    assert!(matches!(err, HostError::ProbeMalformed(_)));
}

#[test]
fn extract_capabilities_missing_end_marker_returns_probe_malformed() {
    let err = extract_capabilities(&format!("{BEGIN_MARKER}\n{{}}\n")).unwrap_err();
    assert!(matches!(err, HostError::ProbeMalformed(_)));
}

#[test]
fn extract_capabilities_invalid_json_returns_probe_malformed() {
    let payload = wrap_payload("not json at all");
    let err = extract_capabilities(&payload).unwrap_err();
    match err {
        HostError::ProbeMalformed(detail) => assert!(detail.contains("not json at all")),
        other => panic!("expected ProbeMalformed, got {other:?}"),
    }
}

#[test]
fn classify_ssh_error_recognizes_permission_denied() {
    let err = classify_ssh_error("foo@bar: Permission denied (publickey).");
    assert!(matches!(err, HostError::SshAuthFailed(_)));
}

#[test]
fn classify_ssh_error_recognizes_generic_authentication_failures() {
    let err = classify_ssh_error("Authentication failed.");
    assert!(matches!(err, HostError::SshAuthFailed(_)));
}

#[test]
fn classify_ssh_error_recognizes_unreachable_phrases() {
    for stderr in [
        "ssh: Could not resolve hostname foo: Name or service not known",
        "ssh: connect to host foo port 22: No route to host",
        "ssh: connect to host foo port 22: Connection refused",
        "ssh: connect to host foo port 22: Connection timed out",
        "ssh: connect to host foo port 22: Operation timed out",
        "ssh: connect to host foo port 22: Network is unreachable",
    ] {
        let err = classify_ssh_error(stderr);
        assert!(
            matches!(err, HostError::HostUnreachable(_)),
            "stderr={stderr:?} classified as {err:?}",
        );
    }
}

#[test]
fn classify_ssh_error_empty_input_falls_back_to_generic() {
    let err = classify_ssh_error("");
    match err {
        HostError::Other(msg) => assert_eq!(msg, "ssh failed"),
        other => panic!("expected Other(\"ssh failed\"), got {other:?}"),
    }
}

#[test]
fn classify_ssh_error_unknown_text_preserves_first_nonempty_line() {
    let err = classify_ssh_error("\n\nweird tls handshake error\nfollowup noise\n");
    match err {
        HostError::Other(msg) => assert_eq!(msg, "weird tls handshake error"),
        other => panic!("expected Other(_), got {other:?}"),
    }
}
