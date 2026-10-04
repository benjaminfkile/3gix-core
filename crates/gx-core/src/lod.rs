//! Octree cell selection: which cells of a frame to fetch for a camera.
//!
//! Implements the renderer's step "fetches matter cells near the camera at a
//! depth chosen by distance" of `space-model.md` section 2, over the octree
//! cells of section 5 (cell geometry in `matter-format.md` section 3.1). The
//! cells it selects are the cells whose sections the renderer then fetches
//! from every layer and composites (`space-model.md` section 8,
//! `matter-format.md` section 3.5) before deriving emission and extinction
//! (`matter-format.md` section 3.3).
//!
//! This is pure geometry. It never looks at matter and never fetches
//! anything: the same camera and frame always give the same cells, whatever
//! the cells hold.
//!
//! # Determinism
//!
//! Cells are refined in a fixed order (largest projected size first, ties by
//! ascending `(depth, x, y, z)`), the field of view tangent comes from a
//! written-out series using only correctly rounded operations, and the
//! result is sorted, so the selection is identical on every run and every
//! platform.

use core::cmp::Ordering;
use core::f64::consts::PI;
use std::collections::BinaryHeap;

use crate::detmath;
use crate::key::CellKey;
use crate::registry::Frame;
use crate::units::Vec3;

/// The cells chosen for one camera.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    /// Selected cells of one frame, sorted ascending by `(depth, x, y, z)`.
    /// No cell contains another.
    pub cells: Vec<CellKey>,
}

/// How finely [`select_cells`] refines.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SelectionParams {
    /// Largest projected cell edge, in pixels, that is not refined further.
    pub pixel_error: f64,
    /// Height of the view in pixels.
    pub view_height_px: f64,
    /// Vertical field of view in radians, between 0 and pi exclusive.
    pub vertical_fov_rad: f64,
    /// Most cells to return.
    pub max_cells: usize,
}

/// Selects the cells of `frame` to fetch for a camera at `camera_in_frame`
/// (meters, frame coordinates).
///
/// Starts from the depth 0 cell. For each cell, `distance` is the distance
/// from the camera to the nearest point of the cell (0 inside), and its
/// projected edge in pixels is
/// `edge / max(distance, edge) * view_height_px / (2 tan(fov / 2))`. A cell
/// whose projected edge exceeds `pixel_error` and whose depth is below
/// `frame.max_depth` is replaced by its eight children; any other cell is
/// selected. A cell whose distance exceeds `4 * frame.root_extent` is culled:
/// neither selected nor refined.
///
/// Cells are refined largest projected edge first. A refinement that would
/// make the selection larger than `max_cells` is not made, and refining
/// stops there: every cell still waiting is selected as is. The result never
/// holds more than `max_cells` cells, and `max_cells` 0 selects nothing.
///
/// A field of view outside `(0, pi)` or a non-finite input refines nothing,
/// so the selection is at most the depth 0 cell.
pub fn select_cells(frame: &Frame, camera_in_frame: Vec3, params: &SelectionParams) -> Selection {
    let extent = frame.root_extent;
    let cull = extent.value() * 4.0;
    let fov = params.vertical_fov_rad;
    let focal = if fov > 0.0 && fov < PI {
        params.view_height_px / (2.0 * detmath::tan(fov / 2.0))
    } else {
        0.0
    };

    // A cell's projected edge in pixels, or `None` if culled.
    let measure = |key: CellKey| -> Option<f64> {
        let g = key.geometry(extent);
        let e = g.edge.value();
        let gap = |c: f64, o: f64| {
            if c < o {
                o - c
            } else if c > o + e {
                c - (o + e)
            } else {
                0.0
            }
        };
        let d = Vec3::new(
            gap(camera_in_frame.x, g.origin.x),
            gap(camera_in_frame.y, g.origin.y),
            gap(camera_in_frame.z, g.origin.z),
        )
        .length();
        // Written so NaN distances are culled too.
        let in_range = d <= cull;
        if !in_range {
            return None;
        }
        Some(e / d.max(e) * focal)
    };
    let wants_refine =
        |key: CellKey, px: f64| px > params.pixel_error && key.depth < frame.max_depth;

    let mut done: Vec<CellKey> = Vec::new();
    let mut pending: BinaryHeap<Pending> = BinaryHeap::new();
    let root = CellKey {
        frame_id: frame.frame_id,
        depth: 0,
        x: 0,
        y: 0,
        z: 0,
    };
    if params.max_cells == 0 {
        return Selection::default();
    }
    let Some(px) = measure(root) else {
        return Selection::default();
    };
    if wants_refine(root, px) {
        pending.push(Pending { px, key: root });
    } else {
        done.push(root);
    }

    while let Some(cell) = pending.pop() {
        let kids: Vec<(CellKey, f64)> = cell
            .key
            .children()
            .into_iter()
            .flatten()
            .filter_map(|k| measure(k).map(|px| (k, px)))
            .collect();
        if done.len() + pending.len() + kids.len() > params.max_cells {
            done.push(cell.key);
            done.extend(pending.drain().map(|p| p.key));
            break;
        }
        for (k, px) in kids {
            if wants_refine(k, px) {
                pending.push(Pending { px, key: k });
            } else {
                done.push(k);
            }
        }
    }

    done.sort_by_key(|k| (k.depth, k.x, k.y, k.z));
    Selection { cells: done }
}

/// A cell waiting to be refined, ordered so the heap pops the largest
/// projected edge first and, among equal edges, the smallest
/// `(depth, x, y, z)`.
struct Pending {
    px: f64,
    key: CellKey,
}

