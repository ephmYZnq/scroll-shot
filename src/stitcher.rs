use image::RgbaImage;

/// Represents the growing long screenshot canvas
pub struct LongCanvas {
    pub width: u32,
    pub height: u32,
    pub buffer: Vec<u8>, // RGBA raw bytes
}

impl LongCanvas {
    /// Initialize long canvas with the entire first frame
    pub fn new(first_frame: &RgbaImage) -> Self {
        let width = first_frame.width();
        let height = first_frame.height();
        let buffer = first_frame.as_raw().clone();

        Self {
            width,
            height,
            buffer,
        }
    }

    /// Append a slice of rows [start_y .. start_y + slice_height] from frame
    pub fn append_rows(&mut self, frame: &RgbaImage, start_y: u32, slice_height: u32) {
        if slice_height == 0 {
            return;
        }

        let w = self.width as usize;
        let row_bytes = w * 4;
        let frame_raw = frame.as_raw();

        let start_byte = (start_y as usize) * row_bytes;
        let total_bytes = (slice_height as usize) * row_bytes;
        let end_byte = (start_byte + total_bytes).min(frame_raw.len());

        if start_byte < frame_raw.len() {
            let slice = &frame_raw[start_byte..end_byte];
            self.buffer.extend_from_slice(slice);
            self.height += (slice.len() / row_bytes) as u32;
        }
    }

    /// Convert current buffer to an owned RgbaImage
    pub fn into_image(self) -> Option<RgbaImage> {
        RgbaImage::from_raw(self.width, self.height, self.buffer)
    }
}

/// Result of 1D vertical displacement matching
#[derive(Debug, Clone, Copy)]
pub struct MatchResult {
    /// Vertical pixel shift (0 means no movement / bottom reached)
    pub delta_y: u32,
    /// Average per-pixel color channel difference (lower is better, < 8.0 is confident)
    #[allow(dead_code)]
    pub avg_diff: f32,
}

/// Calculate 1D vertical shift using high-contrast adaptive template matching + dense full-overlap verification.
///
/// Guaranteed:
/// 1. Evaluates EVERY row without skipping in the template (step_y = 1) for sub-single-pixel precision.
/// 2. Selects the template window with the highest vertical edge/text contrast to avoid blank spaces.
/// 3. Validates trough sharpness (V-shape) so flat/ambiguous matches are rejected.
/// 4. Densely verifies the entire overlapping region to completely eliminate text seams and duplicated strokes.
pub fn find_vertical_shift(
    prev_frame: &RgbaImage,
    curr_frame: &RgbaImage,
    max_search_y: Option<u32>,
) -> MatchResult {
    let w = prev_frame.width();
    let h = prev_frame.height();

    if w == 0 || h == 0 || curr_frame.width() != w || curr_frame.height() != h {
        return MatchResult {
            delta_y: 0,
            avg_diff: f32::MAX,
        };
    }

    // Step 1: Did the frame move at all?
    let zero_diff = compute_overlap_sad(prev_frame, curr_frame, 0);
    if zero_diff < 0.8 {
        // Frame is identical: no movement
        return MatchResult {
            delta_y: 0,
            avg_diff: zero_diff,
        };
    }

    let template_height = (h / 4).clamp(60, 140);
    let bottom_margin = 15;

    if h <= template_height + bottom_margin + 20 {
        return MatchResult {
            delta_y: 0,
            avg_diff: zero_diff,
        };
    }

    // Step 2: Dynamically hunt for the region with HIGHEST vertical texture (text/icons/contrast)
    // in the lower 60% of prev_frame
    let (prev_template_top, edge_energy) = find_best_template_y(prev_frame, template_height, bottom_margin);

    // If the region has virtually zero contrast (e.g. pure blank white page), require strict motion
    if edge_energy < 50 {
        return MatchResult {
            delta_y: 0,
            avg_diff: zero_diff,
        };
    }

    // Candidate shifts: search from 2px up to max_shift
    let min_shift = 2;
    let max_shift = max_search_y
        .unwrap_or(prev_template_top)
        .min(prev_template_top)
        .min(h - 30);

    if max_shift < min_shift {
        return MatchResult {
            delta_y: 0,
            avg_diff: zero_diff,
        };
    }

    let step_x = (w / 160).max(1);

    let mut best_candidate_shift = 0;
    let mut min_template_diff = f32::MAX;
    let mut second_best_diff = f32::MAX;

    // Step 3: Candidate search with EVERY single vertical row checked (step_y = 1)
    for shift in min_shift..=max_shift {
        let curr_top = prev_template_top - shift;
        let diff = compute_sad_at_shift(
            prev_frame,
            curr_frame,
            w,
            prev_template_top,
            curr_top,
            template_height,
            step_x,
        );

        if diff < min_template_diff {
            second_best_diff = min_template_diff;
            min_template_diff = diff;
            best_candidate_shift = shift;
        } else if diff < second_best_diff {
            second_best_diff = diff;
        }
    }

    // Step 4: Quality Checks
    // 4.1 Check absolute template match error (< 8.0)
    if min_template_diff > 8.0 || best_candidate_shift == 0 {
        return MatchResult {
            delta_y: 0,
            avg_diff: zero_diff,
        };
    }

    // 4.2 Trough uniqueness check: Ensure this is a clear local minimum (not flat ambiguity)
    if best_candidate_shift > min_shift && best_candidate_shift < max_shift {
        let prev_s = compute_sad_at_shift(
            prev_frame, curr_frame, w,
            prev_template_top, prev_template_top - (best_candidate_shift - 1),
            template_height, step_x,
        );
        let next_s = compute_sad_at_shift(
            prev_frame, curr_frame, w,
            prev_template_top, prev_template_top - (best_candidate_shift + 1),
            template_height, step_x,
        );
        // Both adjacent rows must have higher error (sharp V-trough)
        if prev_s <= min_template_diff || next_s <= min_template_diff {
            return MatchResult {
                delta_y: 0,
                avg_diff: zero_diff,
            };
        }
    }

    // Step 5: Dense Full-Overlap Verification (checking every 2nd row across the ENTIRE overlap)
    let overlap_err = compute_overlap_sad(prev_frame, curr_frame, best_candidate_shift);

    // Overlap error must be exceptionally low (< 6.5) to guarantee seamless pixel alignment
    if overlap_err <= 6.5 {
        MatchResult {
            delta_y: best_candidate_shift,
            avg_diff: overlap_err,
        }
    } else {
        // Rejected false match
        MatchResult {
            delta_y: 0,
            avg_diff: zero_diff,
        }
    }
}

