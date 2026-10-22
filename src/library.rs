use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use image::DynamicImage;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_image::{
    Resize, StatefulImage,
    picker::{Picker, ProtocolType},
    protocol::StatefulProtocol,
};

use crate::{document::CbzDocument, progress::ProgressStore};

const BG: Color = Color::Rgb(40, 40, 40);
const FG: Color = Color::Rgb(235, 219, 178);
const MUTED: Color = Color::Rgb(168, 153, 132);
const ACCENT: Color = Color::Rgb(184, 187, 38);
const YELLOW: Color = Color::Rgb(250, 189, 47);

pub struct Selection {
    pub path: PathBuf,
    pub page: Option<usize>,
    location: Option<Location>,
}

impl Selection {
    pub fn direct(path: PathBuf) -> Self {
        Self {
            path,
            page: None,
            location: None,
        }
    }
}

struct Location {
    directory: PathBuf,
    level: Level,
    selected: usize,
}

#[derive(Clone)]
enum Entry {
    Directory(PathBuf),
    Series(String),
    Volume {
        label: String,
        path: PathBuf,
    },
    Chapter {
        number: u32,
        path: PathBuf,
        page: usize,
        end: usize,
        saved: Option<usize>,
    },
    LastRead {
        path: PathBuf,
        description: String,
    },
    Read {
        path: PathBuf,
    },
}

impl Entry {
    fn label(&self) -> String {
        match self {
            Self::Directory(path) => format!(
                "▸ {}/",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Self::Series(name) => format!("▸ {name}"),
            Self::Volume { label, .. } => format!("▸ {label}"),
            Self::Chapter {
                number,
                page,
                end,
                saved,
                ..
            } => match saved {
                Some(current) if (*page..*end).contains(current) => format!(
                    "  Chapter {number}  ·  {}/{}",
                    current - page + 1,
                    end - page
                ),
                Some(current) if current >= end => format!("  Chapter {number}  ✓"),
                _ => format!("  Chapter {number}  ·  {} pages", end - page),
            },
            Self::LastRead { description, .. } => format!("↳ Last read\n    {description}"),
            Self::Read { .. } => "  Read volume".into(),
        }
    }

    fn book(&self) -> Option<&Path> {
        match self {
            Self::Volume { path, .. }
            | Self::Chapter { path, .. }
            | Self::LastRead { path, .. }
            | Self::Read { path } => Some(path),
            _ => None,
        }
    }
}

#[derive(Clone)]
enum Level {
    Directory,
    Series(String),
    Volume(PathBuf),
}

struct Preview {
    path: PathBuf,
    result: Result<(DynamicImage, usize), String>,
}

struct Library {
    directory: PathBuf,
    level: Level,
    entries: Vec<Entry>,
    selected: usize,
    preview: Option<Preview>,
    preview_target: Option<PathBuf>,
    cover_cache: HashMap<PathBuf, (DynamicImage, usize)>,
    results: Receiver<Preview>,
    sender: mpsc::Sender<Preview>,
    notice: Option<String>,
    picker: Picker,
    image: Option<StatefulProtocol>,
    progress: Option<usize>,
}

pub fn select(previous: Option<&Selection>) -> Result<Option<Selection>> {
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "panel needs an interactive terminal when started without a book"
    );
    let home = PathBuf::from(env::var_os("HOME").context("HOME is not set")?);
    let directory = default_directory()
        .filter(|path| path.is_dir())
        .unwrap_or(home);
    let (directory, level, selected) = resume_location(previous, directory);
    let (sender, results) = mpsc::channel();
    let mut terminal = ratatui::init();
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((10, 20)));
    if env::var("TERM").is_ok_and(|term| term.contains("ghostty")) {
        picker.set_protocol_type(ProtocolType::Kitty);
    }
    let mut library = Library {
        directory,
        level,
        entries: Vec::new(),
        selected,
        preview: None,
        preview_target: None,
        cover_cache: HashMap::new(),
        results,
        sender,
        notice: None,
        picker,
        image: None,
        progress: None,
    };
    let outcome = library.refresh().and_then(|_| library.run(&mut terminal));
    ratatui::restore();
    outcome
}

