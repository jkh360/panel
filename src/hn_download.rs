//! Local-only, atomic CBZ output for connector images. The browser never sees
//! this directory; only validated image bytes cross its private stdout pipe.
use std::{
    fs::{self, File, OpenOptions},
    io::Cursor,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use image::{ImageFormat, ImageReader, Limits};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const MAX_PAGE: usize = 12_000_000;
const MAX_BOOK: u64 = 512_000_000;

fn name(text: &str) -> String {
    let cleaned: String = text.chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .take(80).collect();
    let cleaned = cleaned.trim_matches([' ', '.']);
    if cleaned.is_empty() {
        "untitled".into()
    } else {
        cleaned.into()
    }
}

fn directory(parent: &Path, component: &str) -> Result<PathBuf> {
    let path = parent.join(component);
    match fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    ensure!(
        fs::symlink_metadata(&path)?.file_type().is_dir(),
        "unsafe download directory"
    );
    Ok(path)
}

fn suffix(identifier: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in identifier.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub struct CbzOutput {
    temp: PathBuf,
    target: PathBuf,
    zip: Option<ZipWriter<File>>,
    bytes: u64,
    saved: bool,
}

impl CbzOutput {
    pub fn new(
        base: &Path,
        source: &str,
        manga: &str,
        chapter: &str,
        chapter_id: &str,
    ) -> Result<Self> {
        ensure!(
            base.is_dir()
                && source.len() <= 80
                && !source.is_empty()
                && source.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'_'
                    || b == b'-'),
            "invalid download location"
        );
        let dir = directory(
            &directory(&directory(base, "Panel Downloads")?, source)?,
            &name(manga),
        )?;
        let target = dir.join(format!("{}--{}.cbz", name(chapter), suffix(chapter_id)));
        ensure!(!target.exists(), "chapter already downloaded");
        let mut created = None;
        for attempt in 0..8 {
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let temp = dir.join(format!(
                ".panel-{}-{now}-{attempt}.part",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)
            {
                Ok(file) => {
                    created = Some((temp, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let (temp, file) = created.context("could not create temporary archive")?;
        Ok(Self {
            temp,
            target,
            zip: Some(ZipWriter::new(file)),
            bytes: 0,
            saved: false,
        })
    }

    pub fn add_page(&mut self, index: usize, bytes: &[u8]) -> Result<()> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_PAGE,
            "image size limit"
        );
        let total = self
            .bytes
            .checked_add(bytes.len() as u64)
            .context("book size overflow")?;
        ensure!(total <= MAX_BOOK, "book size limit");
        let format = image::guess_format(bytes)?;
        let extension = match format {
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Png => "png",
            ImageFormat::WebP => "webp",
            _ => bail!("unsupported image format"),
        };
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(10_000);
        limits.max_image_height = Some(10_000);
        limits.max_alloc = Some(128_000_000);
        reader.limits(limits);
        let image = reader.decode().context("invalid image")?;
        ensure!(
            u64::from(image.width()) * u64::from(image.height()) <= 40_000_000,
            "image dimensions limit"
        );
        self.zip
            .as_mut()
            .context("archive already finished")?
            .start_file(
                format!("{:04}.{extension}", index + 1),
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )?;
        use std::io::Write;
        self.zip
            .as_mut()
            .context("archive already finished")?
            .write_all(bytes)?;
        self.bytes = total;
        Ok(())
    }

    pub fn finish(mut self) -> Result<PathBuf> {
        ensure!(self.bytes > 0, "empty chapter");
        self.zip
            .take()
            .context("archive already finished")?
            .finish()?
            .sync_all()?;
        // Hard-linking on the same filesystem publishes atomically without
        // overwriting a concurrently created file or following a target symlink.
        fs::hard_link(&self.temp, &self.target)
            .context("chapter already exists or cannot be saved")?;
        fs::remove_file(&self.temp)?;
        self.saved = true;
        Ok(self.target.clone())
    }
}

impl Drop for CbzOutput {
    fn drop(&mut self) {
        if !self.saved {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_validated_chapter_and_traversal_protection() {
        let root = std::env::temp_dir().join(format!(
            "panel-download-test-{}-{}",
            std::process::id(),
            suffix(file!())
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let mut writer =
            CbzOutput::new(&root, "source", "../Manga", "../../Chapter", "id").unwrap();
        assert!(writer.add_page(0, b"not an image").is_err());
        writer.add_page(0, &png).unwrap();
        let target = writer.finish().unwrap();
        assert!(target.starts_with(root.join("Panel Downloads/source")));
        let mut archive = zip::ZipArchive::new(File::open(&target).unwrap()).unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive.by_index(0).unwrap().name(), "0001.png");
        assert!(CbzOutput::new(&root, "source", "../Manga", "../../Chapter", "id").is_err());
        std::os::unix::fs::symlink(&root, root.join("Panel Downloads/source/Trap")).unwrap();
        assert!(CbzOutput::new(&root, "source", "Trap", "Chapter", "new").is_err());
        let dir = target.parent().unwrap();
        let unfinished =
            CbzOutput::new(&root, "source", "../Manga", "Unfinished", "different").unwrap();
        drop(unfinished);
        assert!(
            fs::read_dir(dir).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part"))
        );
        fs::remove_dir_all(root).unwrap();
    }
}