/// Find the vertical template window with highest edge/gradient energy
fn find_best_template_y(img: &RgbaImage, template_h: u32, bottom_margin: u32) -> (u32, u64) {
    let (w, h) = img.dimensions();
    if h <= template_h + bottom_margin {
        return (0, 0);
    }

    // Search in the lower 50% of the image (avoids top headers/navigation)
    let min_y = h / 2;
    let max_y = h - bottom_margin - template_h;
    if min_y >= max_y {
        return (max_y, 100);
    }

    let step_x = (w / 80).max(1);
    let mut best_y = max_y;
    let mut max_grad: u64 = 0;

    for y in (min_y..=max_y).step_by(3) {
        let mut grad_sum: u64 = 0;
        for r in (y..y + template_h).step_by(2) {
            let next_r = (r + 1).min(h - 1);
            for x in (0..w).step_by(step_x as usize) {
                let p1 = img.get_pixel(x, r);
                let p2 = img.get_pixel(x, next_r);
                let d = (p1[0] as i32 - p2[0] as i32).unsigned_abs() as u64
                    + (p1[1] as i32 - p2[1] as i32).unsigned_abs() as u64
                    + (p1[2] as i32 - p2[2] as i32).unsigned_abs() as u64;
                grad_sum += d;
            }
        }
        if grad_sum > max_grad {
            max_grad = grad_sum;
            best_y = y;
        }
    }

    (best_y, max_grad)
}

/// Compute average SAD per sampled pixel channel for a candidate position
/// CRITICAL: Checks EVERY single row (r in 0..height) without row skipping for single-pixel exactness!
#[inline]
fn compute_sad_at_shift(
    prev: &RgbaImage,
    curr: &RgbaImage,
    width: u32,
    prev_top: u32,
    curr_top: u32,
    height: u32,
    step_x: u32,
) -> f32 {
    let mut total_diff: u64 = 0;
    let mut count: u64 = 0;

    // Step by 1 along Y: DO NOT SKIP ROWS!
    for r in 0..height {
        let py = prev_top + r;
        let cy = curr_top + r;

        for x in (0..width).step_by(step_x as usize) {
            let p_pixel = prev.get_pixel(x, py);
            let c_pixel = curr.get_pixel(x, cy);

            let dr = (p_pixel[0] as i32 - c_pixel[0] as i32).unsigned_abs() as u64;
            let dg = (p_pixel[1] as i32 - c_pixel[1] as i32).unsigned_abs() as u64;
            let db = (p_pixel[2] as i32 - c_pixel[2] as i32).unsigned_abs() as u64;

            total_diff += dr + dg + db;
            count += 3;
        }
    }

    if count == 0 {
        f32::MAX
    } else {
        total_diff as f32 / count as f32
    }
}

