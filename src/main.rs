mod detection_cache;
mod detector;
mod document;
mod import_regions;
mod library;
mod page_view;
mod progress;
mod region;

use std::{
    cell::{Cell, RefCell},
    env,
    path::PathBuf,
    process,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Result, bail};
use detection_cache::CachedDetection;
use document::CbzDocument;
use gtk::{
    Application, ApplicationWindow, CssProvider, EventControllerKey, Label, Orientation,
    STYLE_PROVIDER_PRIORITY_APPLICATION, gdk, glib, prelude::*,
};
use image::RgbaImage;
use page_view::{PageView, ViewMode};
use progress::{Position, ProgressStore};
use region::Region;

const APPLICATION_ID: &str = "dev.panel.Reader";
const GRUVBOX_DARK: &str = "#282828";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReaderExit {
    Quit,
    Library,
}

fn exit_key(key: gdk::Key) -> Option<ReaderExit> {
    match key {
        gdk::Key::Escape => Some(ReaderExit::Library),
        gdk::Key::q => Some(ReaderExit::Quit),
        _ => None,
    }
}

enum WorkerRequest {
    Load { page: usize, generation: u64 },
    Stop,
}

struct LoadedPage {
    number: usize,
    generation: u64,
    image: RgbaImage,
    regions: Vec<Region>,
    source: String,
    fallback_reason: Option<String>,
}

struct Page {
    texture: gdk::Texture,
    regions: Vec<Region>,
    source: String,
    fallback_reason: Option<String>,
}

struct Reader {
    progress: ProgressStore,
    path: PathBuf,
    title: String,
    page_count: usize,
    position: Position,
    page: Page,
}

fn main() {
    if env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("import-regions")) {
        let Some(book) = env::args_os().nth(2) else {
            eprintln!("panel: usage: panel import-regions BOOK.cbz REGIONS.json");
            process::exit(2);
        };
        let Some(predictions) = env::args_os().nth(3) else {
            eprintln!("panel: usage: panel import-regions BOOK.cbz REGIONS.json");
            process::exit(2);
        };
        if let Err(error) = import_regions::run(
            std::path::Path::new(&book),
            std::path::Path::new(&predictions),
        ) {
            eprintln!("panel: {error:#}");
            process::exit(1);
        }
        return;
    }
    if env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("analyze")) {
        let Some(path) = env::args_os().nth(2) else {
            eprintln!("panel: usage: panel analyze BOOK.cbz");
            process::exit(2);
        };
        if let Err(error) = analyze_cli(&PathBuf::from(path)) {
            eprintln!("panel: {error:#}");
            process::exit(1);
        }
        return;
    }
    let mut selection = match input_path() {
        Ok(Some(selection)) => selection,
        Ok(None) => return,
        Err(error) => {
            eprintln!("panel: {error:#}");
            process::exit(2);
        }
    };
    loop {
        let exit = Rc::new(Cell::new(ReaderExit::Quit));
        let requested_exit = Rc::clone(&exit);
        let application = Application::builder()
            .application_id(APPLICATION_ID)
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        let path = selection.path.clone();
        let page = selection.page;
        application.connect_activate(move |application| {
            if let Err(error) =
                build_reader(application, path.clone(), page, Rc::clone(&requested_exit))
            {
                eprintln!("panel: {error:#}");
                application.quit();
            }
        });
        application.run_with_args::<&str>(&[]);
        if exit.get() == ReaderExit::Quit {
            break;
        }
        selection = match library::select(Some(&selection)) {
            Ok(Some(selection)) => selection,
            Ok(None) => break,
            Err(error) => {
                eprintln!("panel: {error:#}");
                process::exit(2);
            }
        };
    }
}

