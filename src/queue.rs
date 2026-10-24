//! Tickets: `queue/<micros-UTC-16-digits>-<4 hex>.json`, created with `create_new`. The file name
//! orders the queue; the body is the same as `holder.json`; the mtime is the waiter's heartbeat.

use crate::fsx;
use crate::holder::{now_micros, Holder};
use crate::store::{read_body, KeyDir};
use crate::wait::random_u64;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Ticket {
    /// File name without `.json`; sorts in arrival order.
    pub name: String,
    pub path: PathBuf,
    pub body: Holder,
}

pub fn new_name() -> String {
    format!("{:016}-{:04x}", now_micros(), random_u64() & 0xffff)
}

/// Creates a ticket for `body` in the key's queue. If the folder disappears meanwhile (a release
/// removed it), it is created again and the ticket retried.
pub fn create(kd: &KeyDir, body: &Holder) -> io::Result<Ticket> {
    let bytes = body.to_bytes();
    loop {
        fsx::retry(|| fs::create_dir_all(kd.queue_dir()))?;
        kd.ensure_key_file()?;
        let name = new_name();
        let path = kd.queue_dir().join(format!("{name}.json"));
        match fsx::create_exclusive(&path, &bytes) {
            Ok(()) => {
                kd.ensure_key_file()?;
                return Ok(Ticket {
                    name,
                    path,
                    body: body.clone(),
                });
            }
            Err(e) if fsx::is_exists(&e) || fsx::is_not_found(&e) => continue,
            Err(e) => return Err(e),
        }
    }
}

/// The ticket names in the queue, in order (bodies not read).
pub fn names(kd: &KeyDir) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(kd.queue_dir()) {
        Ok(entries) => entries,
        Err(e) if fsx::is_not_found(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(str::to_string)
        })
        .collect();
    names.sort();
    Ok(names)
}

/// The tickets in the queue, in order, with their bodies. Tickets that vanish while being read
/// are skipped.
pub fn list(kd: &KeyDir) -> io::Result<Vec<Ticket>> {
    let mut out = Vec::new();
    for name in names(kd)? {
        let path = kd.queue_dir().join(format!("{name}.json"));
        if let Some(body) = read_body(&path, &kd.key.text)? {
            out.push(Ticket { name, path, body });
        }
    }
    Ok(out)
}

/// Removes a ticket (gone already is fine).
pub fn remove(ticket: &Ticket) -> io::Result<()> {
    fsx::remove_with_retry(&ticket.path).map(|_| ())
}