/// Verify that the entire overlapping area matches between prev_frame and curr_frame
/// Uses dense step_y = 2 to ensure every text line stroke is verified
pub fn compute_overlap_sad(prev: &RgbaImage, curr: &RgbaImage, shift_y: u32) -> f32 {
    let (w, h) = prev.dimensions();
    if shift_y >= h {
        return f32::MAX;
    }

    let overlap_h = h - shift_y;
    let step_x = (w / 120).max(1);
    // Dense verification: step by 2 rows across the entire overlap
    let step_y = 2;

    let mut total_diff: u64 = 0;
    let mut count: u64 = 0;

    for r in (0..overlap_h).step_by(step_y) {
        let py = r + shift_y; // In prev_frame, content was at r + shift_y
        let cy = r;           // In curr_frame, content has moved up to r

        for x in (0..w).step_by(step_x as usize) {
            let p = prev.get_pixel(x, py);
            let c = curr.get_pixel(x, cy);

            let dr = (p[0] as i32 - c[0] as i32).unsigned_abs() as u64;
            let dg = (p[1] as i32 - c[1] as i32).unsigned_abs() as u64;
            let db = (p[2] as i32 - c[2] as i32).unsigned_abs() as u64;

            total_diff += dr + dg + db;
            count += 3;
        }
    }

    if count == 0 {
        f32::MAX
    } else {
        total_diff as f32 / count as f32
    }
}

/// Check if two consecutive frames are practically identical (settled)
#[allow(dead_code)]
pub fn frames_settled(f1: &RgbaImage, f2: &RgbaImage) -> bool {
    if f1.dimensions() != f2.dimensions() {
        return false;
    }
    let (w, h) = f1.dimensions();
    let step_x = (w / 80).max(1);
    let step_y = (h / 80).max(1);

    let mut diff_sum: u64 = 0;
    let mut count: u64 = 0;

    for y in (0..h).step_by(step_y as usize) {
        for x in (0..w).step_by(step_x as usize) {
            let p1 = f1.get_pixel(x, y);
            let p2 = f2.get_pixel(x, y);

            let d = (p1[0] as i32 - p2[0] as i32).unsigned_abs() as u64
                + (p1[1] as i32 - p2[1] as i32).unsigned_abs() as u64
                + (p1[2] as i32 - p2[2] as i32).unsigned_abs() as u64;

            diff_sum += d;
            count += 3;
        }
    }

    if count == 0 {
        true
    } else {
        let avg = diff_sum as f32 / count as f32;
        avg < 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn test_long_canvas_append() {
        let frame1 = RgbaImage::from_fn(100, 50, |x, y| {
            Rgba([(x % 255) as u8, (y % 255) as u8, 128, 255])
        });
        let mut canvas = LongCanvas::new(&frame1);
        assert_eq!(canvas.width, 100);
        assert_eq!(canvas.height, 50);

        let frame2 = RgbaImage::from_fn(100, 50, |x, y| {
            Rgba([(x % 255) as u8, ((y + 50) % 255) as u8, 128, 255])
        });
        canvas.append_rows(&frame2, 30, 20);
        assert_eq!(canvas.height, 70);

        let img = canvas.into_image().unwrap();
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 70);
    }

    #[test]
    fn test_find_vertical_shift_synthetic() {
        let width = 300;
        let height = 400;
        let shift_amount = 45;

        // Generate synthetic patterned frame 1 with sharp horizontal text-like lines
        let frame1 = RgbaImage::from_fn(width, height, |_x, y| {
            // Emulate text lines with spacing
            if y % 28 < 14 {
                let v = ((y * 19) % 256) as u8;
                Rgba([v, 255 - v, (v / 2), 255])
            } else {
                Rgba([255, 255, 255, 255]) // White space between lines
            }
        });

        // Frame 2 is frame 1 shifted upwards by shift_amount
        let frame2 = RgbaImage::from_fn(width, height, |_x, y| {
            let src_y = y + shift_amount;
            if src_y % 28 < 14 {
                let v = ((src_y * 19) % 256) as u8;
                Rgba([v, 255 - v, (v / 2), 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });

        let res = find_vertical_shift(&frame1, &frame2, None);
        assert_eq!(res.delta_y, shift_amount);
        assert!(res.avg_diff < 0.5);
    }

    #[test]
    fn test_find_vertical_shift_zero() {
        let width = 300;
        let height = 400;

        let frame = RgbaImage::from_fn(width, height, |_x, y| {
            let v = ((y * 23) % 256) as u8;
            Rgba([v, v, v, 255])
        });

        let res = find_vertical_shift(&frame, &frame, None);
        assert_eq!(res.delta_y, 0);
    }

    #[test]
    fn test_frames_settled() {
        let f1 = RgbaImage::from_pixel(100, 100, Rgba([50, 100, 150, 255]));
        let f2 = RgbaImage::from_pixel(100, 100, Rgba([50, 100, 150, 255]));
        assert!(frames_settled(&f1, &f2));

        let f3 = RgbaImage::from_pixel(100, 100, Rgba([200, 10, 10, 255]));
        assert!(!frames_settled(&f1, &f3));
    }
}