fn resume_location(previous: Option<&Selection>, default: PathBuf) -> (PathBuf, Level, usize) {
    match previous {
        Some(Selection {
            location: Some(location),
            ..
        }) if location.directory.is_dir() => (
            location.directory.clone(),
            location.level.clone(),
            location.selected,
        ),
        Some(selection) if selection.location.is_none() => {
            let parent = selection
                .path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            if parent.is_dir() {
                (
                    parent.to_path_buf(),
                    Level::Volume(selection.path.clone()),
                    0,
                )
            } else {
                (default, Level::Directory, 0)
            }
        }
        _ => (default, Level::Directory, 0),
    }
}

impl Library {
    fn files(&self) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let mut directories = Vec::new();
        let mut books = Vec::new();
        for item in fs::read_dir(&self.directory)? {
            let item = item?;
            if item.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = item.path();
            let Ok(kind) = item.file_type() else {
                continue;
            };
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_file() && is_cbz(&path) {
                books.push(path);
            }
        }
        directories.retain(|path| !is_sidecar_folder(path, &books));
        directories.sort_by(|a, b| {
            natord::compare_ignore_case(&a.to_string_lossy(), &b.to_string_lossy())
        });
        books.sort_by(|a, b| {
            natord::compare_ignore_case(&a.to_string_lossy(), &b.to_string_lossy())
        });
        Ok((directories, books))
    }

    fn refresh(&mut self) -> Result<()> {
        let (directories, books) = self.files()?;
        self.entries = match &self.level {
            Level::Directory => {
                let mut entries: Vec<_> = directories.into_iter().map(Entry::Directory).collect();
                let mut series: Vec<_> = books.iter().map(|path| series_title(path)).collect();
                series.sort_by(|a, b| natord::compare_ignore_case(a, b));
                series.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
                entries.extend(series.into_iter().map(Entry::Series));
                entries
            }
            Level::Series(name) => {
                let volumes: Vec<_> = books
                    .into_iter()
                    .filter(|path| series_title(path).eq_ignore_ascii_case(name))
                    .collect();
                let last = latest_volume(&volumes);
                let mut entries: Vec<_> = volumes
                    .into_iter()
                    .map(|path| Entry::Volume {
                        label: volume_title(&path),
                        path,
                    })
                    .collect();
                if let Some(last) = last {
                    entries.insert(0, last);
                }
                entries
            }
            Level::Volume(path) => {
                let book = CbzDocument::open(path)?;
                let saved = ProgressStore::open()
                    .ok()
                    .and_then(|store| store.load(path).ok().flatten())
                    .filter(|position| position.page < book.page_count());
                let chapters = chapter_starts(&book);
                let mut entries = Vec::new();
                if let Some(position) = saved {
                    entries.push(Entry::LastRead {
                        path: path.clone(),
                        description: chapter_description(&chapters, position.page),
                    });
                }
                if chapters.is_empty() {
                    entries.push(Entry::Read { path: path.clone() });
                } else {
                    let starts: Vec<_> = chapters.into_iter().collect();
                    entries.extend(starts.iter().enumerate().map(|(index, &(number, page))| {
                        let end = starts
                            .get(index + 1)
                            .map_or(book.page_count(), |(_, next)| *next);
                        Entry::Chapter {
                            number,
                            path: path.clone(),
                            page,
                            end,
                            saved: saved.map(|position| position.page),
                        }
                    }));
                }
                entries
            }
        };
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        self.request_preview();
        Ok(())
    }

    fn request_preview(&mut self) {
        let target = self
            .entries
            .get(self.selected)
            .and_then(|entry| match entry {
                Entry::Series(name) => self.files().ok().and_then(|(_, books)| {
                    books
                        .into_iter()
                        .find(|path| series_title(path).eq_ignore_ascii_case(name))
                }),
                _ => entry.book().map(Path::to_path_buf),
            });
        if target == self.preview_target {
            return;
        }
        self.preview = None;
        self.image = None;
        self.progress = None;
        self.preview_target = target;
        let Some(path) = self.preview_target.clone() else {
            return;
        };
        if let Some(cached) = self.cover_cache.get(&path) {
            self.accept_preview(Preview {
                path: path.clone(),
                result: Ok(cached.clone()),
            });
            return;
        }
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let mut book = CbzDocument::open(&path).map_err(|error| error.to_string())?;
                let pages = book.page_count();
                let cover = sidecar_cover(&path)
                    .and_then(|cover| load_sidecar_cover(&cover).ok())
                    .map_or_else(|| book.load_cover().map_err(|error| error.to_string()), Ok)?;
                Ok((cover.thumbnail(1200, 1600), pages))
            })();
            let _ = sender.send(Preview { path, result });
        });
    }

    fn accept_preview(&mut self, preview: Preview) {
        if let Ok((cover, pages)) = &preview.result {
            if self.cover_cache.len() >= 16 {
                self.cover_cache.clear();
            }
            self.cover_cache
                .insert(preview.path.clone(), (cover.clone(), *pages));
            self.image = Some(self.picker.new_resize_protocol(cover.clone()));
            self.progress = ProgressStore::open()
                .ok()
                .and_then(|store| store.load(&preview.path).ok().flatten())
                .map(|position| position.page + 1);
        }
        self.preview = Some(preview);
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<Option<Selection>> {
        let mut redraw = true;
        loop {
            while let Ok(preview) = self.results.try_recv() {
                if self.preview_target.as_deref() == Some(preview.path.as_path()) {
                    self.accept_preview(preview);
                    redraw = true;
                }
            }
            if redraw {
                terminal.draw(|frame| self.draw(frame))?;
                redraw = false;
            }
            if !event::poll(Duration::from_millis(80))? {
                continue;
            }
            redraw = true;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                KeyCode::Char('j') | KeyCode::Down => {
                    let next = (self.selected + 1).min(self.entries.len().saturating_sub(1));
                    if next != self.selected {
                        self.selected = next;
                        self.request_preview();
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    let next = self.selected.saturating_sub(1);
                    if next != self.selected {
                        self.selected = next;
                        self.request_preview();
                    }
                }
                KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => self.parent()?,
                KeyCode::Char('d') => match set_default_directory(&self.directory) {
                    Ok(()) => {
                        self.notice = Some(format!("Default folder: {}", self.directory.display()))
                    }
                    Err(error) => {
                        self.notice = Some(format!("Could not save default folder: {error}"))
                    }
                },
                KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
                    if let Some(entry) = self.entries.get(self.selected).cloned() {
                        match entry {
                            Entry::Directory(path) => {
                                self.directory = path;
                                self.level = Level::Directory;
                                self.selected = 0;
                                if let Err(error) = self.refresh() {
                                    self.notice = Some(error.to_string());
                                }
                            }
                            Entry::Series(name) => {
                                self.level = Level::Series(name);
                                self.selected = 0;
                                self.refresh()?;
                            }
                            Entry::Volume { path, .. } => {
                                self.level = Level::Volume(path);
                                self.selected = 0;
                                self.refresh()?;
                            }
                            Entry::LastRead { path, .. } => {
                                return Ok(Some(self.select_book(path, None)));
                            }
                            Entry::Chapter { path, page, .. } => {
                                return Ok(Some(self.select_book(path, Some(page))));
                            }
                            Entry::Read { path } => {
                                return Ok(Some(self.select_book(path, None)));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn select_book(&self, path: PathBuf, page: Option<usize>) -> Selection {
        Selection {
            path,
            page,
            location: Some(Location {
                directory: self.directory.clone(),
                level: self.level.clone(),
                selected: self.selected,
            }),
        }
    }

    fn parent(&mut self) -> Result<()> {
        match &self.level {
            Level::Volume(path) => {
                let path = path.clone();
                self.level = Level::Series(series_title(&path));
                self.refresh()?;
                if let Some(index) = self
                    .entries
                    .iter()
                    .position(|entry| entry.book() == Some(path.as_path()))
                {
                    self.selected = index;
                    self.request_preview();
                }
            }
            Level::Series(name) => {
                let name = name.clone();
                self.level = Level::Directory;
                self.refresh()?;
                if let Some(index) = self
                    .entries
                    .iter()
                    .position(|entry| matches!(entry, Entry::Series(title) if title == &name))
                {
                    self.selected = index;
                    self.request_preview();
                }
            }
            Level::Directory => {
                if let Some(parent) = self.directory.parent() {
                    let old = self.directory.clone();
                    self.directory = parent.to_path_buf();
                    self.refresh()?;
                    if let Some(index) = self
                        .entries
                        .iter()
                        .position(|entry| matches!(entry, Entry::Directory(path) if path == &old))
                    {
                        self.selected = index;
                        self.request_preview();
                    }
                }
            }
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(frame.area());
        frame.render_widget(
            Block::default().style(Style::default().bg(BG)),
            frame.area(),
        );
        let location = match &self.level {
            Level::Directory => self.directory.display().to_string(),
            Level::Series(name) => format!("{} / {name}", self.directory.display()),
            Level::Volume(path) => format!(
                "{} / {} / {}",
                self.directory.display(),
                series_title(path),
                volume_title(path)
            ),
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "  PANEL  ",
                    Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
                ),
                Span::styled(location, Style::default().fg(FG)),
            ]))
            .style(Style::default().bg(BG)),
            area[0],
        );
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(area[1]);
        let items: Vec<_> = self
            .entries
            .iter()
            .map(|entry| {
                let color = if matches!(entry, Entry::Directory(_) | Entry::Series(_)) {
                    ACCENT
                } else {
                    FG
                };
                ListItem::new(entry.label()).style(Style::default().fg(color))
            })
            .collect();
        let mut state =
            ListState::default().with_selected((!items.is_empty()).then_some(self.selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .title(" Library ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(MUTED)),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Rgb(80, 73, 69))
                        .fg(YELLOW)
                        .add_modifier(Modifier::BOLD),
                ),
            columns[0],
            &mut state,
        );
        self.draw_preview(frame, columns[1]);
        let hint = self
            .notice
            .as_deref()
            .unwrap_or(" j/k move  ·  l/Enter open  ·  h back  ·  d set default  ·  q quit");
        frame.render_widget(
            Paragraph::new(hint)
                .style(Style::default().fg(MUTED).bg(BG))
                .wrap(Wrap { trim: true }),
            area[2],
        );
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect) {
        if self.preview_target.is_none() {
            return;
        }
        let block = Block::default()
            .title(" Cover ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(MUTED));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let Some(preview) = self.preview.as_ref() else {
            frame.render_widget(
                Paragraph::new("Loading cover…").style(Style::default().fg(MUTED)),
                inner,
            );
            return;
        };
        let Ok((_, pages)) = &preview.result else {
            frame.render_widget(
                Paragraph::new("Could not decode cover").style(Style::default().fg(MUTED)),
                inner,
            );
            return;
        };
        frame.render_widget(
            Paragraph::new(format!("{pages} pages")).style(Style::default().fg(ACCENT)),
            Rect { height: 1, ..inner },
        );
        if let Some(image) = self.image.as_mut() {
            let image_area = Rect {
                x: inner.x,
                y: inner.y.saturating_add(2),
                width: inner.width,
                height: inner.height.saturating_sub(3),
            };
            if image_area.width > 0 && image_area.height > 0 {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Fit(None)),
                    image_area,
                    image,
                );
            }
        }
        if let Some(page) = self.progress {
            frame.render_widget(
                Paragraph::new(format!("Resume at page {page}/{pages}"))
                    .style(Style::default().fg(YELLOW)),
                Rect {
                    x: inner.x,
                    y: inner.y + inner.height.saturating_sub(1),
                    width: inner.width,
                    height: 1,
                },
            );
        }
    }
}

// Heuristics affect presentation only; filenames and files on disk are untouched.
fn volume_marker(name: &str) -> Option<(usize, u32)> {
    let bytes = name.as_bytes();
    for (index, _) in name.char_indices() {
        if index != 0 && !matches!(bytes[index - 1], b' ' | b'-' | b'_' | b'(') {
            continue;
        }
        let rest = &name[index..];
        for prefix in ["volume", "vol", "v"] {
            if !rest
                .get(..prefix.len())
                .is_some_and(|part| part.eq_ignore_ascii_case(prefix))
            {
                continue;
            }
            let suffix = rest[prefix.len()..].trim_start_matches([' ', '.', '_', '-']);
            let digits = suffix.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0
                || suffix
                    .as_bytes()
                    .get(digits)
                    .is_some_and(u8::is_ascii_alphabetic)
            {
                continue;
            }
            if let Ok(number) = suffix[..digits].parse() {
                return Some((index, number));
            }
        }
    }
    None
}

fn series_title(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let name = stem.as_ref();
    let marker = volume_marker(name).map(|(index, _)| index);
    let annotation = name.find(['(', '[']);
    let cutoff = marker
        .into_iter()
        .chain(annotation)
        .min()
        .unwrap_or(name.len());
    let title = name[..cutoff].trim().trim_end_matches(['-', '_']).trim();
    if title.is_empty() {
        name.to_owned()
    } else {
        title.to_owned()
    }
}

fn chapter_starts(book: &CbzDocument) -> BTreeMap<u32, usize> {
    let mut chapters = BTreeMap::new();
    for page in 0..book.page_count() {
        if let Some(number) = chapter_number(book.page_name(page)) {
            chapters.entry(number).or_insert(page);
        }
    }
    chapters
}

fn chapter_description(chapters: &BTreeMap<u32, usize>, page: usize) -> String {
    chapters
        .iter()
        .rev()
        .find(|(_, start)| **start <= page)
        .map(|(number, start)| format!("Chapter {number} · page {}", page - start + 1))
        .unwrap_or_else(|| format!("Page {}", page + 1))
}

fn latest_volume(volumes: &[PathBuf]) -> Option<Entry> {
    latest_volume_with_store(volumes, &ProgressStore::open().ok()?)
}

fn latest_volume_with_store(volumes: &[PathBuf], store: &ProgressStore) -> Option<Entry> {
    let (path, position) = store.most_recent(volumes).ok()??;
    let book = CbzDocument::open(&path).ok()?;
    if position.page >= book.page_count() {
        return None;
    }
    let description = format!(
        "{} · {}",
        volume_title(&path),
        chapter_description(&chapter_starts(&book), position.page)
    );
    Some(Entry::LastRead { path, description })
}

fn volume_title(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    match volume_marker(&stem) {
        Some((_, number)) => format!("Volume {number}"),
        None => "Volume (unspecified)".into(),
    }
}

pub(crate) fn later_volumes(book: &Path) -> Vec<PathBuf> {
    let parent = book
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let Some(current) = book
        .file_stem()
        .and_then(|stem| volume_marker(&stem.to_string_lossy()))
    else {
        return Vec::new();
    };
    let title = series_title(book);
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut later: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| is_cbz(path) && series_title(path).eq_ignore_ascii_case(&title))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_string_lossy();
            let (_, number) = volume_marker(&stem)?;
            (number > current.1).then_some((number, path))
        })
        .collect();
    later.sort_by(|(a_number, a_path), (b_number, b_path)| {
        a_number.cmp(b_number).then_with(|| {
            natord::compare_ignore_case(&a_path.to_string_lossy(), &b_path.to_string_lossy())
        })
    });
    later.into_iter().map(|(_, path)| path).collect()
}

