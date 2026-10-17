//! Quantisation of a scalar field onto (possibly uneven) levels, with optional dithering.

use std::sync::OnceLock;

use crate::settings::Dither;
use crate::types::Size2;

/// Index of the level nearest to `v`; `levels` must be ascending.
pub fn nearest_level(levels: &[f32], v: f32) -> usize {
    let upper = levels.partition_point(|&l| l < v);
    if upper == 0 {
        0
    } else if upper == levels.len() {
        levels.len() - 1
    } else if v - levels[upper - 1] <= levels[upper] - v {
        upper - 1
    } else {
        upper
    }
}

fn bayer(order: u32, x: u32, y: u32) -> f32 {
    // Recursive construction of the Bayer index matrix of size 2^order.
    let mut value = 0u32;
    for bit in 0..order {
        let bx = (x >> bit) & 1;
        let by = (y >> bit) & 1;
        value |= ((bx ^ by) << 1 | by) << (2 * (order - 1 - bit));
    }
    (value as f32 + 0.5) / (1u32 << (2 * order)) as f32
}

/// Side of the tiled blue-noise threshold matrix.
const BLUE_SIDE: usize = 64;

/// Blue-noise thresholds, row-major over `BLUE_SIDE²`, each `(rank + 0.5) / BLUE_SIDE²`.
fn blue_noise() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(void_and_cluster)
}

/// Ulichney's void-and-cluster on a torus: ranks every cell so that each prefix of the ranking is
/// spread as evenly as possible. Deterministic, so every frame gets identical thresholds.
fn void_and_cluster() -> Vec<f32> {
    const N: usize = BLUE_SIDE;
    const AREA: usize = N * N;
    let two_sigma_sq = 2.0 * 1.5f32 * 1.5;
    let wrap = |d: usize| d.min(N - d) as f32;
    let kernel: Vec<f32> = (0..AREA)
        .map(|i| {
            let (dx, dy) = (wrap(i % N), wrap(i / N));
            (-(dx * dx + dy * dy) / two_sigma_sq).exp()
        })
        .collect();
    // Energy of a cell = Gaussian-weighted count of set cells around it.
    let toggle = |on: &mut [bool], energy: &mut [f32], p: usize| {
        on[p] = !on[p];
        let sign = if on[p] { 1.0 } else { -1.0 };
        let (px, py) = (p % N, p / N);
        for (i, e) in energy.iter_mut().enumerate() {
            let (dx, dy) = ((i % N + N - px) % N, (i / N + N - py) % N);
            *e += sign * kernel[dy * N + dx];
        }
    };
    let tightest_cluster = |on: &[bool], energy: &[f32]| {
        (0..AREA).filter(|&i| on[i]).max_by(|&a, &b| energy[a].total_cmp(&energy[b])).unwrap()
    };
    let largest_void = |on: &[bool], energy: &[f32]| {
        (0..AREA).filter(|&i| !on[i]).min_by(|&a, &b| energy[a].total_cmp(&energy[b])).unwrap()
    };

    let (mut on, mut energy) = (vec![false; AREA], vec![0.0f32; AREA]);
    let initial = AREA / 10;
    let mut state = 0x9E37_79B9u32;
    let mut placed = 0;
    while placed < initial {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let p = state as usize % AREA;
        if !on[p] {
            toggle(&mut on, &mut energy, p);
            placed += 1;
        }
    }
    // Relax the initial pattern: move the tightest cluster into the largest void until stable.
    for _ in 0..AREA {
        let cluster = tightest_cluster(&on, &energy);
        toggle(&mut on, &mut energy, cluster);
        let void = largest_void(&on, &energy);
        toggle(&mut on, &mut energy, void);
        if void == cluster {
            break;
        }
    }

    let mut rank = vec![0usize; AREA];
    let (prototype_on, prototype_energy) = (on.clone(), energy.clone());
    for r in (0..initial).rev() {
        let p = tightest_cluster(&on, &energy);
        toggle(&mut on, &mut energy, p);
        rank[p] = r;
    }
    let (mut on, mut energy) = (prototype_on, prototype_energy);
    for r in initial..AREA {
        let p = largest_void(&on, &energy);
        toggle(&mut on, &mut energy, p);
        rank[p] = r;
    }
    rank.iter().map(|&r| (r as f32 + 0.5) / AREA as f32).collect()
}

