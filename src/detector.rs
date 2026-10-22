use std::{
    collections::VecDeque,
    env, fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc::{Receiver, SyncSender},
    thread,
};

use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;

use crate::{
    detection_cache,
    document::CbzDocument,
    library,
    progress::ProgressStore,
    region::{Region, order_rtl},
};

pub enum CommandMessage {
    Prioritize(usize),
    Stop,
}

#[derive(Debug)]
pub struct ResultMessage {
    pub book: PathBuf,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Task {
    book: usize,
    page: usize,
}

fn schedule(counts: &[usize], start: usize) -> VecDeque<Task> {
    let mut queue = VecDeque::new();
    let Some(&current) = counts.first() else {
        return queue;
    };
    // Never consume pages behind the reading position until all forward
    // volumes have been analyzed. The visible volume always takes priority.
    queue.extend((start.min(current)..current).map(|page| Task { book: 0, page }));
    for (book, &count) in counts.iter().enumerate().skip(1) {
        queue.extend((0..count).map(|page| Task { book, page }));
    }
    queue.extend((0..start.min(current)).map(|page| Task { book: 0, page }));
    queue
}

fn prioritize(queue: &mut VecDeque<Task>, page: usize) {
    let target = Task { book: 0, page };
    if queue.contains(&target) {
        queue.retain(|task| *task != target);
        queue.push_front(target);
    }
}

fn run(
    book: &Path,
    start: usize,
    requests: Receiver<CommandMessage>,
    results: &SyncSender<Result<ResultMessage, String>>,
) -> Result<()> {
    let mut process = Process::start()?;
    let store = ProgressStore::open()?;
    run_with(book, start, requests, results, &mut process, &store)
}

fn run_with(
    book: &Path,
    start: usize,
    requests: Receiver<CommandMessage>,
    results: &SyncSender<Result<ResultMessage, String>>,
    process: &mut Process,
    store: &ProgressStore,
) -> Result<()> {
    let mut documents = vec![(book.to_path_buf(), CbzDocument::open(book)?)];
    for path in library::later_volumes(book) {
        match CbzDocument::open(&path) {
            Ok(document) => documents.push((path, document)),
            Err(error) => {
                let _ = results.send(Err(format!("skipping unreadable volume: {error}")));
            }
        }
    }
    let counts: Vec<_> = documents.iter().map(|(_, doc)| doc.page_count()).collect();
    let mut queue = schedule(&counts, start);

    while !queue.is_empty() {
        while let Ok(message) = requests.try_recv() {
            match message {
                CommandMessage::Prioritize(page) if page < counts[0] => {
                    prioritize(&mut queue, page);
                }
                CommandMessage::Prioritize(_) => {}
                CommandMessage::Stop => return Ok(()),
            }
        }
        let task = queue.pop_front().expect("nonempty task queue");
        let (path, document) = &mut documents[task.book];
        let image = document.load_page(task.page)?;
        if detection_cache::load_external(
            store.connection(),
            path,
            task.page,
            image.width(),
            image.height(),
        )?
        .is_some()
        {
            continue;
        }

        match process.detect(task.page, &image, path) {
            Ok(mut result) => {
                ensure!(
                    result.page == task.page,
                    "detector returned a different page index"
                );
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
                    path,
                    task.page,
                    image.width(),
                    image.height(),
                    &result.source,
                    &result.regions,
                )?;
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
        Self::start_with_command(command)
    }

    fn start_with_command(command: impl AsRef<std::ffi::OsStr>) -> Result<Self> {
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

    fn detect(&mut self, page: usize, image: &RgbaImage, book: &Path) -> Result<ResultMessage> {
        let path = self.temporary.join(format!("page-{page:04}.png"));
        image.save(&path)?;
        writeln!(self.input, "{page}\t{}", path.display())?;
        self.input.flush()?;
        let mut line = String::new();
        self.output.read_line(&mut line)?;
        let _ = fs::remove_file(path);
        ensure!(!line.is_empty(), "detector exited unexpectedly");
        parse_response(&line, book)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.temporary);
    }
}

fn parse_response(line: &str, book: &Path) -> Result<ResultMessage> {
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
        book: book.to_path_buf(),
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
    order_rtl(&mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::{Process, Task, parse_response, prioritize, run_with, sanitize, schedule};
    use crate::{detection_cache, progress::ProgressStore, region::Region};
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use std::{
        fs,
        io::{Cursor, Write},
        os::unix::fs::PermissionsExt,
        path::Path,
        sync::mpsc,
        time::SystemTime,
    };

    #[test]
    fn parses_worker_response() {
        let result = parse_response(
            r#"{"page":3,"source":"example","regions":[[1,2,30,40]],"seconds":1.25}"#,
            Path::new("example.cbz"),
        )
        .unwrap();
        assert_eq!(result.page, 3);
        assert_eq!(result.book, Path::new("example.cbz"));
        assert_eq!(result.source, "example");
        assert_eq!(result.regions[0].width, 30);
        assert_eq!(result.seconds, Some(1.25));
    }

    #[test]
    fn worker_panels_keep_the_right_column_together() {
        let left = Region {
            x: 0,
            y: 0,
            width: 90,
            height: 210,
        };
        let top = Region {
            x: 110,
            y: 0,
            width: 90,
            height: 90,
        };
        let bottom = Region {
            x: 110,
            y: 110,
            width: 90,
            height: 90,
        };
        assert_eq!(
            sanitize(vec![left, bottom, top], 200, 210),
            [top, bottom, left]
        );
    }

    #[test]
    fn schedules_reading_position_then_forward_volumes_then_earlier_pages() {
        let tasks: Vec<_> = schedule(&[5, 2, 1], 3).into_iter().collect();
        assert_eq!(
            tasks,
            [
                Task { book: 0, page: 3 },
                Task { book: 0, page: 4 },
                Task { book: 1, page: 0 },
                Task { book: 1, page: 1 },
                Task { book: 2, page: 0 },
                Task { book: 0, page: 0 },
                Task { book: 0, page: 1 },
                Task { book: 0, page: 2 },
            ]
        );
    }

    #[test]
    fn requested_current_volume_page_jumps_ahead_without_duplicating_work() {
        let mut queue = schedule(&[5, 2], 3);
        prioritize(&mut queue, 1);
        assert_eq!(queue.front(), Some(&Task { book: 0, page: 1 }));
        assert_eq!(
            queue
                .iter()
                .filter(|task| task.book == 0 && task.page == 1)
                .count(),
            1
        );
        queue.pop_front();
        prioritize(&mut queue, 1);
        assert!(!queue.contains(&Task { book: 0, page: 1 }));
    }

    #[test]
    fn mock_worker_caches_forward_volumes_without_confusing_identical_page_numbers() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "panel-detector-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let script = root.join("worker.sh");
        fs::write(
            &script,
            b"#!/bin/sh\nprintf '{\"ready\":true}\\n'\nwhile read -r page image; do\n    printf '{\"page\":%s,\"source\":\"mock\",\"regions\":[[0,0,4,4]]}\\n' \"$page\"\ndone\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255])));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        let books = [
            root.join("Climber - Volume_01.cbz"),
            root.join("Climber - Volume_02.cbz"),
        ];
        for (index, path) in books.iter().enumerate() {
            let file = fs::File::create(path).unwrap();
            let mut archive = zip::ZipWriter::new(file);
            for page in 0..(2 - index) {
                archive
                    .start_file(
                        format!("Ch.1/{page:03}.png"),
                        zip::write::SimpleFileOptions::default(),
                    )
                    .unwrap();
                archive.write_all(png.get_ref()).unwrap();
            }
            archive.finish().unwrap();
        }
        let store = ProgressStore::in_memory().unwrap();
        let mut process = Process::start_with_command(&script).unwrap();
        let (_requests_tx, requests) = mpsc::channel();
        let (results_tx, results_rx) = mpsc::sync_channel(8);
        run_with(&books[0], 1, requests, &results_tx, &mut process, &store).unwrap();
        let results: Vec<_> = results_rx
            .try_iter()
            .map(|result| result.unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .map(|result| (&result.book, result.page))
                .collect::<Vec<_>>(),
            [(&books[0], 1), (&books[1], 0), (&books[0], 0)]
        );
        for (book, page) in [(&books[0], 1), (&books[1], 0), (&books[0], 0)] {
            assert!(
                detection_cache::load_external(store.connection(), book, page, 4, 4)
                    .unwrap()
                    .is_some()
            );
        }
        drop(process);
        fs::remove_dir_all(root).unwrap();
    }
}