fn chapter_number(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    for index in 0..bytes.len() {
        if !bytes[index].eq_ignore_ascii_case(&b'c')
            || (index != 0 && bytes[index - 1].is_ascii_alphanumeric())
        {
            continue;
        }
        let rest = &name[index..];
        let prefix = if rest
            .get(..7)
            .is_some_and(|part| part.eq_ignore_ascii_case("chapter"))
        {
            7
        } else if rest
            .get(..2)
            .is_some_and(|part| part.eq_ignore_ascii_case("ch"))
        {
            2
        } else {
            1
        };
        let suffix = &rest[prefix..];
        let suffix = if prefix == 1 {
            suffix
        } else {
            suffix.trim_start_matches([' ', '.', '_', '-', '#'])
        };
        let digits = suffix.bytes().take_while(u8::is_ascii_digit).count();
        if digits != 0
            && !suffix
                .as_bytes()
                .get(digits)
                .is_some_and(u8::is_ascii_alphabetic)
        {
            if let Ok(number) = suffix[..digits].parse() {
                return Some(number);
            }
        }
    }
    None
}

fn sidecar_cover(book: &Path) -> Option<PathBuf> {
    let volume = volume_marker(&book.file_stem()?.to_string_lossy())?.1;
    let covers = book.parent()?.join("Covers");
    let mut images: Vec<_> = fs::read_dir(covers)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ["jpg", "jpeg", "png", "webp"]
                            .iter()
                            .any(|supported| ext.eq_ignore_ascii_case(supported))
                    })
        })
        .collect();
    images.sort();
    images.into_iter().find(|path| {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let trailing = stem
            .rfind(['v', 'V'])
            .and_then(|index| stem[index + 1..].parse::<u32>().ok());
        volume_marker(&stem).map(|(_, number)| number).or(trailing) == Some(volume)
    })
}

