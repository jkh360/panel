//! Isolated HaruNeko metadata and chapter-image pipe. Never launch the
//! backend outside net-sandbox.sh or grant it access to output directories.
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{self, Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use crate::hn_download::CbzOutput;
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
    Download {
        source: Item,
        manga: Item,
        chapter: Item,
        directory: PathBuf,
    },
    DownloadAll {
        source: Item,
        manga: Item,
        chapters: Vec<Item>,
        directory: PathBuf,
    },
    Stop,
}

pub enum Failure {
    Unavailable,
    Connector,
    Download,
}

pub enum Event {
    Loaded(Vec<Item>),
    DownloadProgress {
        done: usize,
        total: usize,
    },
    Downloaded(PathBuf),
    BatchProgress {
        done: usize,
        total: usize,
        saved: usize,
        skipped: usize,
    },
    BatchFinished {
        saved: usize,
        skipped: usize,
    },
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
            if let Request::DownloadAll {
                source,
                manga,
                chapters,
                directory,
            } = &request
            {
                if chapters.len() > 2_000 || chapters.is_empty() {
                    let _ = events.send(Event::Failed(Failure::Download));
                    break;
                }
                let mut saved = 0;
                let mut skipped = 0;
                for (index, chapter) in chapters.iter().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if download_chapter(
                        &mut input,
                        &mut output,
                        (source, manga, chapter, directory),
                        events,
                        cancel,
                    )
                    .is_ok()
                    {
                        saved += 1;
                    } else {
                        skipped += 1;
                    }
                    let _ = events.send(Event::BatchProgress {
                        done: index + 1,
                        total: chapters.len(),
                        saved,
                        skipped,
                    });
                }
                if !cancel.load(Ordering::Relaxed) {
                    let _ = events.send(Event::BatchFinished { saved, skipped });
                }
                continue;
            }
            if let Request::Download {
                source,
                manga,
                chapter,
                directory,
            } = &request
            {
                match download_chapter(
                    &mut input,
                    &mut output,
                    (source, manga, chapter, directory),
                    events,
                    cancel,
                ) {
                    Ok(path) => {
                        if events.send(Event::Downloaded(path)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = events.send(Event::Failed(Failure::Download));
                        break;
                    }
                }
                continue;
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
        Request::Download { .. } | Request::DownloadAll { .. } | Request::Stop => {
            bail!("not a metadata request")
        }
    }
}

fn write_command(input: &mut impl Write, request: &str) -> Result<()> {
    input.write_all(request.as_bytes())?;
    input.write_all(b"\n")?;
    input.flush()?;
    Ok(())
}

fn read_download_line(input: &mut impl BufRead, limit: u64) -> Result<String> {
    let mut bytes = Vec::new();
    let read = input.take(limit + 1).read_until(b'\n', &mut bytes)?;
    ensure!(
        read > 0 && read as u64 <= limit && bytes.last() == Some(&b'\n'),
        "invalid download reply"
    );
    Ok(String::from_utf8(bytes)?.trim_end_matches('\n').to_owned())
}

fn download_chapter(
    input: &mut impl Write,
    output: &mut impl BufRead,
    (source, manga, chapter, directory): (&Item, &Item, &Item, &Path),
    events: &Sender<Event>,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    source_id(&source.id)?;
    ensure!(
        !manga.id.is_empty()
            && manga.id.len() <= 1024
            && manga.title.len() <= 1024
            && !chapter.id.is_empty()
            && chapter.id.len() <= 1024,
        "invalid chapter"
    );
    write_command(
        input,
        &format!(
            r#"{{"id":1,"op":"prepare-download-panel","source":"{}","mangaIdB64":"{}","mangaTitleB64":"{}","chapterIdB64":"{}"}}"#,
            source.id,
            STANDARD.encode(&manga.id),
            STANDARD.encode(&manga.title),
            STANDARD.encode(&chapter.id),
        ),
    )?;
    let prepared = read_download_line(output, 256)?;
    let parts: Vec<_> = prepared.split('\t').collect();
    ensure!(
        parts.len() == 4 && parts[0] == "1" && parts[1] == "prepared",
        "backend request failed"
    );
    let token = parts[2];
    ensure!(
        token.len() == 36 && token.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'),
        "invalid token"
    );
    let count: usize = parts[3].parse()?;
    ensure!((1..=300).contains(&count), "invalid page count");
    let mut archive = CbzOutput::new(
        directory,
        &source.id,
        &manga.title,
        &chapter.title,
        &chapter.id,
    )?;
    for index in 0..count {
        ensure!(!cancel.load(Ordering::Relaxed), "download cancelled");
        write_command(
            input,
            &format!(r#"{{"id":1,"op":"fetch-page-panel","token":"{token}","index":{index}}}"#,),
        )?;
        let reply = read_download_line(output, 16_000_040)?;
        let encoded = reply
            .strip_prefix("1\tpage\t")
            .ok_or_else(|| anyhow::anyhow!("backend request failed"))?;
        ensure!(encoded.len() <= 16_000_000, "oversized image");
        let bytes = STANDARD.decode(encoded)?;
        archive.add_page(index, &bytes)?;
        let _ = events.send(Event::DownloadProgress {
            done: index + 1,
            total: count,
        });
    }
    write_command(
        input,
        &format!(r#"{{"id":1,"op":"finish-download-panel","token":"{token}"}}"#,),
    )?;
    ensure!(
        read_download_line(output, 128)? == "1\tdone",
        "backend request failed"
    );
    archive.finish()
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
    use super::{Item, Request, download_chapter, encode_request, read_reply};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use std::{
        fs,
        io::Cursor,
        sync::{atomic::AtomicBool, mpsc},
    };

    #[test]
    fn chapter_download_writes_a_complete_cbz_from_bounded_pipe() {
        let dir = std::env::temp_dir().join(format!("panel-worker-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let token = "00000000-0000-4000-8000-000000000001";
        let reply = format!(
            "1\tprepared\t{token}\t1\n1\tpage\t{}\n1\tdone\n",
            STANDARD.encode(png)
        );
        let (events, progress) = mpsc::channel();
        let mut input = Vec::new();
        let mut output = Cursor::new(reply);
        let path = download_chapter(
            &mut input,
            &mut output,
            (
                &Item {
                    id: "source".into(),
                    title: "Source".into(),
                },
                &Item {
                    id: "manga".into(),
                    title: "Manga".into(),
                },
                &Item {
                    id: "chapter".into(),
                    title: "Chapter".into(),
                },
                &dir,
            ),
            &events,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(matches!(
            progress.try_recv().unwrap(),
            super::Event::DownloadProgress { done: 1, total: 1 }
        ));
        let wire = String::from_utf8(input).unwrap();
        assert!(wire.contains("prepare-download-panel"));
        assert!(wire.contains("fetch-page-panel"));
        assert!(wire.contains("finish-download-panel"));
        assert_eq!(
            zip::ZipArchive::new(fs::File::open(&path).unwrap())
                .unwrap()
                .len(),
            1
        );
        fs::remove_dir_all(dir).unwrap();
    }

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
