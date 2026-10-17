//! Separable resampling: exact area averaging when shrinking, linear interpolation when enlarging.

use rayon::prelude::*;

use crate::types::Size2;

/// For each destination index, the contributing `(source index, weight)` pairs; weights sum to 1.
fn axis_weights(src: u32, dst: u32) -> Vec<Vec<(usize, f32)>> {
    let scale = src as f64 / dst as f64;
    (0..dst)
        .map(|i| {
            if scale >= 1.0 {
                let start = i as f64 * scale;
                let end = start + scale;
                let mut taps = Vec::new();
                let mut j = start.floor() as usize;
                while (j as f64) < end && j < src as usize {
                    let overlap = (end.min(j as f64 + 1.0) - start.max(j as f64)) as f32;
                    if overlap > 0.0 {
                        taps.push((j, overlap / scale as f32));
                    }
                    j += 1;
                }
                taps
            } else {
                let center = ((i as f64 + 0.5) * scale - 0.5).max(0.0);
                let left = (center.floor() as usize).min(src as usize - 1);
                let right = (left + 1).min(src as usize - 1);
                let frac = (center - left as f64) as f32;
                if right == left { vec![(left, 1.0)] } else { vec![(left, 1.0 - frac), (right, frac)] }
            }
        })
        .collect()
}

/// Resample an `N`-channel image stored row-major.
pub fn resample<const N: usize>(src: &[[f32; N]], from: Size2, to: Size2) -> Vec<[f32; N]> {
    assert_eq!(src.len(), from.area(), "source buffer does not match its size");
    let w = from.w as usize;
    resample_rows(from, to, |y, row: &mut [[f32; N]]| row.copy_from_slice(&src[y * w..(y + 1) * w]))
}

/// Resample an image whose source rows are produced on demand by `fill_row(y, row)`, so a large
/// source never has to exist as a full float buffer.
pub fn resample_rows<const N: usize, F>(from: Size2, to: Size2, fill_row: F) -> Vec<[f32; N]>
where
    F: Fn(usize, &mut [[f32; N]]) + Sync,
{
    assert!(from.w > 0 && from.h > 0 && to.w > 0 && to.h > 0, "empty image");
    let wx = axis_weights(from.w, to.w);
    let wy = axis_weights(from.h, to.h);

    let mut horizontal = vec![[0.0f32; N]; to.w as usize * from.h as usize];
    horizontal.par_chunks_mut(to.w as usize).enumerate().for_each_init(
        || vec![[0.0f32; N]; from.w as usize],
        |row, (y, out)| {
            fill_row(y, row);
            for (o, taps) in out.iter_mut().zip(&wx) {
                for &(j, w) in taps {
                    for c in 0..N {
                        o[c] += row[j][c] * w;
                    }
                }
            }
        },
    );

    let mut out = vec![[0.0f32; N]; to.area()];
    out.par_chunks_mut(to.w as usize).zip(&wy).for_each(|(row, taps)| {
        for &(j, w) in taps {
            let src_row = &horizontal[j * to.w as usize..(j + 1) * to.w as usize];
            for (o, s) in row.iter_mut().zip(src_row) {
                for c in 0..N {
                    o[c] += s[c] * w;
                }
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_downscale_preserves_mean() {
        let src: Vec<[f32; 1]> = (0..12).map(|i| [i as f32]).collect();
        let out = resample(&src, Size2 { w: 4, h: 3 }, Size2 { w: 2, h: 1 });
        // columns 0–1 average to 4.5 (values 0,1,4,5,8,9), columns 2–3 to 6.5
        assert!((out[0][0] - 4.5).abs() < 1e-5);
        assert!((out[1][0] - 6.5).abs() < 1e-5);
    }

    #[test]
    fn fractional_downscale_weights_sum_to_one() {
        for taps in axis_weights(10, 3) {
            let sum: f32 = taps.iter().map(|t| t.1).sum();
            assert!((sum - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn upscale_interpolates() {
        let src = vec![[0.0f32], [1.0]];
        let out = resample(&src, Size2 { w: 2, h: 1 }, Size2 { w: 4, h: 1 });
        assert_eq!(out[0][0], 0.0);
        assert_eq!(out[3][0], 1.0);
        assert!(out[1][0] > 0.0 && out[1][0] < out[2][0]);
    }
}