fn is_sidecar_folder(path: &Path, books: &[PathBuf]) -> bool {
    if !path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("Covers"))
        || !books.iter().any(|book| sidecar_cover(book).is_some())
    {
        return false;
    }
    fs::read_dir(path).is_ok_and(|items| {
        items
            .flatten()
            .all(|item| item.file_type().is_ok_and(|kind| kind.is_file()) && !is_cbz(&item.path()))
    })
}

fn load_sidecar_cover(path: &Path) -> Result<DynamicImage> {
    anyhow::ensure!(fs::metadata(path)?.len() <= 8_000_000, "cover is too large");
    let (width, height) = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_dimensions()?;
    anyhow::ensure!(
        width > 0 && height > 0 && width <= 4000 && height <= 4000,
        "cover dimensions are too large"
    );
    Ok(image::open(path)?)
}

fn configuration_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("panel/default-directory"))
}

fn default_directory() -> Option<PathBuf> {
    let contents = fs::read(configuration_path()?).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(contents)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(contents).ok().map(PathBuf::from)
    }
}

fn set_default_directory(directory: &Path) -> Result<()> {
    let path = configuration_path().context("neither XDG_CONFIG_HOME nor HOME is set")?;
    fs::create_dir_all(path.parent().context("invalid configuration path")?)?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        fs::write(&path, directory.canonicalize()?.as_os_str().as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(
            &path,
            directory.canonicalize()?.to_string_lossy().as_bytes(),
        )?;
    }
    Ok(())
}

