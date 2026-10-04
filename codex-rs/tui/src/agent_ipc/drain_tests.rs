use super::*;
use crate::agent_ipc::DeliveryClass;
use crate::agent_ipc::storage::append_json_line;
use crate::agent_ipc::storage::read_jsonl;
use chrono::Utc;
use pretty_assertions::assert_eq;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;
use tempfile::tempdir;

struct Harness {
    _temp: TempDir,
    chat_path: PathBuf,
    receipt_path: PathBuf,
    identity: AgentIdentity,
    drain: InboundMessageDrain,
}

impl Harness {
    fn new() -> Self {
        let temp = tempdir().unwrap();
        Self {
            chat_path: temp.path().join("agent.chat.jsonl"),
            receipt_path: temp.path().join("receipts.jsonl"),
            _temp: temp,
            identity: identity(),
            drain: InboundMessageDrain::default(),
        }
    }

    fn append(&self, message: &AgentMessage) {
        append_json_line(&self.chat_path, message).unwrap();
    }

    fn run(&mut self) -> io::Result<(DrainReport, Vec<AgentMessage>)> {
        let mut delivered = Vec::new();
        let report = self.drain.drain(
            &self.chat_path,
            &self.receipt_path,
            &self.identity,
            |_| Ok(()),
            |message| delivered.push(message),
        )?;
        Ok((report, delivered))
    }

    fn receipts(&self) -> Vec<String> {
        read_jsonl(&self.receipt_path).unwrap()
    }
}

fn identity() -> AgentIdentity {
    AgentIdentity {
        instance_id: "recipient-instance".to_string(),
        agent_name: "PROJECT:RECIPIENT".to_string(),
        agent_symbol: "R".to_string(),
        agent_aliases: vec!["RECIPIENT".to_string()],
        mailbox_path: None,
        project_path: "/project".to_string(),
        git_branch: "main".to_string(),
        pid: 1,
    }
}

fn message(id: &str, to: &str, delivery_class: DeliveryClass) -> AgentMessage {
    AgentMessage {
        id: id.to_string(),
        from_instance_id: format!("sender-{id}"),
        from_agent_name: "SENDER".to_string(),
        from_symbol: "S".to_string(),
        to: to.to_string(),
        subject: format!("subject-{id}"),
        body: format!("body-{id}"),
        created_at: Utc::now(),
        delivery_class,
    }
}

#[test]
fn incremental_drain_delivers_each_class_once_in_log_order() {
    let mut harness = Harness::new();
    let expected = vec![
        message("one", "RECIPIENT", DeliveryClass::Ephemeral),
        message("two", "RECIPIENT", DeliveryClass::Durable),
    ];
    for message in &expected {
        harness.append(message);
    }

    let (first_report, first) = harness.run().unwrap();
    let (second_report, second) = harness.run().unwrap();

    assert_eq!(first, expected);
    assert_eq!(first_report.delivered, 2);
    assert!(first_report.scanned_bytes > 0);
    assert_eq!(second, Vec::new());
    assert_eq!(second_report, DrainReport::default());
    assert_eq!(
        harness.receipts(),
        vec!["one".to_string(), "two".to_string()]
    );
}

#[test]
fn target_self_and_receipt_filters_share_the_incremental_checkpoint() {
    let mut harness = Harness::new();
    let known = message("known", "RECIPIENT", DeliveryClass::Durable);
    let mut own = message("own", "RECIPIENT", DeliveryClass::Ephemeral);
    own.from_instance_id = harness.identity.instance_id.clone();
    harness.append(&message("other", "OTHER", DeliveryClass::Ephemeral));
    harness.append(&own);
    harness.append(&known);
    append_json_line(&harness.receipt_path, &known.id).unwrap();

    let (report, delivered) = harness.run().unwrap();

    assert_eq!(report.delivered, 0);
    assert_eq!(delivered, Vec::new());
    assert_eq!(harness.run().unwrap(), (DrainReport::default(), Vec::new()));
}

#[test]
fn receipt_is_persisted_before_delivery() {
    let mut harness = Harness::new();
    harness.append(&message("ordered", "RECIPIENT", DeliveryClass::Durable));
    let receipt_path = harness.receipt_path.clone();
    let mut delivered = Vec::new();

    harness
        .drain
        .drain(
            &harness.chat_path,
            &harness.receipt_path,
            &harness.identity,
            |_| Ok(()),
            |message| {
                assert_eq!(
                    read_jsonl::<String>(&receipt_path).unwrap(),
                    vec!["ordered".to_string()]
                );
                delivered.push(message);
            },
        )
        .unwrap();

    assert_eq!(delivered.len(), 1);
}

