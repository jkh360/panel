use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc::{Receiver, SyncSender},
    thread,
};

use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;

use crate::{detection_cache, document::CbzDocument, progress::ProgressStore, region::Region};

pub enum CommandMessage {
    Prioritize(usize),
    Stop,
}

#[derive(Debug)]
pub struct ResultMessage {
    pub page: usize,
    pub regions: Vec<Region>,
    pub source: String,
    pub seconds: Option<f64>,
}

pub fn configured() -> bool {
    env::var_os("PANEL_DETECTOR_COMMAND").is_some()
}

pub fn spawn(
    book: PathBuf,
    start: usize,
    requests: Receiver<CommandMessage>,
    results: SyncSender<Result<ResultMessage, String>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(error) = run(&book, start, requests, &results) {
            let _ = results.send(Err(error.to_string()));
        }
    })
}

fn run(
    book: &Path,
    start: usize,
    requests: Receiver<CommandMessage>,
    results: &SyncSender<Result<ResultMessage, String>>,
) -> Result<()> {
    let mut process = Process::start()?;
    let mut document = CbzDocument::open(book)?;
    let store = ProgressStore::open()?;
    let count = document.page_count();
    let mut queue: Vec<usize> = (start..count).chain(0..start).collect();
    let mut done = vec![false; count];

    while let Some(page) = queue.first().copied() {
        while let Ok(message) = requests.try_recv() {
            match message {
                CommandMessage::Prioritize(page) if page < count && !done[page] => {
                    queue.retain(|candidate| *candidate != page);
                    queue.insert(0, page);
                }
                CommandMessage::Prioritize(_) => {}
                CommandMessage::Stop => return Ok(()),
            }
        }
        queue.remove(0);

        let image = document.load_page(page)?;
        if detection_cache::load_external(
            store.connection(),
            book,
            page,
            image.width(),
            image.height(),
        )?
        .is_some()
        {
            done[page] = true;
            continue;
        }

        match process.detect(page, &image) {
            Ok(mut result) => {
                result.regions = sanitize(result.regions, image.width(), image.height());
                if result.regions.is_empty() {
                    result.regions.push(Region {
                        x: 0,
                        y: 0,
                        width: image.width(),
                        height: image.height(),
                    });
                    result.source.push_str(": whole-page");
                }
                detection_cache::save_external(
                    store.connection(),
                    book,
                    page,
                    image.width(),
                    image.height(),
                    &result.source,
                    &result.regions,
                )?;
                done[page] = true;
                let _ = results.send(Ok(result));
            }
            Err(error) => {
                let _ = results.send(Err(error.to_string()));
            }
        }
    }
    Ok(())
}

struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    temporary: PathBuf,
}

impl Process {
    fn start() -> Result<Self> {
        let command =
            env::var_os("PANEL_DETECTOR_COMMAND").context("PANEL_DETECTOR_COMMAND is not set")?;
        let mut child = Command::new(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to start external detector")?;
        let input = child.stdin.take().context("detector stdin unavailable")?;
        let mut output =
            BufReader::new(child.stdout.take().context("detector stdout unavailable")?);
        let mut ready = String::new();
        output.read_line(&mut ready)?;
        ensure!(
            ready.trim() == r#"{"ready":true}"#,
            "detector did not become ready: {ready}"
        );
        let temporary = env::temp_dir().join(format!("panel-detector-{}", std::process::id()));
        fs::create_dir_all(&temporary)?;
        Ok(Self {
            child,
            input,
            output,
            temporary,
        })
    }

    fn detect(&mut self, page: usize, image: &RgbaImage) -> Result<ResultMessage> {
        let path = self.temporary.join(format!("page-{page:04}.png"));
        image.save(&path)?;
        writeln!(self.input, "{page}\t{}", path.display())?;
        self.input.flush()?;
        let mut line = String::new();
        self.output.read_line(&mut line)?;
        let _ = fs::remove_file(path);
        ensure!(!line.is_empty(), "detector exited unexpectedly");
        parse_response(&line)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.temporary);
    }
}

fn parse_response(line: &str) -> Result<ResultMessage> {
    if line.contains("\"error\"") {
        bail!("external detector failed: {line}")
    }
    let page = unsigned(line, "page")? as usize;
    let source = string(line, "source").unwrap_or_else(|| "external".to_owned());
    let seconds = decimal(line, "seconds");
    let body = line
        .split_once("\"regions\":[")
        .context("missing regions")?
        .1
        .rsplit_once(']')
        .context("invalid regions")?
        .0;
    let values: Vec<u32> = body
        .split(|character: char| !character.is_ascii_digit())
        .filter(|value| !value.is_empty())
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(values.len() % 4 == 0, "invalid region array");
    Ok(ResultMessage {
        page,
        source,
        seconds,
        regions: values
            .chunks_exact(4)
            .map(|value| Region {
                x: value[0],
                y: value[1],
                width: value[2],
                height: value[3],
            })
            .collect(),
    })
}

fn unsigned(line: &str, key: &str) -> Result<u32> {
    let marker = format!("\"{key}\":");
    Ok(line
        .split_once(&marker)
        .context("missing number")?
        .1
        .split(|character: char| !character.is_ascii_digit())
        .next()
        .unwrap_or_default()
        .parse()?)
}

fn decimal(line: &str, key: &str) -> Option<f64> {
    let marker = format!("\"{key}\":");
    line.split_once(&marker)?
        .1
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .next()?
        .parse()
        .ok()
}

fn string(line: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    Some(line.split_once(&marker)?.1.split_once('"')?.0.to_owned())
}

fn sanitize(regions: Vec<Region>, width: u32, height: u32) -> Vec<Region> {
    let page_area = u64::from(width) * u64::from(height);
    let mut output: Vec<_> = regions
        .into_iter()
        .filter_map(|region| {
            let x = region.x.min(width);
            let y = region.y.min(height);
            let right = region.right().min(width);
            let bottom = region.bottom().min(height);
            (right > x && bottom > y).then_some(Region {
                x,
                y,
                width: right - x,
                height: bottom - y,
            })
        })
        .filter(|region| region.area() as f64 / page_area as f64 >= 0.025)
        .collect();
    output.sort_by(|a, b| {
        let tolerance = a.height.min(b.height) / 3;
        if a.y.abs_diff(b.y) <= tolerance {
            b.x.cmp(&a.x)
        } else {
            a.y.cmp(&b.y)
        }
    });
    output
}

#[cfg(test)]
mod tests {
    use super::parse_response;

    #[test]
    fn parses_worker_response() {
        let result = parse_response(
            r#"{"page":3,"source":"example","regions":[[1,2,30,40]],"seconds":1.25}"#,
        )
        .unwrap();
        assert_eq!(result.page, 3);
        assert_eq!(result.source, "example");
        assert_eq!(result.regions[0].width, 30);
        assert_eq!(result.seconds, Some(1.25));
    }
}
