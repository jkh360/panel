//! Keyboard-first connector browser with explicit local chapter CBZ downloads.
use std::{path::Path, time::Duration};

use anyhow::Result;
use crossterm::event::{self, Event as KeyEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::hn_client::{Client, Event, Failure, Item, Request};

const BG: Color = Color::Rgb(40, 40, 40);
const FG: Color = Color::Rgb(235, 219, 178);
const MUTED: Color = Color::Rgb(168, 153, 132);
const YELLOW: Color = Color::Rgb(250, 189, 47);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Sources,
    Query,
    Titles,
    Chapters,
}

struct Browser {
    screen: Screen,
    selected: usize,
    sources: Vec<Item>,
    filtered_sources: Vec<Item>,
    source_filter: String,
    filtering: bool,
    titles: Vec<Item>,
    chapters: Vec<Item>,
    source: Option<Item>,
    title: Option<Item>,
    query: String,
    loading: bool,
    notice: String,
}

impl Browser {
    fn new() -> Self {
        Self {
            screen: Screen::Sources,
            selected: 0,
            sources: Vec::new(),
            filtered_sources: Vec::new(),
            source_filter: String::new(),
            filtering: false,
            titles: Vec::new(),
            chapters: Vec::new(),
            source: None,
            title: None,
            query: String::new(),
            loading: true,
            notice: "Loading connector catalog…".into(),
        }
    }

    fn items(&self) -> &[Item] {
        match self.screen {
            Screen::Sources => &self.filtered_sources,
            Screen::Query => &[],
            Screen::Titles => &self.titles,
            Screen::Chapters => &self.chapters,
        }
    }