fn is_cbz(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cbz"))
}

#[cfg(test)]
mod tests {
    use super::{
        Entry, Level, Location, Selection, chapter_number, is_sidecar_folder, later_volumes,
        latest_volume_with_store, resume_location, series_title, sidecar_cover, volume_title,
    };
    use crate::{
        document::CbzDocument,
        progress::{Position, ProgressStore},
    };
    use std::{collections::BTreeSet, env, fs, path::Path, time::SystemTime};
    use zip::write::SimpleFileOptions;

    #[test]
    fn returning_from_reader_keeps_the_original_selector_position() {
        let directory = env::temp_dir();
        let selection = Selection {
            path: directory.join("Climber - Volume_02.cbz"),
            page: Some(3),
            location: Some(Location {
                directory: directory.clone(),
                level: Level::Series("Climber".into()),
                selected: 4,
            }),
        };
        let (resumed, level, selected) = resume_location(Some(&selection), Path::new("/").into());
        assert_eq!(resumed, directory);
        assert!(matches!(level, Level::Series(name) if name == "Climber"));
        assert_eq!(selected, 4);
        let direct = Selection::direct(selection.path.clone());
        let (resumed, level, _) = resume_location(Some(&direct), Path::new("/").into());
        assert_eq!(resumed, directory);
        assert!(matches!(level, Level::Volume(path) if path == selection.path));
    }

