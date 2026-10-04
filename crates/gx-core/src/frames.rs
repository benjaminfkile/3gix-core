//! Reference frames with a floating origin: the tree of frames, the current
//! state of every frame, and the transforms between frames.
//!
//! Implements the first numerical device of `space-model.md` section 5 and
//! the simulation time of section 6. In the words of section 5:
//!
//! > **Reference frames with a floating origin.** One coordinate system
//! > cannot hold an outer orbit and a one-meter rock in the same double.
//! > Every frame has a parent frame and a state vector relative to it. Matter
//! > cells are addressed in their frame. The camera is positioned relative to
//! > the frame it is nearest, and re-parents when it leaves one frame's region
//! > for another. The GPU only ever sees camera-relative single-precision
//! > coordinates. Frames are a precision device; gravity still acts between
//! > all frames regardless of the tree.
//!
//! A [`FrameSystem`] holds a [`FrameTree`], the current [`FrameState`] of
//! every frame, and the current time in seconds of Barycentric Dynamical Time
//! since J2000. [`crate::integrate`] advances it; this module only reads and
//! transforms.
//!
//! # Coordinates
//!
//! *Root coordinates* have their origin at the root frame's origin at the
//! epoch and the axes in which every state vector of the registry is given
//! (`matter-format.md` section 5.2). They are inertial.
//!
//! - A frame's `position` and `velocity` are the offset of its origin from
//!   its parent's origin, and the rate of change of that offset, expressed in
//!   root axes. Translations therefore add along the parent chain with no
//!   rotation: [`FrameSystem::root_position`] is the plain sum from the root
//!   down.
//! - A frame's `orientation` gives its axes relative to its parent's axes.
//!   The root's orientation gives its axes relative to root axes. Rotations
//!   compose along the chain: [`FrameSystem::root_orientation`].
//! - A frame's `angular_velocity` is in radians per second about the frame's
//!   own axes.
//! - A point "in a frame" is relative to that frame's origin, in that frame's
//!   axes. This is how matter cells are addressed.
//!
//! The root frame's position and velocity are zero at the epoch. A massive
//! root moves under gravity like every other massive frame, so they may not
//! stay zero; root coordinates do not move with it.
//!
//! Every function here iterates frames by ascending `frame_id` or along the
//! parent chain in a fixed order, and uses only `+`, `-`, `*`, `/`, and
//! `sqrt`, so results are bitwise identical on every platform.

use crate::registry::FrameTree;
use crate::units::{Quat, Seconds, Vec3};

/// The state of one frame relative to its parent (see the module docs for the
/// axes of each field).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameState {
    /// Offset of the frame origin from the parent origin, meters, root axes.
    pub position: Vec3,
    /// Rate of change of `position`, meters per second, root axes.
    pub velocity: Vec3,
    /// Unit quaternion giving the frame axes relative to the parent axes.
    pub orientation: Quat,
    /// Angular velocity in radians per second about the frame axes.
    pub angular_velocity: Vec3,
}

/// A frame tree with the current state of every frame and the current time.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSystem {
    tree: FrameTree,
    /// One state per frame, in the order of `tree.frames()` (ascending id).
    states: Vec<FrameState>,
    /// For each frame, the index of its parent; `None` for the root.
    parents: Vec<Option<usize>>,
    time: Seconds,
}

impl FrameSystem {
    /// Builds a system at the tree's epoch, with every frame in the state
    /// its registry declares.
    pub fn from_tree(tree: FrameTree) -> FrameSystem {
        let frames = tree.frames();
        let states = frames
            .iter()
            .map(|f| FrameState {
                position: f.position,
                velocity: f.velocity,
                orientation: f.orientation,
                angular_velocity: f.angular_velocity,
            })
            .collect();
        let parents = frames
            .iter()
            .map(|f| {
                (!f.is_root()).then(|| {
                    frames
                        .binary_search_by_key(&f.parent_frame_id, |p| p.frame_id)
                        .expect("a frame tree holds every parent")
                })
            })
            .collect();
        let time = tree.epoch();
        FrameSystem {
            tree,
            states,
            parents,
            time,
        }
    }

    /// Current time in seconds of Barycentric Dynamical Time since J2000.
    pub fn time(&self) -> Seconds {
        self.time
    }

    /// The frame tree.
    pub fn tree(&self) -> &FrameTree {
        &self.tree
    }

    /// The current state of the frame relative to its parent, or `None` if
    /// the id is not in the tree.
    pub fn state(&self, frame_id: u64) -> Option<&FrameState> {
        self.index(frame_id).map(|i| &self.states[i])
    }

