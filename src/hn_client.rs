//! Experimental metadata-only HaruNeko client. The backend remains behind its
//! independent certificate-review gate; never launch it outside net-sandbox.sh.
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{self, Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use anyhow::{Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub title: String,
}

pub enum Request {
    Sources,
    Search {
        source: String,
        query: String,
    },
    Chapters {
        source: String,
        id: String,
        title: String,
    },
    Stop,
}

pub enum Failure {
    Unavailable,
    Connector,
}

pub enum Event {
    Loaded(Vec<Item>),
    Failed(Failure),
}

pub struct Client {
    pub requests: Sender<Request>,
    pub events: Receiver<Event>,
    child: Arc<Mutex<Option<Child>>>,
    cancel: Arc<AtomicBool>,
}

impl Client {
    pub fn start() -> Self {
        let (requests, incoming) = mpsc::channel();
        let (outgoing, events) = mpsc::channel();
        let child = Arc::new(Mutex::new(None));
        let active = Arc::clone(&child);
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&cancel);
        thread::spawn(move || {
            if run(incoming, &outgoing, &active, &stopped).is_err() {
                let _ = outgoing.send(Event::Failed(Failure::Unavailable));
            }
        });
        Self {
            requests,
            events,
            child,
            cancel,
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.requests.send(Request::Stop);
        // Interrupt a hung website request on leaving Search. The shell's TERM
        // trap tears down the network namespace, Xvfb and browser children.
        if let Ok(guard) = self.child.lock() {
            if let Some(child) = guard.as_ref() {
                unsafe {
                    libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
                }
            }
        }
    }
}

fn find_tool(name: &str) -> Result<PathBuf> {
    let path = env::var_os("PATH").ok_or_else(|| anyhow::anyhow!("missing PATH"))?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|tool| tool.is_file())
        .ok_or_else(|| anyhow::anyhow!("required tool unavailable"))
}

fn run(
    incoming: Receiver<Request>,
    events: &Sender<Event>,
    active: &Arc<Mutex<Option<Child>>>,
    cancel: &AtomicBool,
) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hn/net-sandbox.sh");
    ensure!(script.is_file(), "sandbox launcher is unavailable");
    let electron = find_tool("electron")?;
    let xvfb = find_tool("Xvfb")?;
    let mut command = Command::new(script);
    command
        .arg("/bin/sh")
        .arg("/app/backend/app/run.sh")
        .arg(xvfb)
        .arg(electron)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let parent = process::id() as libc::pid_t;
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("missing backend pipe"))?;
    let mut output = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing backend pipe"))?,
    );
    *active
        .lock()
        .map_err(|_| anyhow::anyhow!("backend lock failed"))? = Some(child);
    let result = (|| -> Result<()> {
        for request in incoming {
            if cancel.load(Ordering::Relaxed) || matches!(request, Request::Stop) {
                break;
            }
            let line = encode_request(&request)?;
            input.write_all(line.as_bytes())?;
            input.write_all(b"\n")?;
            input.flush()?;
            match read_reply(&mut output) {
                Ok(items) => {
                    if events.send(Event::Loaded(items)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let failure = if error.to_string() == "backend request failed" {
                        Failure::Connector
                    } else {
                        Failure::Unavailable
                    };
                    let _ = events.send(Event::Failed(failure));
                    break;
                }
            }
        }
        Ok(())
    })();
    if let Some(mut child) = active
        .lock()
        .map_err(|_| anyhow::anyhow!("backend lock failed"))?
        .take()
    {
        // The backend is no longer needed; do not keep a browser open in the
        // background when the selector closes.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let _ = child.wait();
    }
    result
}

fn source_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 80
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
        "invalid source"
    );
    Ok(())
}

fn encode_request(request: &Request) -> Result<String> {
    match request {
        Request::Sources => Ok(r#"{"id":1,"op":"sources-panel"}"#.into()),
        Request::Search { source, query } => {
            source_id(source)?;
            ensure!(
                (2..=120).contains(&query.chars().count()) && query.len() <= 480,
                "invalid query"
            );
            Ok(format!(
                r#"{{"id":1,"op":"search-panel","source":"{source}","queryB64":"{}"}}"#,
                STANDARD.encode(query)
            ))
        }
        Request::Chapters { source, id, title } => {
            source_id(source)?;
            ensure!(
                !id.is_empty() && id.len() <= 1024 && !title.is_empty() && title.len() <= 1024,
                "invalid chapter request"
            );
            Ok(format!(
                r#"{{"id":1,"op":"chapters-panel","source":"{source}","mangaIdB64":"{}","mangaTitleB64":"{}"}}"#,
                STANDARD.encode(id),
                STANDARD.encode(title)
            ))
        }
        Request::Stop => bail!("not a backend request"),
    }
}

fn safe_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(256)
        .collect()
}

fn read_reply(input: &mut impl BufRead) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    loop {
        let mut line = Vec::new();
        let read = input.take(8193).read_until(b'\n', &mut line)?;
        ensure!(
            read > 0 && read <= 8192 && line.last() == Some(&b'\n'),
            "invalid backend response"
        );
        let text = std::str::from_utf8(&line)?;
        let parts: Vec<_> = text.trim_end_matches('\n').split('\t').collect();
        ensure!(parts.first() == Some(&"1"), "unmatched response");
        match parts.as_slice() {
            [_, "end"] => return Ok(items),
            [_, "error"] => bail!("backend request failed"),
            [_, "item", id, title] if items.len() < 2_000 => {
                let id = STANDARD.decode(id)?;
                let title = STANDARD.decode(title)?;
                ensure!(
                    !id.is_empty() && id.len() <= 1024 && title.len() <= 2048,
                    "invalid metadata"
                );
                items.push(Item {
                    id: String::from_utf8(id)?,
                    title: safe_text(&String::from_utf8(title)?),
                });
            }
            _ => bail!("invalid metadata response"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Item, Request, encode_request, read_reply};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use std::io::Cursor;

    #[test]
    fn backend_wire_round_trip_and_bounds() {
        assert_eq!(
            encode_request(&Request::Sources).unwrap(),
            r#"{"id":1,"op":"sources-panel"}"#
        );
        let line = encode_request(&Request::Search {
            source: "mangadex".into(),
            query: "恋人 \";\\".into(),
        })
        .unwrap();
        assert!(line.contains("queryB64"));
        assert!(!line.contains('恋'));
        let item = read_reply(&mut Cursor::new(
            b"1\titem\tbWFuZ2FkZXg=\tVGVzdA==\n1\tend\n",
        ))
        .unwrap();
        assert_eq!(
            item,
            vec![Item {
                id: "mangadex".into(),
                title: "Test".into()
            }]
        );
        assert!(read_reply(&mut Cursor::new(b"1\titem\t%%%\tVGVzdA==\n1\tend\n")).is_err());
        assert!(read_reply(&mut Cursor::new(b"1\terror\n")).is_err());
        assert!(read_reply(&mut Cursor::new(b"1\titem\tAA==\tAA==\n")).is_err());
        let text = STANDARD.encode("Climber\u{1b}[31m\u{202e} next\n");
        let wire = format!("1\titem\tYm9vaw==\t{text}\n1\tend\n");
        let safe = read_reply(&mut Cursor::new(wire)).unwrap();
        assert_eq!(safe[0].title, "Climber[31m next");
    }
}
