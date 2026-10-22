use image::{GrayImage, RgbaImage, imageops};

const MIN_REGION_AREA_FRACTION: f64 = 0.10;
const MAX_REGIONS: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub(crate) fn area(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub(crate) fn right(self) -> u32 {
        self.x + self.width
    }

    pub(crate) fn bottom(self) -> u32 {
        self.y + self.height
    }

    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Detection {
    pub regions: Vec<Region>,
    pub fallback_reason: Option<&'static str>,
}

pub fn detect(page: &RgbaImage) -> Detection {
    // Projection analysis does not need source resolution. Keeping the longest
    // edge near 1000 px bounds detector cost on very large scans.
    let scale = (page.width().max(page.height()) as f32 / 1000.0).max(1.0);
    let analysis_width = (page.width() as f32 / scale).round() as u32;
    let analysis_height = (page.height() as f32 / scale).round() as u32;
    let gray = imageops::resize(
        &imageops::grayscale(page),
        analysis_width,
        analysis_height,
        imageops::FilterType::Triangle,
    );
    let analysis_root = Region {
        x: 0,
        y: 0,
        width: analysis_width,
        height: analysis_height,
    };
    let mut regions = Vec::new();
    split(&gray, analysis_root, analysis_root.area(), 0, &mut regions);
    regions = regions
        .into_iter()
        .map(|region| Region {
            x: (region.x as f32 * scale).round() as u32,
            y: (region.y as f32 * scale).round() as u32,
            width: (region.width as f32 * scale).round() as u32,
            height: (region.height as f32 * scale).round() as u32,
        })
        .collect();
    order_rtl(&mut regions);
    merge_small_regions(
        &mut regions,
        u64::from(page.width()) * u64::from(page.height()),
    );
    order_rtl(&mut regions);

    let root = Region {
        x: 0,
        y: 0,
        width: page.width(),
        height: page.height(),
    };
    if let Some(reason) = fallback_reason(&regions, root) {
        Detection {
            regions: vec![root],
            fallback_reason: Some(reason),
        }
    } else {
        Detection {
            regions,
            fallback_reason: None,
        }
    }
}

fn split(gray: &GrayImage, region: Region, page_area: u64, depth: u8, output: &mut Vec<Region>) {
    if depth >= 5 || region.area() < page_area / 4 {
        output.push(region);
        return;
    }
    let horizontal = best_gap(gray, region, Axis::Horizontal);
    let vertical = best_gap(gray, region, Axis::Vertical);
    let choice = match (horizontal, vertical) {
        (Some(h), Some(v)) => Some(if h.score >= v.score { h } else { v }),
        (Some(gap), None) | (None, Some(gap)) => Some(gap),
        (None, None) => None,
    };
    let Some(gap) = choice else {
        output.push(region);
        return;
    };
    let (first, second) = match gap.axis {
        Axis::Horizontal => (
            Region {
                height: gap.start,
                ..region
            },
            Region {
                y: region.y + gap.end,
                height: region.height - gap.end,
                ..region
            },
        ),
        Axis::Vertical => (
            Region {
                width: gap.start,
                ..region
            },
            Region {
                x: region.x + gap.end,
                width: region.width - gap.end,
                ..region
            },
        ),
    };
    if first.area() < page_area / 18 || second.area() < page_area / 18 {
        output.push(region);
        return;
    }
    split(gray, first, page_area, depth + 1, output);
    split(gray, second, page_area, depth + 1, output);
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy)]
struct Gap {
    axis: Axis,
    start: u32,
    end: u32,
    score: f32,
}

fn best_gap(gray: &GrayImage, region: Region, axis: Axis) -> Option<Gap> {
    let (length, cross) = match axis {
        Axis::Horizontal => (region.height, region.width),
        Axis::Vertical => (region.width, region.height),
    };
    let edge = (length / 9).max(1);
    let minimum_gap = (length / 100).max(7);
    let mut best = None;
    let mut start = None;
    for offset in edge..length.saturating_sub(edge) {
        let mut light = 0;
        let mut dark = 0;
        for perpendicular in (0..cross).step_by(2) {
            let pixel = match axis {
                Axis::Horizontal => gray.get_pixel(region.x + perpendicular, region.y + offset)[0],
                Axis::Vertical => gray.get_pixel(region.x + offset, region.y + perpendicular)[0],
            };
            light += u32::from(pixel >= 245);
            dark += u32::from(pixel <= 10);
        }
        let blank = light.max(dark) as f32 / cross.div_ceil(2).max(1) as f32 >= 0.985;
        if blank {
            start.get_or_insert(offset);
        } else if let Some(run_start) = start.take() {
            consider_gap(axis, run_start, offset, length, minimum_gap, &mut best);
        }
    }
    if let Some(run_start) = start {
        consider_gap(
            axis,
            run_start,
            length - edge,
            length,
            minimum_gap,
            &mut best,
        );
    }
    best
}

