use std::{fs, path::Path};

use anyhow::{Context, Result, bail, ensure};

use crate::{
    detection_cache,
    document::CbzDocument,
    progress::ProgressStore,
    region::{Region, order_rtl},
};

pub fn run(book: &Path, predictions: &Path) -> Result<()> {
    let document = CbzDocument::open(book)?;
    let text = fs::read_to_string(predictions)
        .with_context(|| format!("failed to read {}", predictions.display()))?;
    let records = parse_records(&text)?;
    ensure!(!records.is_empty(), "prediction file contains no pages");
    let store = ProgressStore::open()?;
    let mut imported = 0;
    let mut whole_pages = 0;
    for record in records {
        let page = record.page;
        ensure!(
            page < document.page_count(),
            "prediction page {} is outside the book",
            page + 1
        );
        ensure!(
            record.width > 0 && record.height > 0,
            "page {} has invalid dimensions",
            page + 1
        );
        let mut regions = sanitize(record.regions, record.width, record.height);
        let source = if regions.is_empty() {
            whole_pages += 1;
            regions.push(Region {
                x: 0,
                y: 0,
                width: record.width,
                height: record.height,
            });
            "import: whole-page"
        } else {
            "import"
        };
        detection_cache::save_external(
            store.connection(),
            book,
            page,
            record.width,
            record.height,
            source,
            &regions,
        )?;
        imported += 1;
    }
    eprintln!(
        "panel: imported {imported} pages from external predictions ({whole_pages} whole-page fallbacks)"
    );
    Ok(())
}

struct Record {
    page: usize,
    width: u32,
    height: u32,
    regions: Vec<Region>,
}

fn parse_records(text: &str) -> Result<Vec<Record>> {
    // This parser intentionally accepts only the documented import schema.
    let body = text
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .context("predictions must be a JSON array")?;
    let objects = split_objects(body)?;
    objects
        .iter()
        .map(|object| {
            let size = array_numbers(object, "size")?;
            ensure!(size.len() == 2, "invalid size array");
            let flat = nested_array_numbers(object, "panels")?;
            ensure!(flat.len() % 4 == 0, "invalid panel array");
            let filename = string_field(object, "filename")?;
            let page = filename
                .rsplit_once("page-")
                .and_then(|(_, suffix)| suffix.split('.').next())
                .and_then(|number| number.parse::<usize>().ok())
                .context("filename must contain zero-based page-NNN")?;
            Ok(Record {
                page,
                width: size[0],
                height: size[1],
                regions: flat
                    .chunks_exact(4)
                    .map(|v| Region {
                        x: v[0],
                        y: v[1],
                        width: v[2],
                        height: v[3],
                    })
                    .collect(),
            })
        })
        .collect()
}

fn split_objects(body: &str) -> Result<Vec<&str>> {
    let mut output = Vec::new();
    let mut depth = 0;
    let mut start = None;
    let mut string = false;
    let mut escape = false;
    for (index, byte) in body.bytes().enumerate() {
        if string {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                string = false;
            }
            continue;
        }
        if byte == b'"' {
            string = true;
        } else if byte == b'{' {
            if depth == 0 {
                start = Some(index);
            }
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                output.push(&body[start.context("invalid object")?..=index]);
            }
        }
    }
    ensure!(depth == 0 && !string, "malformed JSON");
    Ok(output)
}

fn string_field<'a>(object: &'a str, key: &str) -> Result<&'a str> {
    let marker = format!("\"{key}\"");
    let after = object.split_once(&marker).context("missing field")?.1;
    let start = after.find('"').context("missing string")? + 1;
    let end = after[start..].find('"').context("unterminated string")?;
    Ok(&after[start..start + end])
}

fn array_body<'a>(object: &'a str, key: &str) -> Result<&'a str> {
    let marker = format!("\"{key}\"");
    let after = object.split_once(&marker).context("missing field")?.1;
    let start = after.find('[').context("missing array")?;
    let mut depth = 0;
    for (offset, byte) in after[start..].bytes().enumerate() {
        if byte == b'[' {
            depth += 1;
        } else if byte == b']' {
            depth -= 1;
            if depth == 0 {
                return Ok(&after[start + 1..start + offset]);
            }
        }
    }
    bail!("unterminated array")
}

fn array_numbers(object: &str, key: &str) -> Result<Vec<u32>> {
    numbers(array_body(object, key)?)
}
fn nested_array_numbers(object: &str, key: &str) -> Result<Vec<u32>> {
    numbers(array_body(object, key)?)
}
fn numbers(value: &str) -> Result<Vec<u32>> {
    value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().map_err(Into::into))
        .collect()
}

fn sanitize(regions: Vec<Region>, width: u32, height: u32) -> Vec<Region> {
    let page_area = u64::from(width) * u64::from(height);
    let mut output: Vec<_> = regions
        .into_iter()
        .filter_map(|r| {
            let x = r.x.min(width);
            let y = r.y.min(height);
            let right = r.right().min(width);
            let bottom = r.bottom().min(height);
            (right > x && bottom > y).then_some(Region {
                x,
                y,
                width: right - x,
                height: bottom - y,
            })
        })
        .filter(|r| r.area() as f64 / page_area as f64 >= 0.025)
        .collect();
    order_rtl(&mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::{parse_records, sanitize};
    use crate::region::Region;
    #[test]
    fn parses_region_output() {
        let r = parse_records(
            r#"[{"filename":"page-000.jpg","size":[100,200],"panels":[[1,2,30,40]]}]"#,
        )
        .unwrap();
        assert_eq!(r[0].page, 0);
        assert_eq!(r[0].regions[0].width, 30);
    }

    #[test]
    fn imported_panels_keep_the_right_column_together() {
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
}