fn analyze_cli(path: &std::path::Path) -> Result<()> {
    use std::time::Instant;

    let mut document = CbzDocument::open(path)?;
    let cache = ProgressStore::open()?;
    println!(
        "{{\"document\":\"{}\",\"detector_version\":\"{}\",\"pages\":[",
        json_escape(&path.to_string_lossy()),
        detection_cache::DETECTOR_VERSION
    );
    for page in 0..document.page_count() {
        let started = Instant::now();
        let image = document.load_page(page)?;
        let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
        let detection_started = Instant::now();
        let detection = load_or_detect(cache.connection(), path, page, &image);
        let detection_ms = detection_started.elapsed().as_secs_f64() * 1000.0;
        let page_area = u64::from(image.width()) * u64::from(image.height());
        let covered = detection.regions.iter().map(|r| r.area()).sum::<u64>();
        let overlap = total_pair_overlap(&detection.regions);
        let coverage = covered as f64 / page_area as f64;
        let overlap_fraction = overlap as f64 / page_area as f64;
        let suspicious = detection.source == "whole-page"
            || detection.regions.len() > 8
            || !(0.55..=1.08).contains(&coverage)
            || overlap_fraction > 0.08;
        if page != 0 {
            println!(",");
        }
        print!(
            "{{\"page\":{},\"name\":\"{}\",\"width\":{},\"height\":{},\"source\":\"{}\",\"cached\":{},\"fallback_reason\":{},\"region_count\":{},\"coverage\":{:.4},\"overlap\":{:.4},\"suspicious\":{},\"decode_ms\":{:.2},\"detection_ms\":{:.2},\"regions\":[",
            page,
            json_escape(document.page_name(page)),
            image.width(),
            image.height(),
            json_escape(&detection.source),
            detection.cached,
            detection.fallback_reason.as_ref().map_or_else(
                || "null".to_owned(),
                |reason| format!("\"{}\"", json_escape(reason))
            ),
            detection.regions.len(),
            coverage,
            overlap_fraction,
            suspicious,
            decode_ms,
            detection_ms,
        );
        for (index, region) in detection.regions.iter().enumerate() {
            if index != 0 {
                print!(",");
            }
            print!(
                "[{},{},{},{}]",
                region.x, region.y, region.width, region.height
            );
        }
        print!("]}}");
    }
    println!("\n]}}");
    Ok(())
}

fn total_pair_overlap(regions: &[Region]) -> u64 {
    let mut total = 0;
    for (index, a) in regions.iter().enumerate() {
        for b in &regions[index + 1..] {
            let width = a.right().min(b.right()).saturating_sub(a.x.max(b.x));
            let height = a.bottom().min(b.bottom()).saturating_sub(a.y.max(b.y));
            total += u64::from(width) * u64::from(height);
        }
    }
    total
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn input_path() -> Result<Option<library::Selection>> {
    let mut arguments = env::args_os().skip(1);
    let Some(path) = arguments.next() else {
        return library::select(None);
    };
    if arguments.next().is_some() {
        bail!("expected exactly one document path");
    }
    let path = PathBuf::from(path);
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("cbz"))
    {
        bail!("the first build supports CBZ input only");
    }
    Ok(Some(library::Selection::direct(path)))
}

