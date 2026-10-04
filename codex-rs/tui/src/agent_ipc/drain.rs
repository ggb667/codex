use serde_json::Error as JsonError;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::fs::Metadata;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;
#[cfg(not(unix))]
use std::time::SystemTime;

use super::AgentIdentity;
use super::AgentMessage;
use super::storage::append_json_line;
use super::storage::target_matches;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created_at: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DrainCheckpoint {
    identity: LogIdentity,
    offset: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DrainReport {
    pub(crate) delivered: usize,
    pub(crate) scanned_bytes: u64,
}

#[derive(Debug, Default)]
pub(crate) struct InboundMessageDrain {
    checkpoint: Option<DrainCheckpoint>,
}

impl InboundMessageDrain {
    pub(crate) fn drain<P, D>(
        &mut self,
        chat_path: &Path,
        receipt_path: &Path,
        identity: &AgentIdentity,
        mut prepare: P,
        mut deliver: D,
    ) -> io::Result<DrainReport>
    where
        P: FnMut(&AgentMessage) -> io::Result<()>,
        D: FnMut(AgentMessage),
    {
        let metadata = match fs::metadata(chat_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                self.checkpoint = None;
                return Ok(DrainReport::default());
            }
            Err(err) => return Err(err),
        };
        let identity_key = log_identity(&metadata);
        let snapshot_len = metadata.len();
        let start_offset = self
            .checkpoint
            .as_ref()
            .filter(|checkpoint| {
                checkpoint.identity == identity_key && checkpoint.offset <= snapshot_len
            })
            .map_or(/*default*/ 0, |checkpoint| checkpoint.offset);
        if start_offset == snapshot_len {
            return Ok(DrainReport::default());
        }

        let mut receipts = read_receipts(receipt_path)?;
        let mut file = File::open(chat_path)?;
        file.seek(SeekFrom::Start(start_offset))?;
        let bounded = file.take(snapshot_len.saturating_sub(start_offset));
        let mut reader = BufReader::new(bounded);
        let mut offset = start_offset;
        let mut report = DrainReport::default();
        self.checkpoint = Some(DrainCheckpoint {
            identity: identity_key.clone(),
            offset,
        });

        loop {
            let mut line = Vec::new();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            report.scanned_bytes += read as u64;
            if !line.ends_with(b"\n") {
                break;
            }
            let next_offset = offset + read as u64;
            let trimmed = trim_ascii_whitespace(&line);
            if trimmed.is_empty() {
                offset = next_offset;
                self.advance(identity_key.clone(), offset);
                continue;
            }
            let message = serde_json::from_slice::<AgentMessage>(trimmed)
                .map_err(|err| invalid_record(offset, err))?;
            if message.from_instance_id != identity.instance_id
                && target_matches(&message.to, identity)
                && !receipts.contains(&message.id)
            {
                prepare(&message)?;
                append_json_line(receipt_path, &message.id)?;
                receipts.insert(message.id.clone());
                deliver(message);
                report.delivered += 1;
            }
            offset = next_offset;
            self.advance(identity_key.clone(), offset);
        }

        Ok(report)
    }

    fn advance(&mut self, identity: LogIdentity, offset: u64) {
        self.checkpoint = Some(DrainCheckpoint { identity, offset });
    }
}

pub(crate) fn requests_idle_transition(message: &str) -> bool {
    message.trim_end().ends_with('Ω')
}

pub(crate) fn suppress_idle_transition(message: &mut String) {
    let trimmed = message.trim_end();
    let Some(without_marker) = trimmed.strip_suffix('Ω') else {
        return;
    };
    message.truncate(without_marker.trim_end().len());
}

fn invalid_record(offset: u64, err: JsonError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid agent IPC record at byte {offset}: {err}"),
    )
}

fn read_receipts(path: &Path) -> io::Result<HashSet<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(err) => return Err(err),
    };
    let mut receipts = HashSet::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let receipt = serde_json::from_str::<String>(trimmed).map_err(io::Error::other)?;
        receipts.insert(receipt);
    }
    Ok(receipts)
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[cfg(unix)]
fn log_identity(metadata: &Metadata) -> LogIdentity {
    use std::os::unix::fs::MetadataExt;

    LogIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn log_identity(metadata: &Metadata) -> LogIdentity {
    LogIdentity {
        created_at: metadata.created().ok(),
    }
}

#[cfg(test)]
#[path = "drain_tests.rs"]
mod tests;