    #[test]
    fn series_last_read_opens_the_latest_volume_at_its_saved_chapter() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            env::temp_dir().join(format!("panel-series-last-{}-{unique}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let books: Vec<_> = (1..=2)
            .map(|volume| directory.join(format!("Climber - Volume_{volume:02}.cbz")))
            .collect();
        for book in &books {
            let mut archive = zip::ZipWriter::new(fs::File::create(book).unwrap());
            for (chapter, page) in [(101, 0), (102, 1), (102, 2)] {
                archive
                    .start_file(
                        format!("Chapter {chapter}/{page:03}.png"),
                        SimpleFileOptions::default(),
                    )
                    .unwrap();
                use std::io::Write;
                archive.write_all(b"test page").unwrap();
            }
            archive.finish().unwrap();
        }
        let store = ProgressStore::in_memory().unwrap();
        store
            .save(&books[0], Position { page: 0, region: 0 }, false)
            .unwrap();
        store
            .save(&books[1], Position { page: 2, region: 1 }, false)
            .unwrap();
        let older = books[0]
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        store
            .connection()
            .execute(
                "UPDATE progress SET updated_at = 1 WHERE path = ?1",
                [older],
            )
            .unwrap();
        match latest_volume_with_store(&books, &store).unwrap() {
            Entry::LastRead { path, description } => {
                assert_eq!(path, books[1]);
                assert_eq!(description, "Volume 2 · Chapter 102 · page 2");
            }
            _ => panic!("expected a last-read volume"),
        }
        store
            .save(&books[0], Position { page: 0, region: 0 }, false)
            .unwrap();
        assert!(matches!(latest_volume_with_store(&books, &store),
            Some(Entry::LastRead { path, .. }) if path == books[0]));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn groups_single_volume() {
        let book = Path::new("Berserk v41 (2022) (Digital).cbz");
        assert_eq!(series_title(book), "Berserk");
        assert_eq!(volume_title(book), "Volume 41");
        assert_eq!(chapter_number("Berserk - c358 (v41) - p000.jpg"), Some(358));
        assert_eq!(chapter_number("page-001.jpg"), None);
    }

    #[test]
    fn groups_numbered_volumes_and_chapter_directories_without_moving_files() {
        for number in 1..=17 {
            let book = format!("Kokou no Hito - Volume_{number:02}.cbz");
            let path = Path::new(&book);
            assert_eq!(series_title(path), "Kokou no Hito");
            assert_eq!(volume_title(path), format!("Volume {number}"));
        }
        assert_eq!(chapter_number("Vol.1 Ch.1 - First/000.jpg"), Some(1));
        assert_eq!(chapter_number("Vol.10 Ch.093 - Snow/001.png"), Some(93));
        assert_eq!(chapter_number("Vol.17 Ch.170 - End/033.jpg"), Some(170));
        assert_eq!(chapter_number("Chapter 42/page_0001.webp"), Some(42));
        assert_eq!(chapter_number("book/chapter-cover.jpg"), None);
        assert_eq!(chapter_number("book/scanned-page-001.jpg"), None);
    }

    #[test]
    fn associates_sidecar_covers_by_volume_without_confusing_one_and_ten() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root =
            env::temp_dir().join(format!("panel-cover-test-{}-{unique}", std::process::id()));
        let covers = root.join("Covers");
        fs::create_dir_all(&covers).expect("create temporary covers");
        let one = covers.join("1422v1.jpg");
        let ten = covers.join("1422v10.jpg");
        fs::write(&one, []).expect("create mock cover one");
        fs::write(&ten, []).expect("create mock cover ten");
        assert_eq!(
            sidecar_cover(&root.join("Kokou no Hito - Volume_01.cbz")),
            Some(one)
        );
        assert_eq!(
            sidecar_cover(&root.join("Kokou no Hito - Volume_10.cbz")),
            Some(ten)
        );
        assert_eq!(
            sidecar_cover(&root.join("Kokou no Hito - Volume_02.cbz")),
            None
        );
        let volumes = [root.join("Kokou no Hito - Volume_01.cbz")];
        assert!(is_sidecar_folder(&covers, &volumes));
        fs::write(covers.join("unexpected.cbz"), []).expect("create book in covers");
        assert!(!is_sidecar_folder(&covers, &volumes));
        fs::remove_dir_all(root).expect("remove temporary covers");
    }

    #[test]
    fn later_volumes_only_follow_the_same_series_in_numeric_order() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = env::temp_dir().join(format!("panel-volumes-{}-{unique}", std::process::id()));
        fs::create_dir(&root).expect("create volumes directory");
        for name in [
            "Climber - Volume_01.cbz",
            "Climber - Volume_02.cbz",
            "Climber - Volume_10.cbz",
            "Climber - Volume_17.cbz",
            "Other - Volume_03.cbz",
        ] {
            fs::write(root.join(name), []).expect("create mock archive");
        }
        assert_eq!(
            later_volumes(&root.join("Climber - Volume_02.cbz")),
            [
                root.join("Climber - Volume_10.cbz"),
                root.join("Climber - Volume_17.cbz")
            ]
        );
        fs::remove_dir_all(root).expect("remove volumes directory");
    }