    /// Position of the frame origin in root coordinates: the sum of the
    /// positions along the parent chain, added from the root down.
    ///
    /// # Panics
    ///
    /// Panics if `frame_id` is not in the tree.
    pub fn root_position(&self, frame_id: u64) -> Vec3 {
        self.chain(frame_id)
            .iter()
            .fold(Vec3::zero(), |acc, &i| acc + self.states[i].position)
    }

    /// Velocity of the frame origin in root coordinates: the sum of the
    /// velocities along the parent chain, added from the root down.
    ///
    /// # Panics
    ///
    /// Panics if `frame_id` is not in the tree.
    pub fn root_velocity(&self, frame_id: u64) -> Vec3 {
        self.chain(frame_id)
            .iter()
            .fold(Vec3::zero(), |acc, &i| acc + self.states[i].velocity)
    }

    /// The frame axes relative to root axes: the orientations along the
    /// parent chain composed from the root down, `q_root * ... * q_frame`.
    ///
    /// # Panics
    ///
    /// Panics if `frame_id` is not in the tree.
    pub fn root_orientation(&self, frame_id: u64) -> Quat {
        self.chain(frame_id)
            .iter()
            .fold(Quat::identity(), |acc, &i| acc * self.states[i].orientation)
    }

    /// Converts a point in root coordinates into the frame: relative to the
    /// frame origin, in the frame axes.
    ///
    /// # Panics
    ///
    /// Panics if `frame_id` is not in the tree.
    pub fn to_frame(&self, point_in_root: Vec3, frame_id: u64) -> Vec3 {
        let q = self.root_orientation(frame_id);
        q.conjugate()
            .rotate(point_in_root - self.root_position(frame_id))
    }

    /// Converts a point in the frame (relative to its origin, in its axes)
    /// into root coordinates. The inverse of [`FrameSystem::to_frame`].
    ///
    /// # Panics
    ///
    /// Panics if `frame_id` is not in the tree.
    pub fn from_frame(&self, point_in_frame: Vec3, frame_id: u64) -> Vec3 {
        self.root_position(frame_id) + self.root_orientation(frame_id).rotate(point_in_frame)
    }

    /// The vector from `origin` (a point in `origin_frame`) to `point` (a
    /// point in `in_frame`), in root axes.
    ///
    /// This is the floating-origin primitive. The result is computed without
    /// passing through root coordinates: both points are carried up the tree
    /// only to the nearest common ancestor of the two frames, so the large
    /// offsets above that ancestor never enter the arithmetic. A caller
    /// passes a camera position and gets a small vector it can cast to `f32`.
    ///
    /// # Panics
    ///
    /// Panics if either frame id is not in the tree.
    pub fn relative(&self, point: Vec3, in_frame: u64, origin: Vec3, origin_frame: u64) -> Vec3 {
        let a = self.chain(in_frame);
        let b = self.chain(origin_frame);
        // Both chains start at the root; they share a prefix up to the
        // nearest common ancestor.
        let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let to_point = self.offset_below(&a, shared, point);
        let to_origin = self.offset_below(&b, shared, origin);
        to_point - to_origin
    }

    /// The frame whose origin is nearest to the point, in root coordinates.
    /// Ties go to the lower `frame_id`.
    pub fn nearest_frame(&self, point_in_root: Vec3) -> u64 {
        let frames = self.tree.frames();
        let mut best = frames[0].frame_id;
        let mut best_d = f64::INFINITY;
        for f in frames {
            let d = (self.root_position(f.frame_id) - point_in_root).length_squared();
            if d < best_d {
                best = f.frame_id;
                best_d = d;
            }
        }
        best
    }

    /// Index of the frame in `tree.frames()` and `states`.
    fn index(&self, frame_id: u64) -> Option<usize> {
        self.tree
            .frames()
            .binary_search_by_key(&frame_id, |f| f.frame_id)
            .ok()
    }

    /// Indices from the root down to the frame, both included.
    fn chain(&self, frame_id: u64) -> Vec<usize> {
        let mut at = Some(
            self.index(frame_id)
                .unwrap_or_else(|| panic!("frame_id {frame_id} is not in the frame tree")),
        );
        let mut chain = Vec::new();
        while let Some(i) = at {
            chain.push(i);
            at = self.parents[i];
        }
        chain.reverse();
        chain
    }

    /// The offset, in root axes, of a point in the last frame of `chain`
    /// from the origin of the frame at `chain[shared - 1]`: the positions of
    /// the frames below that ancestor, added from the top down, plus the
    /// point rotated into root axes.
    fn offset_below(&self, chain: &[usize], shared: usize, point: Vec3) -> Vec3 {
        let mut offset = Vec3::zero();
        let mut q = Quat::identity();
        for (k, &i) in chain.iter().enumerate() {
            q = q * self.states[i].orientation;
            if k >= shared {
                offset = offset + self.states[i].position;
            }
        }
        offset + q.rotate(point)
    }