    fn update_source_filter(&mut self) {
        let needle = self.source_filter.to_lowercase();
        self.filtered_sources = self
            .sources
            .iter()
            .filter(|source| {
                source.title.to_lowercase().contains(&needle)
                    || source.id.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        self.selected = 0;
    }

    fn back(&mut self) -> bool {
        self.loading = false;
        self.selected = 0;
        match self.screen {
            Screen::Sources => return false,
            Screen::Query => self.screen = Screen::Sources,
            Screen::Titles => self.screen = Screen::Query,
            Screen::Chapters => self.screen = Screen::Titles,
        }
        self.notice.clear();
        true
    }

    fn request(&mut self, client: &Client, request: Request) {
        self.loading = true;
        self.notice = "Waiting for the isolated backend…".into();
        if client.requests.send(request).is_err() {
            self.loading = false;
            self.notice = "Backend unavailable; see hn/SECURITY.md".into();
        }
    }

    fn accept(&mut self, event: Event) {
        if let Event::DownloadProgress { done, total } = event {
            self.notice = format!("Saving page {done}/{total}… Esc cancels");
            return;
        }
        if let Event::BatchProgress {
            done,
            total,
            saved,
            skipped,
        } = event
        {
            self.notice =
                format!("Chapters {done}/{total}: saved {saved}, skipped {skipped} · Esc cancels");
            return;
        }
        self.loading = false;
        match event {
            Event::DownloadProgress { .. } | Event::BatchProgress { .. } => unreachable!(),
            Event::BatchFinished { saved, skipped } => {
                self.notice =
                    format!("Batch finished: {saved} CBZs saved, {skipped} skipped or failed");
            }
            Event::Downloaded(path) => {
                self.notice = format!("Saved: {}", path.display());
            }
            Event::Failed(Failure::Download) => {
                self.notice =
                    "Download failed (site, image, size limit or disk); reopen Search to retry"
                        .into();
            }
            Event::Failed(Failure::Unavailable) => {
                self.notice = "Online backend unavailable / paused (hn/SECURITY.md)".into();
            }
            Event::Failed(Failure::Connector) => {
                self.notice = "Connector failed (site may require login or have changed)".into();
            }
            Event::Loaded(items) => {
                self.selected = 0;
                self.notice = if items.is_empty() {
                    "No results from this connector".into()
                } else {
                    String::new()
                };
                match self.screen {
                    Screen::Sources => {
                        self.sources = items;
                        self.update_source_filter();
                    }
                    Screen::Titles => self.titles = items,
                    Screen::Chapters => self.chapters = items,
                    Screen::Query => {} // Stale replies cannot select a manga.
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(Style::default().bg(BG)), area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(area);
        let heading = match self.screen {
            Screen::Sources => "Search / sources (loaded ≠ working)",
            Screen::Query => "Search / title query",
            Screen::Titles => "Search / matching titles",
            Screen::Chapters => "Search / chapters (read-only)",
        };
        frame.render_widget(
            Paragraph::new(format!("  PANEL  {heading}")).style(
                Style::default()
                    .fg(YELLOW)
                    .bg(BG)
                    .add_modifier(Modifier::BOLD),
            ),
            rows[0],
        );
        if self.screen == Screen::Sources && self.filtering {
            frame.render_widget(
                Paragraph::new(format!(
                    "Filter sources: {}▌\nEnter: select · Esc: clear",
                    self.source_filter
                ))
                .block(
                    Block::default()
                        .title(" Source filter ")
                        .borders(Borders::ALL),
                )
                .style(Style::default().fg(FG).bg(BG)),
                rows[1],
            );
        } else if self.screen == Screen::Query {
            frame.render_widget(
                Paragraph::new(format!(
                    "Source: {}\n\nSearch: {}▌\n\nEnter: search · Esc: sources",
                    self.source.as_ref().map_or("", |item| item.title.as_str()),
                    self.query
                ))
                .block(Block::default().title(" Title ").borders(Borders::ALL))
                .style(Style::default().fg(FG).bg(BG))
                .wrap(Wrap { trim: true }),
                rows[1],
            );
        } else {
            let items: Vec<_> = self
                .items()
                .iter()
                .map(|item| ListItem::new(item.title.as_str()))
                .collect();
            let mut selection =
                ListState::default().with_selected((!items.is_empty()).then_some(self.selected));
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::default().title(" Results ").borders(Borders::ALL))
                    .style(Style::default().fg(FG).bg(BG))
                    .highlight_style(Style::default().bg(Color::Rgb(80, 73, 69)).fg(YELLOW)),
                rows[1],
                &mut selection,
            );
        }
        let hint = if self.notice.is_empty() {
            match self.screen {
                Screen::Chapters => "d save chapter · D save all chapters · Esc back · q library",
                Screen::Query => "Type a title · Enter search · Esc back",
                Screen::Sources => "f or / filter sources · j/k select · Enter open · q library",
                _ => "j/k select · Enter open · Esc back · q library",
            }
        } else {
            &self.notice
        };
        frame.render_widget(
            Paragraph::new(hint)
                .style(Style::default().fg(MUTED).bg(BG))
                .wrap(Wrap { trim: true }),
            rows[2],
        );
    }
}

pub fn browse(terminal: &mut DefaultTerminal, directory: &Path) -> Result<()> {
    let client = Client::start();
    let mut state = Browser::new();
    state.request(&client, Request::Sources);
    let mut redraw = true;
    loop {
        while let Ok(event) = client.events.try_recv() {
            state.accept(event);
            redraw = true;
        }
        if redraw {
            terminal.draw(|frame| state.draw(frame))?;
            redraw = false;
        }
        if !event::poll(Duration::from_millis(80))? {
            continue;
        }
        let KeyEvent::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        redraw = true;
        if state.loading {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                break;
            }
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            break;
        }
        if state.screen == Screen::Sources && state.filtering {
            match key.code {
                KeyCode::Esc => {
                    state.filtering = false;
                    state.source_filter.clear();
                    state.update_source_filter();
                }
                KeyCode::Enter => state.filtering = false,
                KeyCode::Backspace => {
                    state.source_filter.pop();
                    state.update_source_filter();
                }
                KeyCode::Char(c) if !c.is_control() && state.source_filter.chars().count() < 80 => {
                    state.source_filter.push(c);
                    state.update_source_filter();
                }
                _ => {}
            }
            continue;
        }
        if state.screen == Screen::Sources && matches!(key.code, KeyCode::Char('f' | '/')) {
            state.filtering = true;
            continue;
        }
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('h') && state.screen != Screen::Query)
        {
            if !state.back() {
                break;
            }
            continue;
        }
        if key.code == KeyCode::Char('q') && state.screen != Screen::Query {
            break;
        }
        if state.screen == Screen::Query {
            match key.code {
                KeyCode::Char(c)
                    if !c.is_control()
                        && state.query.chars().count() < 120
                        && state.query.len() + c.len_utf8() <= 480 =>
                {
                    state.query.push(c)
                }
                KeyCode::Backspace => {
                    state.query.pop();
                }
                KeyCode::Enter if !state.loading => {
                    if let Some(source) = state.source.as_ref() {
                        if state.query.chars().count() >= 2 {
                            let source = source.id.clone();
                            let query = state.query.clone();
                            state.screen = Screen::Titles;
                            state.request(&client, Request::Search { source, query });
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Char('D') if state.screen == Screen::Chapters => {
                if let (Some(source), Some(manga)) = (state.source.clone(), state.title.clone()) {
                    state.request(
                        &client,
                        Request::DownloadAll {
                            source,
                            manga,
                            chapters: state.chapters.clone(),
                            directory: directory.to_path_buf(),
                        },
                    );
                }
            }
            KeyCode::Char('d') if state.screen == Screen::Chapters => {
                if let (Some(source), Some(manga), Some(chapter)) = (
                    state.source.clone(),
                    state.title.clone(),
                    state.items().get(state.selected).cloned(),
                ) {
                    state.request(
                        &client,
                        Request::Download {
                            source,
                            manga,
                            chapter,
                            directory: directory.to_path_buf(),
                        },
                    );
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                state.selected = (state.selected + 1).min(state.items().len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => state.selected = state.selected.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right if !state.loading => {
                let Some(item) = state.items().get(state.selected).cloned() else {
                    continue;
                };
                match state.screen {
                    Screen::Sources => {
                        state.source = Some(item);
                        state.screen = Screen::Query;
                        state.selected = 0;
                        state.notice.clear();
                    }
                    Screen::Titles => {
                        state.title = Some(item.clone());
                        state.screen = Screen::Chapters;
                        if let Some(source) = &state.source {
                            state.request(
                                &client,
                                Request::Chapters {
                                    source: source.id.clone(),
                                    id: item.id,
                                    title: item.title,
                                },
                            );
                        }
                    }
                    Screen::Chapters | Screen::Query => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Browser, Screen};
    use crate::hn_client::{Event, Failure, Item};

    #[test]
    fn metadata_routes_to_its_own_screen_and_download_progress_stays_loading() {
        let mut browser = Browser::new();
        browser.accept(Event::Loaded(vec![Item {
            id: "source".into(),
            title: "Source".into(),
        }]));
        assert_eq!(browser.sources.len(), 1);
        browser.screen = Screen::Titles;
        browser.accept(Event::Loaded(vec![Item {
            id: "book".into(),
            title: "Manga".into(),
        }]));
        assert_eq!(browser.titles[0].id, "book");
        browser.screen = Screen::Chapters;
        browser.accept(Event::Loaded(vec![Item {
            id: "chapter".into(),
            title: "Chapter 1".into(),
        }]));
        assert_eq!(browser.chapters[0].title, "Chapter 1");
        assert_eq!(browser.sources.len(), 1);
        browser.loading = true;
        browser.accept(Event::DownloadProgress { done: 2, total: 3 });
        assert!(browser.loading);
        assert!(browser.notice.contains("2/3"));
        browser.accept(Event::Failed(Failure::Connector));
        assert!(browser.notice.contains("may require login"));
    }
    #[test]
    fn source_filter_matches_title_or_connector_id() {
        let mut browser = Browser::new();
        browser.accept(Event::Loaded(vec![
            Item {
                id: "shueishamangaplus".into(),
                title: "MANGA Plus by Shueisha".into(),
            },
            Item {
                id: "other".into(),
                title: "Another source".into(),
            },
        ]));
        browser.source_filter = "shueisha".into();
        browser.update_source_filter();
        assert_eq!(browser.items().len(), 1);
        assert_eq!(browser.items()[0].id, "shueishamangaplus");
        browser.source_filter = "other".into();
        browser.update_source_filter();
        assert_eq!(browser.items()[0].id, "other");
    }

    #[test]
    fn back_navigation_returns_to_titles() {
        let mut browser = Browser::new();
        browser.screen = Screen::Chapters;
        assert!(browser.back());
        assert_eq!(browser.screen, Screen::Titles);
        assert!(browser.back());
        assert_eq!(browser.screen, Screen::Query);
        assert!(browser.back());
        assert_eq!(browser.screen, Screen::Sources);
        assert!(!browser.back());
    }
}
