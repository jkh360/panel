use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::region::Region;

pub const DETECTOR_VERSION: &str = "gutter-v1";

#[derive(Clone, Debug)]
pub struct CachedDetection {
    pub regions: Vec<Region>,
    pub source: String,
    pub fallback_reason: Option<String>,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS detections (
             document_path TEXT NOT NULL,
             document_size INTEGER NOT NULL,
             document_modified INTEGER NOT NULL,
             page INTEGER NOT NULL CHECK(page >= 0),
             detector_version TEXT NOT NULL,
             width INTEGER NOT NULL CHECK(width > 0),
             height INTEGER NOT NULL CHECK(height > 0),
             source TEXT NOT NULL,
             fallback_reason TEXT,
             regions TEXT NOT NULL,
             PRIMARY KEY(document_path, document_size, document_modified, page, detector_version)
         );
         CREATE TABLE IF NOT EXISTS external_detections (
             document_path TEXT NOT NULL,
             document_size INTEGER NOT NULL,
             document_modified INTEGER NOT NULL,
             page INTEGER NOT NULL CHECK(page >= 0),
             width INTEGER NOT NULL CHECK(width > 0),
             height INTEGER NOT NULL CHECK(height > 0),
             source TEXT NOT NULL,
             regions TEXT NOT NULL,
             PRIMARY KEY(document_path, document_size, document_modified, page)
         );",
    )?;
    // Older installations stored external results under this name. Preserve
    // them on upgrade; never replace a newer result for the same page.
    let legacy_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'imported_detections')",
        [],
        |row| row.get(0),
    )?;
    if legacy_exists {
        connection.execute_batch(
            "INSERT OR IGNORE INTO external_detections
                 (document_path, document_size, document_modified, page,
                  width, height, source, regions)
             SELECT document_path, document_size, document_modified, page,
                    width, height, source, regions
             FROM imported_detections;",
        )?;
    }
    Ok(())
}

pub fn load_external(
    connection: &Connection,
    path: &Path,
    page: usize,
    width: u32,
    height: u32,
) -> Result<Option<CachedDetection>> {
    let identity = identity(path)?;
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT source, regions FROM external_detections
             WHERE document_path = ?1 AND document_size = ?2
               AND document_modified = ?3 AND page = ?4
               AND width = ?5 AND height = ?6",
            params![
                identity.path,
                identity.size,
                identity.modified,
                page as i64,
                width,
                height
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(source, encoded)| {
        Ok(CachedDetection {
            regions: decode_regions(&encoded)?,
            source,
            fallback_reason: None,
        })
    })
    .transpose()
}

pub fn save_external(
    connection: &Connection,
    path: &Path,
    page: usize,
    width: u32,
    height: u32,
    source: &str,
    regions: &[Region],
) -> Result<()> {
    let identity = identity(path)?;
    connection.execute(
        "INSERT INTO external_detections(
             document_path, document_size, document_modified, page,
             width, height, source, regions
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(document_path, document_size, document_modified, page)
         DO UPDATE SET width = excluded.width, height = excluded.height,
             source = excluded.source, regions = excluded.regions",
        params![
            identity.path,
            identity.size,
            identity.modified,
            page as i64,
            width,
            height,
            source,
            encode_regions(regions)
        ],
    )?;
    Ok(())
}

pub fn load(
    connection: &Connection,
    path: &Path,
    page: usize,
    width: u32,
    height: u32,
) -> Result<Option<CachedDetection>> {
    let identity = identity(path)?;
    let row: Option<(String, Option<String>, String)> = connection
        .query_row(
            "SELECT source, fallback_reason, regions FROM detections
             WHERE document_path = ?1 AND document_size = ?2
               AND document_modified = ?3 AND page = ?4
               AND detector_version = ?5 AND width = ?6 AND height = ?7",
            params![
                identity.path,
                identity.size,
                identity.modified,
                page as i64,
                DETECTOR_VERSION,
                width,
                height
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(source, fallback_reason, encoded)| {
        Ok(CachedDetection {
            regions: decode_regions(&encoded)?,
            source,
            fallback_reason,
        })
    })
    .transpose()
}

pub fn save(
    connection: &Connection,
    path: &Path,
    page: usize,
    width: u32,
    height: u32,
    detection: &CachedDetection,
) -> Result<()> {
    save_versioned(
        connection,
        path,
        page,
        width,
        height,
        DETECTOR_VERSION,
        detection,
    )
}

pub fn save_versioned(
    connection: &Connection,
    path: &Path,
    page: usize,
    width: u32,
    height: u32,
    detector_version: &str,
    detection: &CachedDetection,
) -> Result<()> {
    let identity = identity(path)?;
    connection.execute(
        "INSERT INTO detections(
             document_path, document_size, document_modified, page,
             detector_version, width, height, source, fallback_reason, regions
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(document_path, document_size, document_modified, page, detector_version)
         DO UPDATE SET width = excluded.width, height = excluded.height,
             source = excluded.source, fallback_reason = excluded.fallback_reason,
             regions = excluded.regions",
        params![
            identity.path,
            identity.size,
            identity.modified,
            page as i64,
            detector_version,
            width,
            height,
            detection.source,
            detection.fallback_reason,
            encode_regions(&detection.regions)
        ],
    )?;
    Ok(())
}

struct Identity {
    path: String,
    size: i64,
    modified: i64,
}

fn identity(path: &Path) -> Result<Identity> {
    let canonical = path.canonicalize()?;
    let metadata = canonical.metadata()?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    Ok(Identity {
        path: canonical.to_string_lossy().into_owned(),
        size: metadata.len() as i64,
        modified,
    })
}

fn encode_regions(regions: &[Region]) -> String {
    regions
        .iter()
        .map(|r| format!("{},{},{},{}", r.x, r.y, r.width, r.height))
        .collect::<Vec<_>>()
        .join(";")
}

fn decode_regions(encoded: &str) -> Result<Vec<Region>> {
    if encoded.is_empty() {
        return Ok(Vec::new());
    }
    encoded
        .split(';')
        .map(|value| {
            let values = value
                .split(',')
                .map(str::parse)
                .collect::<std::result::Result<Vec<u32>, _>>()?;
            anyhow::ensure!(values.len() == 4, "invalid cached region");
            Ok(Region {
                x: values[0],
                y: values[1],
                width: values[2],
                height: values[3],
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{decode_regions, encode_regions};
    use crate::region::Region;

    #[test]
    fn regions_round_trip() {
        let regions = vec![
            Region {
                x: 1,
                y: 2,
                width: 30,
                height: 40,
            },
            Region {
                x: 50,
                y: 60,
                width: 70,
                height: 80,
            },
        ];
        assert_eq!(decode_regions(&encode_regions(&regions)).unwrap(), regions);
    }
}