impl Pending {
    fn order(&self) -> (u8, u32, u32, u32) {
        (self.key.depth, self.key.x, self.key.y, self.key.z)
    }
}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        self.px
            .total_cmp(&other.px)
            .then_with(|| other.order().cmp(&self.order()))
    }
}

impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Pending {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ROOT_PARENT;
    use crate::units::{Kilograms, Meters, Quat};

    const R: f64 = 1.0e6;

    fn frame(max_depth: u8) -> Frame {
        Frame {
            frame_id: 7,
            parent_frame_id: ROOT_PARENT,
            root_extent: Meters::new(R),
            max_depth,
            mass: Kilograms::new(0.0),
            position: Vec3::zero(),
            velocity: Vec3::zero(),
            orientation: Quat::identity(),
            angular_velocity: Vec3::zero(),
        }
    }

    fn params(max_cells: usize) -> SelectionParams {
        SelectionParams {
            pixel_error: 256.0,
            view_height_px: 1000.0,
            vertical_fov_rad: PI / 3.0,
            max_cells,
        }
    }

    /// Sum of each cell's share of the root volume, `8^-depth`.
    fn coverage(s: &Selection) -> f64 {
        s.cells
            .iter()
            .map(|k| 1.0 / (1u64 << (3 * u32::from(k.depth))) as f64)
            .sum()
    }

    fn containing(s: &Selection, p: Vec3) -> CellKey {
        let hits: Vec<_> = s
            .cells
            .iter()
            .filter(|k| k.contains_point(Meters::new(R), p))
            .collect();
        assert_eq!(hits.len(), 1, "{p:?}");
        *hits[0]
    }

    fn is_sorted(s: &Selection) -> bool {
        s.cells
            .windows(2)
            .all(|w| (w[0].depth, w[0].x, w[0].y, w[0].z) < (w[1].depth, w[1].x, w[1].y, w[1].z))
    }

    #[test]
    fn far_camera_gives_the_root() {
        // Nearest distance 3.5 R: projected edge 1000 / (2 tan 30 deg) / 3.5,
        // about 247 pixels, below the 256 pixel error.
        let s = select_cells(&frame(6), Vec3::new(4.0 * R, 0.0, 0.0), &params(10_000));
        assert_eq!(s.cells, vec![CellKey::new(7, 0, 0, 0, 0).unwrap()]);
    }

    #[test]
    fn camera_beyond_cull_distance_selects_nothing() {
        let s = select_cells(&frame(6), Vec3::new(0.0, 4.6 * R, 0.0), &params(10_000));
        assert!(s.cells.is_empty());
    }

    #[test]
    fn corner_camera_refines_near_and_not_far() {
        let cam = Vec3::new(-0.49 * R, -0.49 * R, -0.49 * R);
        let s = select_cells(&frame(6), cam, &params(100_000));
        assert!(is_sorted(&s));
        assert!((coverage(&s) - 1.0).abs() < 1e-12);
        assert_eq!(containing(&s, cam).depth, 6);
        let far = containing(&s, Vec3::new(0.49 * R, 0.49 * R, 0.49 * R));
        assert_eq!(far.depth, 2);
        let mid = containing(&s, Vec3::new(-0.2 * R, -0.2 * R, -0.2 * R));
        assert!(mid.depth > far.depth && mid.depth < 6);
        // Depth never increases moving away from the camera along the
        // diagonal.
        let mut last = u8::MAX;
        for i in 0..=98 {
            let c = -0.49 * R + f64::from(i) * 0.01 * R;
            let d = containing(&s, Vec3::new(c, c, c)).depth;
            assert!(d <= last, "depth {d} after {last} at step {i}");
            last = d;
        }
    }

    #[test]
    fn repeatable_and_sorted() {
        let cam = Vec3::new(0.1 * R, -0.3 * R, 0.45 * R);
        let a = select_cells(&frame(8), cam, &params(5_000));
        let b = select_cells(&frame(8), cam, &params(5_000));
        assert_eq!(a, b);
        assert!(is_sorted(&a));
        assert!(a.cells.iter().all(|k| k.frame_id == 7 && k.is_valid()));
    }

    #[test]
    fn max_cells_is_respected() {
        let cam = Vec3::new(-0.49 * R, -0.49 * R, -0.49 * R);
        let full = select_cells(&frame(6), cam, &params(100_000));
        assert!(full.cells.len() > 100);
        for max in [1, 7, 8, 15, 50, 100] {
            let s = select_cells(&frame(6), cam, &params(max));
            assert!(s.cells.len() <= max, "{} > {max}", s.cells.len());
            assert!(is_sorted(&s));
            assert!((coverage(&s) - 1.0).abs() < 1e-12);
        }
        assert_eq!(select_cells(&frame(6), cam, &params(1)).cells.len(), 1);
        assert!(select_cells(&frame(6), cam, &params(0)).cells.is_empty());
        // The near corner still gets the deepest cells the budget allows.
        let s = select_cells(&frame(6), cam, &params(50));
        let near = containing(&s, cam).depth;
        let far = containing(&s, Vec3::new(0.49 * R, 0.49 * R, 0.49 * R)).depth;
        assert!(near > far);
    }

    #[test]
    fn max_depth_zero_and_bad_fov() {
        let cam = Vec3::zero();
        let root = vec![CellKey::new(7, 0, 0, 0, 0).unwrap()];
        assert_eq!(select_cells(&frame(0), cam, &params(100)).cells, root);
        let mut p = params(100);
        p.vertical_fov_rad = 0.0;
        assert_eq!(select_cells(&frame(6), cam, &p).cells, root);
        p.vertical_fov_rad = f64::NAN;
        assert_eq!(select_cells(&frame(6), cam, &p).cells, root);
    }
}