fn build_reader(
    application: &Application,
    path: PathBuf,
    initial_page: Option<usize>,
    exit: Rc<Cell<ReaderExit>>,
) -> Result<()> {
    install_css();
    let mut document = CbzDocument::open(&path)?;
    let page_count = document.page_count();
    let title = document.title();
    let progress = ProgressStore::open()?;
    let restored = progress.load(&path)?.unwrap_or_default();
    let selected_page = initial_page.unwrap_or(restored.page).min(page_count - 1);
    let image = document.load_page(selected_page)?;
    let detection = load_or_detect(progress.connection(), &path, selected_page, &image);
    let position = Position {
        page: selected_page,
        region: restored_region(restored, initial_page, detection.regions.len()),
    };
    let page = make_page(image, detection);
    let reader = Rc::new(RefCell::new(Reader {
        progress,
        path: path.clone(),
        title: title.clone(),
        page_count,
        position,
        page,
    }));

    let (requests, worker_requests) = mpsc::sync_channel(1);
    let (worker_results, results) = mpsc::channel();
    let worker_path = path.clone();
    let worker = thread::spawn(move || {
        let cache = ProgressStore::open().expect("failed to open worker detection cache");
        while let Ok(request) = worker_requests.recv() {
            match request {
                WorkerRequest::Load { page, generation } => {
                    let result = document
                        .load_page(page)
                        .map(|image| {
                            let detection =
                                load_or_detect(cache.connection(), &worker_path, page, &image);
                            LoadedPage {
                                number: page,
                                generation,
                                image,
                                regions: detection.regions,
                                source: detection.source,
                                fallback_reason: detection.fallback_reason,
                            }
                        })
                        .map_err(|error| error.to_string());
                    if worker_results.send(result).is_err() {
                        break;
                    }
                }
                WorkerRequest::Stop => break,
            }
        }
    });
    let requests = Rc::new(requests);
    let worker = Rc::new(RefCell::new(Some(worker)));
    let (detector_requests, external_requests) = mpsc::sync_channel(8);
    let (external_results, detector_results) = mpsc::sync_channel(8);
    let detector_cancel = Arc::new(AtomicBool::new(false));
    let detector_worker = detector::configured().then(|| {
        detector::spawn(
            path.clone(),
            position.page,
            external_requests,
            external_results,
            Arc::clone(&detector_cancel),
        )
    });
    let detector_worker = Rc::new(RefCell::new(detector_worker));
    let detector_requests = Rc::new(detector_requests);
    let generation = Rc::new(Cell::new(0_u64));
    let pending = Rc::new(Cell::new(false));
    let alive = Rc::new(Cell::new(true));

    let view = PageView::new();
    view.set_zoom(reader.borrow().progress.load_zoom(&path)?);
    let zoom_indicator = Label::new(None);
    zoom_indicator.set_halign(gtk::Align::Start);
    zoom_indicator.set_valign(gtk::Align::Start);
    zoom_indicator.set_margin_start(16);
    zoom_indicator.set_margin_top(16);
    zoom_indicator.set_can_target(false);
    zoom_indicator.add_css_class("zoom-indicator");
    zoom_indicator.set_visible(false);
    let zoom_timeout = Rc::new(Cell::new(0_u64));
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view));
    overlay.add_overlay(&zoom_indicator);
    let status = Label::new(None);
    status.set_halign(gtk::Align::Center);
    status.set_margin_top(4);
    status.set_margin_bottom(4);
    status.add_css_class("status");
    let layout = gtk::Box::new(Orientation::Vertical, 0);
    layout.add_css_class("reader");
    layout.append(&overlay);
    layout.append(&status);
    let window = ApplicationWindow::builder()
        .application(application)
        .title(&title)
        .default_width(1100)
        .default_height(800)
        .child(&layout)
        .build();
    present(&reader.borrow(), &view, &status, false);
    {
        let reader = reader.borrow();
        if let Err(error) = reader.progress.save(&reader.path, reader.position, false) {
            eprintln!("panel: failed to save opening position: {error:#}");
        }
    }

    {
        let reader = Rc::clone(&reader);
        let view = view.clone();
        let status = status.clone();
        let generation = Rc::clone(&generation);
        let pending = Rc::clone(&pending);
        let alive = Rc::clone(&alive);
        glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
            if !alive.get() {
                return glib::ControlFlow::Break;
            }
            while let Ok(result) = results.try_recv() {
                pending.set(false);
                match result {
                    Ok(loaded) if loaded.generation == generation.get() => {
                        let mut reader = reader.borrow_mut();
                        reader.position = Position {
                            page: loaded.number,
                            region: 0,
                        };
                        let mut detection = DetectionResult {
                            regions: loaded.regions,
                            source: loaded.source,
                            fallback_reason: loaded.fallback_reason,
                            cached: true,
                        };
                        if let Ok(Some(external)) = detection_cache::load_external(
                            reader.progress.connection(),
                            &reader.path,
                            loaded.number,
                            loaded.image.width(),
                            loaded.image.height(),
                        ) {
                            apply_external(&mut detection, external);
                        }
                        reader.page = make_page(loaded.image, detection);
                        present(&reader, &view, &status, false);
                        let completed = reader.position.page + 1 == reader.page_count;
                        if let Err(error) =
                            reader
                                .progress
                                .save(&reader.path, reader.position, completed)
                        {
                            eprintln!("panel: {error:#}");
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("panel: {error}");
                        update_status(&reader.borrow(), &status, view.mode(), false);
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    {
        let reader = Rc::clone(&reader);
        let view = view.clone();
        let status = status.clone();
        let alive = Rc::clone(&alive);
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !alive.get() {
                return glib::ControlFlow::Break;
            }
            while let Ok(result) = detector_results.try_recv() {
                match result {
                    Ok(result) => {
                        eprintln!(
                            "panel: detector {} page {} completed in {:.2}s",
                            result
                                .book
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            result.page + 1,
                            result.seconds.unwrap_or_default()
                        );
                        let mut reader = reader.borrow_mut();
                        if result.book != reader.path {
                            continue;
                        }
                        if reader.position.page == result.page {
                            let mut regions = result.regions;
                            region::order_rtl(&mut regions);
                            reader.page = Page {
                                texture: reader.page.texture.clone(),
                                regions,
                                source: result.source,
                                fallback_reason: None,
                            };
                            reader.position.region =
                                reader.position.region.min(reader.page.regions.len() - 1);
                            present(&reader, &view, &status, false);
                        } else {
                            update_status(&reader, &status, view.mode(), false);
                        }
                    }
                    Err(error) => eprintln!("panel: detector worker: {error}"),
                }
            }
            glib::ControlFlow::Continue
        });
    }

    let keys = EventControllerKey::new();
    {
        let reader = Rc::clone(&reader);
        let view = view.clone();
        let status = status.clone();
        let requests = Rc::clone(&requests);
        let generation = Rc::clone(&generation);
        let pending = Rc::clone(&pending);
        let detector_requests = Rc::clone(&detector_requests);
        let zoom_indicator = zoom_indicator.clone();
        let zoom_timeout = Rc::clone(&zoom_timeout);
        let weak_window = window.downgrade();
        let exit = Rc::clone(&exit);
        keys.connect_key_pressed(move |_, key, _, _| {
            if let Some(action) = exit_key(key) {
                exit.set(action);
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                }
                return glib::Propagation::Stop;
            }
            let zoom_step = match key {
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => 1,
                gdk::Key::minus | gdk::Key::KP_Subtract => -1,
                _ => 0,
            };
            if zoom_step != 0 {
                let zoom = view.change_zoom(zoom_step);
                let reader = reader.borrow();
                if let Err(error) = reader.progress.save_zoom(&reader.path, zoom) {
                    eprintln!("panel: failed to save zoom: {error:#}");
                }
                let displayed = if view.mode() == ViewMode::Focus {
                    zoom * 2.0
                } else {
                    zoom
                };
                zoom_indicator.set_text(&format!("Zoom {displayed:.1}×"));
                zoom_indicator.set_visible(true);
                let generation = zoom_timeout.get().wrapping_add(1);
                zoom_timeout.set(generation);
                let zoom_indicator = zoom_indicator.clone();
                let zoom_timeout = Rc::clone(&zoom_timeout);
                glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                    if zoom_timeout.get() == generation {
                        zoom_indicator.set_visible(false);
                    }
                });
                return glib::Propagation::Stop;
            }
            let mode = match key {
                gdk::Key::f => Some(ViewMode::Focus),
                gdk::Key::c => Some(ViewMode::Crop),
                gdk::Key::w => Some(ViewMode::Page),
                gdk::Key::Tab => Some(match view.mode() {
                    ViewMode::Focus => ViewMode::Crop,
                    ViewMode::Crop => ViewMode::Page,
                    ViewMode::Page => ViewMode::Focus,
                }),
                _ => None,
            };
            if let Some(mode) = mode {
                view.set_mode(mode);
                update_status(&reader.borrow(), &status, mode, pending.get());
                return glib::Propagation::Stop;
            }
            if pending.get() {
                return glib::Propagation::Stop;
            }

            let delta = match key {
                gdk::Key::j | gdk::Key::h => 1,
                gdk::Key::k | gdk::Key::l => -1,
                _ => return glib::Propagation::Proceed,
            };
            let region_key =
                view.mode() != ViewMode::Page && matches!(key, gdk::Key::j | gdk::Key::k);
            let mut reader = reader.borrow_mut();
            if region_key && move_region(&mut reader, delta) {
                present(&reader, &view, &status, false);
                let _ = reader.progress.save(&reader.path, reader.position, false);
            } else {
                let target = (reader.position.page as isize + delta)
                    .clamp(0, reader.page_count as isize - 1) as usize;
                if detector::configured() {
                    let _ =
                        detector_requests.try_send(detector::CommandMessage::Prioritize(target));
                }
                if target != reader.position.page {
                    let next_generation = generation.get() + 1;
                    if requests
                        .try_send(WorkerRequest::Load {
                            page: target,
                            generation: next_generation,
                        })
                        .is_ok()
                    {
                        generation.set(next_generation);
                        pending.set(true);
                        update_status(&reader, &status, view.mode(), true);
                    }
                }
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    {
        let requests = Rc::clone(&requests);
        let worker = Rc::clone(&worker);
        let detector_requests = Rc::clone(&detector_requests);
        let detector_worker = Rc::clone(&detector_worker);
        let reader = Rc::clone(&reader);
        let alive = Rc::clone(&alive);
        window.connect_close_request(move |_| {
            alive.set(false);
            detector_cancel.store(true, Ordering::Relaxed);
            let reader = reader.borrow();
            if let Err(error) = reader.progress.save(
                &reader.path,
                reader.position,
                reader.position.page + 1 == reader.page_count,
            ) {
                eprintln!("panel: failed to save reading position: {error:#}");
            }
            drop(reader);
            let _ = requests.try_send(WorkerRequest::Stop);
            if let Some(worker) = worker.borrow_mut().take() {
                thread::spawn(move || {
                    let _ = worker.join();
                });
            }
            let _ = detector_requests.try_send(detector::CommandMessage::Stop);
            if let Some(worker) = detector_worker.borrow_mut().take() {
                thread::spawn(move || {
                    let _ = worker.join();
                });
            }
            glib::Propagation::Proceed
        });
    }
    window.present();
    Ok(())
}

struct DetectionResult {
    regions: Vec<Region>,
    source: String,
    fallback_reason: Option<String>,
    cached: bool,
}

fn load_or_detect(
    connection: &rusqlite::Connection,
    path: &std::path::Path,
    page: usize,
    image: &RgbaImage,
) -> DetectionResult {
    match detection_cache::load_external(connection, path, page, image.width(), image.height()) {
        Ok(Some(imported)) if !imported.regions.is_empty() => {
            return ordered(DetectionResult {
                regions: imported.regions,
                source: imported.source,
                fallback_reason: None,
                cached: true,
            });
        }
        Ok(_) => {}
        Err(error) => eprintln!("panel: failed to read external regions: {error:#}"),
    }
    match detection_cache::load(connection, path, page, image.width(), image.height()) {
        Ok(Some(cached)) if !cached.regions.is_empty() => {
            return ordered(DetectionResult {
                regions: cached.regions,
                source: cached.source,
                fallback_reason: cached.fallback_reason,
                cached: true,
            });
        }
        Ok(_) => {}
        Err(error) => eprintln!("panel: failed to read detection cache: {error:#}"),
    }
    let result = detect_regions(image);
    let cached = CachedDetection {
        regions: result.regions.clone(),
        source: result.source.clone(),
        fallback_reason: result.fallback_reason.clone(),
    };
    if let Err(error) = detection_cache::save(
        connection,
        path,
        page,
        image.width(),
        image.height(),
        &cached,
    ) {
        eprintln!("panel: failed to save detection cache: {error:#}");
    }
    ordered(result)
}

fn ordered(mut detection: DetectionResult) -> DetectionResult {
    region::order_rtl(&mut detection.regions);
    detection
}

fn apply_external(detection: &mut DetectionResult, external: CachedDetection) {
    if external.regions.is_empty() {
        return;
    }
    detection.regions = external.regions;
    detection.source = external.source;
    detection.fallback_reason = None;
    region::order_rtl(&mut detection.regions);
}

fn detect_regions(image: &RgbaImage) -> DetectionResult {
    let detection = region::detect(image);
    let fallback_reason = detection.fallback_reason.map(str::to_owned);
    let source = if fallback_reason.is_some() {
        "whole-page"
    } else {
        "gutter"
    };
    DetectionResult {
        regions: detection.regions,
        source: source.into(),
        fallback_reason,
        cached: false,
    }
}

fn make_page(image: RgbaImage, detection: DetectionResult) -> Page {
    let detection = ordered(detection);
    Page {
        texture: texture(image),
        regions: detection.regions,
        source: detection.source,
        fallback_reason: detection.fallback_reason,
    }
}

fn present(reader: &Reader, view: &PageView, status: &Label, loading: bool) {
    let bounds = region::padded(
        reader.page.regions[reader.position.region],
        reader.page.texture.width() as u32,
        reader.page.texture.height() as u32,
    );
    view.set_page(reader.page.texture.clone(), bounds);
    update_status(reader, status, view.mode(), loading);
}

fn update_status(reader: &Reader, status: &Label, mode: ViewMode, loading: bool) {
    let mode = match mode {
        ViewMode::Crop => "crop",
        ViewMode::Focus => "focus",
        ViewMode::Page => "page",
    };
    let current = reader.page.fallback_reason.as_ref().map_or_else(
        || reader.page.source.clone(),
        |reason| format!("whole page · {reason}"),
    );
    let remaining = (reader.page_count - reader.position.page - 1).min(10);
    let ahead = if remaining == 0 {
        String::new()
    } else {
        match detection_cache::count_ahead(
            reader.progress.connection(),
            &reader.path,
            reader.position.page,
            remaining,
        ) {
            Ok(ready) if detector::configured() || ready > 0 => {
                format!(" · ahead {ready}/{remaining} ready")
            }
            Ok(_) => String::new(),
            Err(error) => {
                eprintln!("panel: failed to read detector coverage: {error:#}");
                String::new()
            }
        }
    };
    let loading = if loading { " · loading page…" } else { "" };
    status.set_text(&format!(
        "{} — page {}/{} · panel {}/{} · {} · {}{}{}",
        reader.title,
        reader.position.page + 1,
        reader.page_count,
        reader.position.region + 1,
        reader.page.regions.len(),
        mode,
        current,
        ahead,
        loading,
    ));
}

fn texture(image: RgbaImage) -> gdk::Texture {
    document::texture(image)
}

fn restored_region(restored: Position, initial_page: Option<usize>, region_count: usize) -> usize {
    if initial_page.is_some() {
        0
    } else {
        restored.region.min(region_count.saturating_sub(1))
    }
}

fn move_region(reader: &mut Reader, delta: isize) -> bool {
    if delta > 0 && reader.position.region + 1 < reader.page.regions.len() {
        reader.position.region += 1;
        true
    } else if delta < 0 && reader.position.region > 0 {
        reader.position.region -= 1;
        true
    } else {
        false
    }
}

fn install_css() {
    let provider = CssProvider::new();
    provider.load_from_data(&format!(
        ".reader {{ background-color: {GRUVBOX_DARK}; }}\n.status {{ background-color: #3c3836; color: #ebdbb2; padding: 6px 12px; }}\n.zoom-indicator {{ background-color: #3c3836; color: #ebdbb2; padding: 6px 10px; border-radius: 4px; }}"
    ));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DetectionResult, ReaderExit, apply_external, exit_key, load_or_detect, restored_region,
    };
    use crate::{
        detection_cache::{self, CachedDetection},
        progress::{Position, ProgressStore},
        region::Region,
    };
    use gtk::gdk;
    use image::RgbaImage;
    use std::{fs, time::SystemTime};

    #[test]
    fn escape_returns_to_library_and_q_quits() {
        assert_eq!(exit_key(gdk::Key::Escape), Some(ReaderExit::Library));
        assert_eq!(exit_key(gdk::Key::q), Some(ReaderExit::Quit));
        assert_eq!(exit_key(gdk::Key::w), None);
    }

    #[test]
    fn restores_saved_panel_unless_a_chapter_was_chosen() {
        let saved = Position {
            page: 12,
            region: 3,
        };
        assert_eq!(restored_region(saved, None, 5), 3);
        assert_eq!(restored_region(saved, None, 2), 1);
        assert_eq!(restored_region(saved, Some(12), 5), 0);
    }

    #[test]
    fn late_external_cache_override_uses_the_new_order() {
        let left = Region {
            x: 0,
            y: 0,
            width: 110,
            height: 200,
        };
        let top = Region {
            x: 120,
            y: 10,
            width: 80,
            height: 80,
        };
        let bottom = Region {
            x: 120,
            y: 100,
            width: 80,
            height: 95,
        };
        let footer = Region {
            x: 0,
            y: 198,
            width: 200,
            height: 70,
        };
        let mut detection = DetectionResult {
            regions: vec![footer],
            source: "gutter".into(),
            fallback_reason: Some("old analysis".into()),
            cached: true,
        };
        apply_external(
            &mut detection,
            CachedDetection {
                regions: vec![top, left, bottom, footer],
                source: "magi".into(),
                fallback_reason: None,
            },
        );
        assert_eq!(detection.regions, [top, bottom, left, footer]);
        assert_eq!(detection.source, "magi");
        assert_eq!(detection.fallback_reason, None);
        apply_external(
            &mut detection,
            CachedDetection {
                regions: vec![],
                source: "broken".into(),
                fallback_reason: None,
            },
        );
        assert_eq!(detection.regions, [top, bottom, left, footer]);
    }

    #[test]
    fn previously_cached_external_panels_are_reordered_without_reanalysis() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let book = std::env::temp_dir().join(format!(
            "panel-order-cache-{}-{unique}.cbz",
            std::process::id()
        ));
        fs::write(&book, []).unwrap();
        let store = ProgressStore::in_memory().unwrap();
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
        detection_cache::save_external(
            store.connection(),
            &book,
            0,
            200,
            210,
            "magi",
            &[left, top, bottom],
        )
        .unwrap();
        let detection = load_or_detect(store.connection(), &book, 0, &RgbaImage::new(200, 210));
        assert_eq!(detection.regions, [top, bottom, left]);
        assert_eq!(detection.source, "magi");
        assert!(detection.cached);
        fs::remove_file(book).unwrap();
    }
}