#[test]
fn callback_failures_retry_without_early_delivery_or_checkpoint_advance() {
    let mut harness = Harness::new();
    let expected = message("retry", "RECIPIENT", DeliveryClass::Ephemeral);
    harness.append(&expected);

    let err = harness
        .drain
        .drain(
            &harness.chat_path,
            &harness.receipt_path,
            &harness.identity,
            |_| Err(io::Error::other("mailbox append failed")),
            |_| panic!("failed preparation must not deliver"),
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "mailbox append failed");
    assert_eq!(harness.run().unwrap().1, vec![expected]);
}

#[test]
fn receipt_write_failure_retries_without_delivery() {
    let mut harness = Harness::new();
    let expected = message("retry-receipt", "RECIPIENT", DeliveryClass::Durable);
    harness.append(&expected);
    fs::create_dir(&harness.receipt_path).unwrap();

    assert!(harness.run().is_err());
    fs::remove_dir(&harness.receipt_path).unwrap();

    assert_eq!(harness.run().unwrap().1, vec![expected]);
}

#[test]
fn append_during_snapshot_is_seen_on_the_next_check() {
    let mut harness = Harness::new();
    harness.append(&message("first", "RECIPIENT", DeliveryClass::Ephemeral));
    let chat_path = harness.chat_path.clone();
    let mut delivered = Vec::new();

    harness
        .drain
        .drain(
            &harness.chat_path,
            &harness.receipt_path,
            &harness.identity,
            |_| {
                append_json_line(
                    &chat_path,
                    &message("second", "RECIPIENT", DeliveryClass::Ephemeral),
                )
            },
            |message| delivered.push(message),
        )
        .unwrap();
    delivered.extend(harness.run().unwrap().1);

    assert_eq!(
        delivered
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
}

#[test]
fn partial_record_is_retried_after_completion() {
    let mut harness = Harness::new();
    let expected = message("partial", "RECIPIENT", DeliveryClass::Ephemeral);
    let serialized = serde_json::to_vec(&expected).unwrap();
    let split = serialized.len() / 2;
    fs::write(&harness.chat_path, &serialized[..split]).unwrap();

    assert_eq!(harness.run().unwrap().1, Vec::new());
    let mut file = OpenOptions::new()
        .append(true)
        .open(&harness.chat_path)
        .unwrap();
    file.write_all(&serialized[split..]).unwrap();
    file.write_all(b"\n").unwrap();

    assert_eq!(harness.run().unwrap().1, vec![expected]);
}

#[test]
fn malformed_complete_record_blocks_until_repaired() {
    let mut harness = Harness::new();
    fs::write(&harness.chat_path, b"{}\n").unwrap();

    assert_eq!(
        harness.run().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let expected = message("repaired", "RECIPIENT", DeliveryClass::Durable);
    fs::write(&harness.chat_path, b"").unwrap();
    harness.append(&expected);

    assert_eq!(harness.run().unwrap().1, vec![expected]);
}

#[test]
fn replaced_and_truncated_logs_recover_without_receipt_replay() {
    let mut harness = Harness::new();
    let first = message("first-long-id", "RECIPIENT", DeliveryClass::Durable);
    let second = message("two", "RECIPIENT", DeliveryClass::Durable);
    harness.append(&first);
    assert_eq!(harness.run().unwrap().1, vec![first.clone()]);

    let replacement = harness.chat_path.with_extension("replacement");
    append_json_line(&replacement, &first).unwrap();
    append_json_line(&replacement, &second).unwrap();
    fs::remove_file(&harness.chat_path).unwrap();
    fs::rename(&replacement, &harness.chat_path).unwrap();
    assert_eq!(harness.run().unwrap().1, vec![second]);

    let third = message("3", "RECIPIENT", DeliveryClass::Ephemeral);
    fs::write(&harness.chat_path, b"").unwrap();
    harness.append(&third);
    assert_eq!(harness.run().unwrap().1, vec![third]);
}

#[test]
fn separate_launch_paths_have_identical_drain_semantics() {
    let mut direct = Harness::new();
    let expected = message("shared", "RECIPIENT", DeliveryClass::Ephemeral);
    direct.append(&expected);
    let mut hosted = Harness::new();
    hosted.append(&expected);

    assert_eq!(direct.run().unwrap().1, hosted.run().unwrap().1);
}

#[test]
fn idle_marker_detection_and_suppression_are_terminal_only() {
    assert!(requests_idle_transition("awaiting instructions. Ω\n"));
    assert!(!requests_idle_transition("Ω is only mentioned here."));
    let mut message = "Finished.  Ω\n".to_string();

    suppress_idle_transition(&mut message);

    insta::assert_snapshot!(message, @"Finished.");
}