/// Quantise `values` (row-major over `size`) to indices into ascending `levels`.
/// `pick(cell, value)` chooses the level for an already-dithered value — normally
/// [`nearest_level`], but callers may add hysteresis.
pub fn quantize(
    values: &[f32],
    size: Size2,
    levels: &[f32],
    dither: Dither,
    mut pick: impl FnMut(usize, f32) -> usize,
) -> Vec<usize> {
    let step = if levels.len() > 1 { 1.0 / (levels.len() - 1) as f32 } else { 1.0 };
    let (w, h) = (size.w as usize, size.h as usize);
    match dither {
        Dither::None => values.iter().enumerate().map(|(i, &v)| pick(i, v)).collect(),
        Dither::Bayer4 | Dither::Bayer8 => {
            let order = if dither == Dither::Bayer4 { 2 } else { 3 };
            values
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let threshold = bayer(order, (i % w) as u32, (i / w) as u32);
                    pick(i, v + (threshold - 0.5) * step)
                })
                .collect()
        }
        Dither::BlueNoise => {
            let table = blue_noise();
            values
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let threshold = table[(i / w) % BLUE_SIDE * BLUE_SIDE + (i % w) % BLUE_SIDE];
                    pick(i, v + (threshold - 0.5) * step)
                })
                .collect()
        }
        Dither::FloydSteinberg | Dither::Atkinson => {
            let kernel: &[(isize, isize, f32)] = if dither == Dither::FloydSteinberg {
                &[(1, 0, 7.0 / 16.0), (-1, 1, 3.0 / 16.0), (0, 1, 5.0 / 16.0), (1, 1, 1.0 / 16.0)]
            } else {
                &[(1, 0, 0.125), (2, 0, 0.125), (-1, 1, 0.125), (0, 1, 0.125), (1, 1, 0.125), (0, 2, 0.125)]
            };
            let mut field = values.to_vec();
            let mut out = vec![0usize; values.len()];
            for y in 0..h {
                // Serpentine scan avoids directional worm artefacts.
                let reverse = y % 2 == 1;
                for step_x in 0..w {
                    let x = if reverse { w - 1 - step_x } else { step_x };
                    let i = y * w + x;
                    let v = field[i];
                    let k = pick(i, v);
                    out[i] = k;
                    let err = v - levels[k];
                    for &(dx, dy, weight) in kernel {
                        let dx = if reverse { -dx } else { dx };
                        let (nx, ny) = (x as isize + dx, y as isize + dy);
                        if nx >= 0 && (nx as usize) < w && (ny as usize) < h {
                            field[ny as usize * w + nx as usize] += err * weight;
                        }
                    }
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_level_handles_uneven_levels() {
        let levels = [0.0, 0.1, 0.9, 1.0];
        assert_eq!(nearest_level(&levels, -1.0), 0);
        assert_eq!(nearest_level(&levels, 0.4), 1);
        assert_eq!(nearest_level(&levels, 0.6), 2);
        assert_eq!(nearest_level(&levels, 2.0), 3);
    }

    #[test]
    fn bayer_matrix_is_a_permutation() {
        let mut seen: Vec<f32> = (0..4).flat_map(|y| (0..4).map(move |x| bayer(2, x, y))).collect();
        seen.sort_by(f32::total_cmp);
        for (i, v) in seen.iter().enumerate() {
            assert!((v - (i as f32 + 0.5) / 16.0).abs() < 1e-6);
        }
    }

    #[test]
    fn blue_noise_matrix_is_a_permutation() {
        let mut seen = blue_noise().to_vec();
        assert_eq!(seen.len(), BLUE_SIDE * BLUE_SIDE);
        seen.sort_by(f32::total_cmp);
        for (i, v) in seen.iter().enumerate() {
            assert!((v - (i as f32 + 0.5) / seen.len() as f32).abs() < 1e-6);
        }
    }

    #[test]
    fn blue_noise_spreads_the_darkest_thresholds() {
        // The lowest 10 % of thresholds must not touch each other (toroidally): that is what
        // separates blue noise from white noise, where neighbours are common.
        let table = blue_noise();
        let n = BLUE_SIDE;
        let set = |x: usize, y: usize| table[(y % n) * n + x % n] < 0.1;
        for y in 0..n {
            for x in 0..n {
                if set(x, y) {
                    assert!(!set(x + 1, y) && !set(x, y + 1), "adjacent low thresholds at ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn error_diffusion_preserves_mean_grey() {
        let size = Size2 { w: 32, h: 32 };
        let values = vec![0.3f32; size.area()];
        for dither in [Dither::FloydSteinberg, Dither::Bayer8, Dither::BlueNoise] {
            let out = quantize(&values, size, &[0.0, 1.0], dither, |_, v| nearest_level(&[0.0, 1.0], v));
            let mean = out.iter().sum::<usize>() as f32 / out.len() as f32;
            assert!((mean - 0.3).abs() < 0.03, "{dither:?} mean {mean}");
        }
    }
}