fn consider_gap(
    axis: Axis,
    start: u32,
    end: u32,
    length: u32,
    minimum_gap: u32,
    best: &mut Option<Gap>,
) {
    let width = end - start;
    let left = start;
    let right = length - end;
    if width < minimum_gap || left < length / 6 || right < length / 6 {
        return;
    }
    let balance = left.min(right) as f32 / left.max(right) as f32;
    let candidate = Gap {
        axis,
        start,
        end,
        score: width as f32 / length as f32 + balance * 0.04,
    };
    if best.is_none_or(|current| candidate.score > current.score) {
        *best = Some(candidate);
    }
}

fn merge_small_regions(regions: &mut Vec<Region>, page_area: u64) {
    let minimum = (page_area as f64 * MIN_REGION_AREA_FRACTION) as u64;
    while regions.len() > 1 {
        let Some(index) = regions.iter().position(|region| region.area() < minimum) else {
            break;
        };
        let target = regions
            .iter()
            .enumerate()
            .filter(|(candidate, _)| *candidate != index)
            .min_by_key(|(_, candidate)| distance(regions[index], **candidate))
            .map_or(index, |(candidate, _)| candidate);
        let merged = regions[index].union(regions[target]);
        let high = index.max(target);
        let low = index.min(target);
        regions.remove(high);
        regions.remove(low);
        regions.insert(low, merged);
        order_rtl(regions);
    }
}

fn distance(a: Region, b: Region) -> u64 {
    let horizontal = if a.right() < b.x {
        b.x - a.right()
    } else {
        a.x.saturating_sub(b.right())
    };
    let vertical = if a.bottom() < b.y {
        b.y - a.bottom()
    } else {
        a.y.saturating_sub(b.bottom())
    };
    u64::from(horizontal).pow(2) + u64::from(vertical).pow(2)
}

fn fallback_reason(regions: &[Region], page: Region) -> Option<&'static str> {
    if regions.len() < 2 {
        return Some("no reliable gutter layout");
    }
    if regions.len() > MAX_REGIONS {
        return Some("layout too complex");
    }
    let coverage =
        regions.iter().map(|region| region.area()).sum::<u64>() as f64 / page.area() as f64;
    if !(0.72..=0.985).contains(&coverage) {
        return Some("irregular layout");
    }
    if regions
        .iter()
        .any(|region| region.area() as f64 / (page.area() as f64) < MIN_REGION_AREA_FRACTION)
    {
        return Some("small ambiguous panel");
    }
    None
}

/// Order panel *groups*, not individual top edges. A page-wide horizontal
/// gutter separates reading tiers; otherwise an uninterrupted vertical gutter
/// separates columns. In a grid, tiers take precedence over columns.
pub(crate) fn order_rtl(regions: &mut [Region]) {
    let mut ordered = Vec::with_capacity(regions.len());
    order_group(regions.to_vec(), &mut ordered);
    regions.copy_from_slice(&ordered);
}

fn order_group(group: Vec<Region>, ordered: &mut Vec<Region>) {
    if group.len() < 2 {
        ordered.extend(group);
        return;
    }
    if let Some((top, bottom)) = divide(&group, true) {
        order_group(top, ordered);
        order_group(bottom, ordered);
    } else if let Some((left, right)) = divide(&group, false) {
        order_group(right, ordered);
        order_group(left, ordered);
    } else {
        // An overlapping/irregular layout has no reliable full-length gutter.
        // Use a deterministic fallback rather than a non-transitive comparator.
        let mut group = group;
        group.sort_by_key(|r| (r.y, std::cmp::Reverse(r.x), r.bottom(), r.right()));
        ordered.extend(group);
    }
}