    #[test]
    fn optional_local_collection_has_all_chapters() {
        let Some(directory) = env::var_os("PANEL_TEST_KOKOU_DIRECTORY") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let mut chapters = BTreeSet::new();
        let mut volumes = BTreeSet::new();
        let mut books = Vec::new();
        for entry in fs::read_dir(&directory).expect("list local collection") {
            let path = entry.expect("read local entry").path();
            if !super::is_cbz(&path) {
                continue;
            }
            assert_eq!(series_title(&path), "Kokou no Hito");
            let volume = super::volume_marker(&path.file_stem().unwrap().to_string_lossy())
                .expect("volume label")
                .1;
            assert!(volumes.insert(volume), "volume appears twice");
            let cover = sidecar_cover(&path).expect("missing volume cover");
            super::load_sidecar_cover(&cover).expect("decode volume cover");
            let book = CbzDocument::open(&path).expect("read volume");
            books.push(path);
            for page in 0..book.page_count() {
                if let Some(chapter) = chapter_number(book.page_name(page)) {
                    chapters.insert(chapter);
                }
            }
        }
        assert_eq!(volumes, (1..=17).collect());
        assert_eq!(chapters, (1..=170).collect());
        assert!(is_sidecar_folder(&directory.join("Covers"), &books));
        assert_eq!(
            later_volumes(&directory.join("Kokou no Hito - Volume_01.cbz")).len(),
            16
        );
        assert!(later_volumes(&directory.join("Kokou no Hito - Volume_17.cbz")).is_empty());
    }
}
