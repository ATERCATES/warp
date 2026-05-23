use futures::executor::block_on;
use futures::io::Cursor;

use super::{CcStream, ControlEvent};

fn collect(input: &str) -> Vec<ControlEvent> {
    let mut stream = CcStream::new(Cursor::new(input.as_bytes().to_vec()));
    block_on(async {
        let mut events = Vec::new();
        while let Some(evt) = stream.next_event().await {
            events.push(evt);
        }
        events
    })
}

#[test]
fn begin_end_block_accumulates_payload_lines() {
    let input = "%begin 1700000000 1 0\n\
                 sess-1|main|1700000000|1|bash\n\
                 sess-2|work|1700000000|0|zsh\n\
                 %end 1700000000 1 0\n";
    let events = collect(input);
    assert_eq!(
        events,
        vec![
            ControlEvent::Begin { id: 1 },
            ControlEvent::End {
                id: 1,
                output: vec![
                    "sess-1|main|1700000000|1|bash".into(),
                    "sess-2|work|1700000000|0|zsh".into(),
                ],
            },
        ]
    );
}

#[test]
fn error_block_returns_payload_lines() {
    let input = "%begin 1700000000 7 0\n\
                 can't find session: foo\n\
                 %error 1700000000 7 0\n";
    let events = collect(input);
    assert_eq!(
        events,
        vec![
            ControlEvent::Begin { id: 7 },
            ControlEvent::Error {
                id: 7,
                output: vec!["can't find session: foo".into()],
            },
        ]
    );
}

#[test]
fn session_changed_splits_id_and_name() {
    let events = collect("%session-changed $0 main\n");
    assert_eq!(
        events,
        vec![ControlEvent::SessionChanged {
            session_id: "$0".into(),
            name: "main".into(),
        }]
    );
}

#[test]
fn session_changed_with_name_containing_spaces_keeps_trailing_segments() {
    let events = collect("%session-changed $0 my session with spaces\n");
    assert_eq!(
        events,
        vec![ControlEvent::SessionChanged {
            session_id: "$0".into(),
            name: "my session with spaces".into(),
        }]
    );
}

#[test]
fn client_session_changed_splits_three_segments() {
    let events = collect("%client-session-changed /dev/pts/0 $0 work\n");
    assert_eq!(
        events,
        vec![ControlEvent::ClientSessionChanged {
            tty: "/dev/pts/0".into(),
            session_id: "$0".into(),
            name: "work".into(),
        }]
    );
}

#[test]
fn exit_without_reason_is_none() {
    let events = collect("%exit\n");
    assert_eq!(events, vec![ControlEvent::Exit { reason: None }]);
}

#[test]
fn exit_with_reason_preserves_trailing_text() {
    let events = collect("%exit server exited\n");
    assert_eq!(
        events,
        vec![ControlEvent::Exit {
            reason: Some("server exited".into()),
        }]
    );
}

#[test]
fn unknown_percent_line_is_preserved_for_observation() {
    let events = collect("%pane-mode-changed 0 0\n");
    assert_eq!(
        events,
        vec![ControlEvent::Unknown("%pane-mode-changed 0 0".into())]
    );
}

#[test]
fn sessions_changed_event_is_dispatched() {
    let events = collect("%sessions-changed\n");
    assert_eq!(events, vec![ControlEvent::SessionsChanged]);
}

#[test]
fn config_error_carries_full_line_text() {
    let events = collect("%config-error unknown option foo\n");
    assert_eq!(
        events,
        vec![ControlEvent::ConfigError {
            line: "unknown option foo".into(),
        }]
    );
}