fn divide(group: &[Region], horizontal: bool) -> Option<(Vec<Region>, Vec<Region>)> {
    let mut best: Option<(u32, usize, u32)> = None;
    for panel in group {
        let cut = if horizontal {
            panel.bottom()
        } else {
            panel.right()
        };
        let mut before = 0;
        let mut after = 0;
        let mut nearest = u32::MAX;
        let mut crossed = false;
        for candidate in group {
            let (start, end) = if horizontal {
                (candidate.y, candidate.bottom())
            } else {
                (candidate.x, candidate.right())
            };
            if end <= cut {
                before += 1;
            } else if start >= cut {
                after += 1;
                nearest = nearest.min(start);
            } else {
                crossed = true;
                break;
            }
        }
        if crossed || before == 0 || after == 0 {
            continue;
        }
        // Prefer a clear gutter, then a balanced split. A zero-width gutter
        // still counts when panel rectangles meet exactly at their edges.
        let candidate = (nearest - cut, before.min(after), cut);
        if best.is_none_or(|previous| candidate > previous) {
            best = Some(candidate);
        }
    }
    let cut = best?.2;
    let (before, after) = group.iter().copied().partition(|r| {
        if horizontal {
            r.bottom() <= cut
        } else {
            r.right() <= cut
        }
    });
    Some((before, after))
}

pub fn padded(region: Region, page_width: u32, page_height: u32) -> Region {
    let padding = (region.width.min(region.height) / 40).max(8);
    let x = region.x.saturating_sub(padding);
    let y = region.y.saturating_sub(padding);
    let right = (region.right() + padding).min(page_width);
    let bottom = (region.bottom() + padding).min(page_height);
    Region {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{Region, detect, order_rtl};

    fn panel(x: u32, y: u32, width: u32, height: u32) -> Region {
        Region {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn finishes_right_stack_before_tall_left_panel() {
        let left = panel(0, 0, 90, 210);
        let top_right = panel(110, 0, 90, 90);
        let bottom_right = panel(110, 110, 90, 90);
        let mut regions = [left, bottom_right, top_right];
        order_rtl(&mut regions);
        assert_eq!(regions, [top_right, bottom_right, left]);
        order_rtl(&mut regions);
        assert_eq!(regions, [top_right, bottom_right, left]);
    }

    #[test]
    fn reads_two_by_two_grid_by_rows_not_columns() {
        let top_left = panel(0, 0, 90, 90);
        let top_right = panel(110, 0, 90, 90);
        let bottom_left = panel(0, 110, 90, 90);
        let bottom_right = panel(110, 110, 90, 90);
        let mut regions = [bottom_left, bottom_right, top_left, top_right];
        order_rtl(&mut regions);
        assert_eq!(regions, [top_right, top_left, bottom_right, bottom_left]);
    }

    #[test]
    fn reads_full_width_header_before_both_columns() {
        let header = panel(0, 0, 200, 60);
        let left = panel(0, 80, 90, 210);
        let top_right = panel(110, 80, 90, 90);
        let bottom_right = panel(110, 190, 90, 90);
        let mut regions = [left, bottom_right, header, top_right];
        order_rtl(&mut regions);
        assert_eq!(regions, [header, top_right, bottom_right, left]);
    }

    #[test]
    fn ambiguous_overlaps_have_a_deterministic_fallback() {
        let left = panel(0, 0, 115, 210);
        let top_right = panel(110, 0, 90, 90);
        let bottom_right = panel(110, 110, 90, 90);
        let mut regions = [bottom_right, left, top_right];
        order_rtl(&mut regions);
        assert_eq!(regions, [top_right, left, bottom_right]);
    }

    #[test]
    fn splits_two_panels_across_a_white_gutter() {
        let mut page = RgbaImage::from_pixel(400, 300, Rgba([255, 255, 255, 255]));
        for y in 20..280 {
            for x in 20..190 {
                page.put_pixel(x, y, Rgba([80, 80, 80, 255]));
            }
            for x in 210..380 {
                page.put_pixel(x, y, Rgba([80, 80, 80, 255]));
            }
        }
        let detection = detect(&page);
        assert_eq!(detection.regions.len(), 2);
        assert!(detection.regions[0].x > detection.regions[1].x);
    }

    #[test]
    fn full_bleed_page_falls_back_to_whole_page() {
        let page = RgbaImage::from_pixel(400, 600, Rgba([90, 90, 90, 255]));
        let detection = detect(&page);
        assert_eq!(detection.regions.len(), 1);
        assert!(detection.fallback_reason.is_some());
    }
}