    /// Every frame's state in ascending `frame_id` order, for the integrator.
    pub(crate) fn states_mut(&mut self) -> &mut [FrameState] {
        &mut self.states
    }

    /// Every frame's parent index, in ascending `frame_id` order.
    pub(crate) fn parent_indices(&self) -> &[Option<usize>] {
        &self.parents
    }

    /// Sets the current time.
    pub(crate) fn set_time(&mut self, time: Seconds) {
        self.time = time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Frame, Registry, ROOT_PARENT};
    use crate::units::{Kilograms, Meters};

    fn frame(id: u64, parent: u64, position: Vec3, orientation: Quat) -> Frame {
        Frame {
            frame_id: id,
            parent_frame_id: parent,
            root_extent: Meters::new(1.0e3),
            max_depth: 4,
            mass: Kilograms::new(0.0),
            position,
            velocity: position.scale(0.5),
            orientation,
            angular_velocity: Vec3::zero(),
        }
    }

    fn system() -> FrameSystem {
        let h = core::f64::consts::FRAC_1_SQRT_2;
        let quarter_z = Quat::new(0.0, 0.0, h, h);
        let quarter_x = Quat::new(h, 0.0, 0.0, h);
        let frames = vec![
            frame(1, ROOT_PARENT, Vec3::zero(), Quat::identity()),
            frame(5, 1, Vec3::new(100.0, 0.0, 0.0), quarter_z),
            frame(7, 5, Vec3::new(0.0, 10.0, 0.0), quarter_x),
            frame(3, 1, Vec3::new(0.0, 0.0, -50.0), Quat::identity()),
        ];
        let reg = Registry::new(Seconds::new(42.0), frames).unwrap();
        FrameSystem::from_tree(FrameTree::from_registries(&[reg]).unwrap())
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-12
    }

    #[test]
    fn epoch_states_and_time() {
        let s = system();
        assert_eq!(s.time(), Seconds::new(42.0));
        assert_eq!(s.state(7).unwrap().position, Vec3::new(0.0, 10.0, 0.0));
        assert!(s.state(2).is_none());
        assert_eq!(s.tree().root().frame_id, 1);
    }

    #[test]
    fn root_sums_and_orientation() {
        let s = system();
        assert_eq!(s.root_position(7), Vec3::new(100.0, 10.0, 0.0));
        assert_eq!(s.root_velocity(7), Vec3::new(50.0, 5.0, 0.0));
        assert_eq!(s.root_position(1), Vec3::zero());
        let q = s.root_orientation(7);
        // z quarter turn after x quarter turn: local y goes to x then z,
        // so local y maps to root z, rotated about z leaves z.
        assert!(close(
            q.rotate(Vec3::new(0.0, 1.0, 0.0)),
            Vec3::new(0.0, 0.0, 1.0)
        ));
        assert!(close(
            q.rotate(Vec3::new(1.0, 0.0, 0.0)),
            Vec3::new(0.0, 1.0, 0.0)
        ));
    }

    #[test]
    fn to_and_from_frame_round_trip() {
        let s = system();
        let p = Vec3::new(1.0, 2.0, 3.0);
        let root = s.from_frame(p, 7);
        assert!(close(s.to_frame(root, 7), p));
        assert!(close(s.from_frame(s.to_frame(p, 5), 5), p));
        assert!(close(
            s.from_frame(Vec3::zero(), 5),
            Vec3::new(100.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn relative_matches_root_difference() {
        let s = system();
        let p = Vec3::new(1.0, -2.0, 0.5);
        let o = Vec3::new(-3.0, 4.0, 2.0);
        for (a, b) in [(7, 5), (7, 3), (3, 7), (1, 7), (7, 7)] {
            let expect = s.from_frame(p, a) - s.from_frame(o, b);
            assert!(close(s.relative(p, a, o, b), expect), "{a} {b}");
        }
    }

    #[test]
    fn nearest_frame_ties_to_lower_id() {
        let s = system();
        assert_eq!(s.nearest_frame(Vec3::new(99.0, 9.0, 0.0)), 7);
        assert_eq!(s.nearest_frame(Vec3::new(1.0, 0.0, -1.0)), 1);
        // Equidistant from frame 1 (origin) and frame 3 (0, 0, -50).
        assert_eq!(s.nearest_frame(Vec3::new(0.0, 0.0, -25.0)), 1);
        // Equidistant from frame 5 (100, 0, 0) and frame 7 (100, 10, 0).
        assert_eq!(s.nearest_frame(Vec3::new(100.0, 5.0, 0.0)), 5);
    }
}
