use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use gtk::{gdk, glib, prelude::*};
use image::{DynamicImage, RgbaImage};
use zip::ZipArchive;

pub struct CbzDocument {
    archive: ZipArchive<File>,
    path: PathBuf,
    entries: Vec<String>,
}

impl CbzDocument {
    pub fn open(path: &Path) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
        let mut archive = ZipArchive::new(file)
            .with_context(|| format!("{} is not a valid CBZ/ZIP archive", path.display()))?;
        let mut entries = Vec::new();

        for index in 0..archive.len() {
            let entry = archive.by_index(index)?;
            if !entry.is_dir() && is_supported_image(entry.name()) {
                entries.push(entry.name().to_owned());
            }
        }
        entries.sort_by(|a, b| natord::compare_ignore_case(a, b));

        if entries.is_empty() {
            bail!("{} contains no supported images", path.display());
        }

        Ok(Self {
            archive,
            path: path.to_path_buf(),
            entries,
        })
    }

    pub fn title(&self) -> String {
        self.path
            .file_stem()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    pub fn page_count(&self) -> usize {
        self.entries.len()
    }

    pub fn page_name(&self, page: usize) -> &str {
        &self.entries[page]
    }

    fn page_bytes(&mut self, page: usize) -> Result<Vec<u8>> {
        let name = &self.entries[page];
        let mut entry = self
            .archive
            .by_name(name)
            .with_context(|| format!("archive entry disappeared: {name}"))?;
        let mut encoded = Vec::with_capacity(entry.size().try_into().unwrap_or(0));
        entry.read_to_end(&mut encoded)?;
        Ok(encoded)
    }

    pub fn load_page(&mut self, page: usize) -> Result<RgbaImage> {
        let encoded = self.page_bytes(page)?;
        image::load_from_memory(&encoded)
            .with_context(|| format!("failed to decode {}", self.entries[page]))
            .map(|image| image.into_rgba8())
    }

    pub fn load_cover(&mut self) -> Result<DynamicImage> {
        let jpeg = self.entries[0].rsplit_once('.').is_some_and(|(_, ext)| {
            ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg")
        });
        if jpeg {
            let encoded = self.page_bytes(0)?;
            let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(encoded));
            decoder.scale(600, 800)?;
            let pixels = decoder.decode()?;
            let info = decoder.info().context("JPEG cover has no dimensions")?;
            let (width, height) = (u32::from(info.width), u32::from(info.height));
            let cover = match info.pixel_format {
                jpeg_decoder::PixelFormat::RGB24 => {
                    image::RgbImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8)
                }
                jpeg_decoder::PixelFormat::L8 => {
                    image::GrayImage::from_raw(width, height, pixels).map(DynamicImage::ImageLuma8)
                }
                _ => None,
            };
            if let Some(cover) = cover {
                return Ok(cover);
            }
        }
        self.load_page(0).map(DynamicImage::ImageRgba8)
    }
}

pub fn texture(image: RgbaImage) -> gdk::Texture {
    let (width, height) = image.dimensions();
    let bytes = glib::Bytes::from_owned(image.into_raw());
    gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    )
    .upcast()
}

fn is_supported_image(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::is_supported_image;

    #[test]
    fn cover_decodes_when_sample_book_is_provided() {
        let Some(path) = std::env::var_os("PANEL_TEST_BOOK") else {
            return;
        };
        let mut book = super::CbzDocument::open(std::path::Path::new(&path)).unwrap();
        let cover = book.load_cover().unwrap();
        assert!(cover.width() > 0 && cover.height() > 0);
        assert!(cover.width() <= 1200 && cover.height() <= 1600);
    }

    #[test]
    fn image_extensions_are_case_insensitive() {
        assert!(is_supported_image("001.JPG"));
        assert!(is_supported_image("page.webp"));
        assert!(!is_supported_image("ComicInfo.xml"));
    }
}
