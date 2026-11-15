#![allow(clippy::unwrap_used, clippy::expect_used)]

use secrecy::SecretString;

use super::*;

fn settings() -> ConnectionSettings {
    ConnectionSettings {
        name: "local".into(),
        kind: DatabaseKind::Postgres,
        host: "127.0.0.1".into(),
        port: 5432,
        user: "dbased".into(),
        database: "dbased".into(),
        tls: TlsMode::Prefer,
    }
}

#[test]
fn debug_of_commands_hides_typed_passwords() {
    let secret = "hunter2-very-secret";
    let commands = [
        Command::SaveConnection {
            request: RequestId(1),
            id: None,
            settings: settings(),
            password: PasswordUpdate::Save(SecretString::from(secret)),
        },
        Command::TestConnection {
            request: RequestId(2),
            id: None,
            settings: settings(),
            password: PasswordSource::Typed(SecretString::from(secret)),
        },
        Command::Connect {
            request: RequestId(3),
            id: ConnectionId(1),
            password: PasswordSource::Typed(SecretString::from(secret)),
        },
    ];
    for command in commands {
        let printed = format!("{command:?}");
        assert!(!printed.contains(secret), "leaked in: {printed}");
    }
}

#[test]
fn bridge_round_trips_commands_and_events() {
    let (ui_link, core_link) = bridge(COMMAND_CAPACITY, EVENT_CAPACITY);

    let tab = TabId(7);
    ui_link
        .commands
        .send_blocking(Command::CloseTab { tab })
        .unwrap();
    let received = core_link.commands.recv_blocking().unwrap();
    assert!(matches!(received, Command::CloseTab { tab: received_tab } if received_tab == tab));

    let event = Event::TabClosed { tab };
    core_link.events.send_blocking(event.clone()).unwrap();
    assert_eq!(ui_link.events.recv_blocking().unwrap(), event);
}

#[test]
fn closing_every_sender_ends_the_receiver() {
    let (ui_link, core_link) = bridge(4, 4);
    let UiLink { commands, events } = ui_link;
    let CoreLink {
        commands: command_receiver,
        events: event_sender,
    } = core_link;

    commands
        .send_blocking(Command::CloseTab { tab: TabId(1) })
        .unwrap();
    drop(commands);
    assert!(command_receiver.recv_blocking().is_ok());
    assert!(command_receiver.recv_blocking().is_err());

    event_sender
        .send_blocking(Event::TabClosed { tab: TabId(1) })
        .unwrap();
    drop(event_sender);
    assert!(events.recv_blocking().is_ok());
    assert!(events.recv_blocking().is_err());
}

#[test]
fn result_id_is_derived_from_request_and_statement() {
    let first = ResultId {
        request: RequestId(5),
        statement: 0,
    };
    let second = ResultId {
        request: RequestId(5),
        statement: 1,
    };
    assert_ne!(first, second);
    assert_eq!(first.request, second.request);
}
