// SPDX-License-Identifier: GPL-3.0-or-later
//! Rendering (animated) scenes to images: the high-level entry point used by
//! the viewer and the C API.

use crate::anim::{Clip, Pose, WrapMode};
use crate::color::Rgba8;
use crate::math::{Aabb, Vec3, Viewport};
use crate::pipeline::{RenderState, Renderer};
use crate::raster::{CullMode, Rect};
use crate::scene::{Camera, NodeId, Scene};
use crate::shading::{DrawParams, Lighting, ShadeError, ShadingModel, draw_nodes};
use crate::target::{ColorBuffer, Framebuffer, SizeError, downsample_rect};

/// Where the camera comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum CameraSource {
    /// A fixed camera.
    Fixed(Camera),
    /// The camera at a scene node (follows the node's animation).
    Node(NodeId),
    /// A camera framing the whole scene (over the whole clip, if animated)
    /// from `direction`, with
    /// the given vertical field of view (or orthographic for `None`).
    Framed {
        /// Direction from the camera towards the scene.
        direction: Vec3,
        /// Vertical field of view in radians, or `None` for orthographic.
        fov_y: Option<f32>,
    },
}

/// How frames are rendered.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSettings {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Camera.
    pub camera: CameraSource,
    /// Shading model.
    pub shading: ShadingModel,
    /// Lights (for lit shading models).
    pub lighting: Lighting,
    /// Face culling (double-sided materials are never culled).
    pub cull: CullMode,
    /// Background colour.
    pub background: Rgba8,
    /// Supersampling anti-aliasing: samples per axis (1 = off, up to
    /// [`MAX_ANTIALIAS`]). Rendering cost grows with its square.
    pub antialias: u32,
    /// Rendering threads (see [`Renderer::with_threads`]); takes effect with
    /// the `parallel` feature. The output does not depend on it.
    pub threads: usize,
}

/// Largest supported [`FrameSettings::antialias`].
pub const MAX_ANTIALIAS: u32 = 8;

impl FrameSettings {
    /// Settings for a `width × height` image framing the scene from the
    /// front right, slightly above, with unlit shading and back-face culling
    /// on an opaque black background.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            camera: CameraSource::Framed {
                direction: Vec3::new(-0.6, -0.45, -1.0),
                fov_y: Some(0.7),
            },
            shading: ShadingModel::Unlit,
            lighting: Lighting::default(),
            cull: CullMode::Back,
            background: Rgba8::BLACK,
            antialias: 1,
            threads: 1,
        }
    }
}

/// Rendering a frame failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// The image size is not supported.
    #[error(transparent)]
    Size(#[from] SizeError),
    /// [`FrameSettings::antialias`] is outside `1..=MAX_ANTIALIAS`.
    #[error("antialias factor {0} is outside 1..={MAX_ANTIALIAS}")]
    Antialias(u32),
    /// `CameraSource::Node` names a node without a camera.
    #[error("node {0:?} has no camera")]
    NoCamera(NodeId),
    /// Drawing failed (e.g. a lit model on a mesh without normals).
    #[error(transparent)]
    Shade(#[from] ShadeError),
}

/// The playback time of frame `index` at `fps` frames per second starting
/// at `start`, computed from the integer index so it never drifts.
#[must_use]
pub fn frame_time(start: f64, index: u64, fps: f64) -> f64 {
    // Frame indices stay far below 2^53, where u64 -> f64 is exact.
    #[allow(clippy::cast_precision_loss)]
    let i = index as f64;
    start + i / fps
}

/// Number of frames needed to cover `duration` seconds at `fps`: the
/// product rounded to the nearest integer (so 3.3333 s at 30 fps is 100
/// frames), and at least one.
#[must_use]
pub fn frame_count(duration: f64, fps: f64) -> u64 {
    let n = (duration * fps).round();
    // Clamped to a sane, exactly representable range before converting.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = if n.is_finite() {
        n.clamp(1.0, 1e15) as u64
    } else {
        1
    };
    n
}

/// Poses sampled across a clip to find the space it moves through.
const FRAMING_SAMPLES: u16 = 64;
/// Relative margin for motion between the samples.
const FRAMING_MARGIN: f32 = 0.02;

/// Bounds of the model in `scene` (not its scenery) over the whole of
/// `clip` (or at rest): the union of the bounds of evenly sampled poses,
/// grown by a small margin, so a framing camera keeps moving models in view.
pub(crate) fn animated_bounds(scene: &Scene, clip: Option<&Clip>) -> Aabb {
    let Some(clip) = clip.filter(|c| c.duration() > 0.0) else {
        return scene.model_bounds_with(&scene.world_matrices());
    };
    let n = f32::from(FRAMING_SAMPLES);
    let bounds = (0..=FRAMING_SAMPLES)
        .map(|i| clip.start() + clip.duration() * f32::from(i) / n)
        .map(|t| scene.model_bounds_with(&Pose::evaluate(scene, clip, t).world(scene)))
        .fold(Aabb::EMPTY, Aabb::union);
    if bounds.is_empty() {
        return bounds;
    }
    let grow = bounds.size() * FRAMING_MARGIN * 0.5;
    Aabb::new(bounds.min - grow, bounds.max + grow)
}

/// Renders a scene, optionally animated by a clip, at arbitrary times.
#[derive(Debug)]
pub struct AnimationRenderer<'a> {
    scene: &'a Scene,
    clip: Option<(&'a Clip, WrapMode)>,
    settings: FrameSettings,
    /// The framing camera, computed once from the bounds over the whole clip
    /// so that it neither jitters nor loses the moving model.
    framed: Option<Camera>,
    renderer: Renderer,
    /// Render target, `antialias` times the output size per axis.
    target: Framebuffer,
    /// The downsampled image when anti-aliasing.
    resolved: Option<ColorBuffer>,
    /// Per node: fixed scenery, drawn as background. Scenery that the clip
    /// animates, or seen through an animated camera node, moves and is
    /// drawn like the model.
    fixed: Vec<bool>,
    /// Target pixels the moving geometry of the last frame may have drawn
    /// on; everything else shows just the background and fixed scenery (in
    /// colour and depth). `None` before the first frame (or after a failed
    /// one): the whole target must be redrawn.
    drawn: Option<Rect>,
    /// Output pixels the last frame changed; see [`AnimationRenderer::damage`].
    damage: Rect,
}

impl<'a> AnimationRenderer<'a> {
    /// A renderer for `scene`, animated by `clip` (played with `wrap`) if
    /// given.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Size`] for unsupported image sizes and
    /// [`RenderError::NoCamera`] if the camera node has no camera.
    pub fn new(
        scene: &'a Scene,
        clip: Option<(&'a Clip, WrapMode)>,
        settings: FrameSettings,
    ) -> Result<Self, RenderError> {
        let k = settings.antialias;
        if !(1..=MAX_ANTIALIAS).contains(&k) {
            return Err(RenderError::Antialias(k));
        }
        let scaled = |v: u32| {
            v.checked_mul(k).ok_or(SizeError {
                width: settings.width,
                height: settings.height,
            })
        };
        let target = Framebuffer::new(
            scaled(settings.width)?,
            scaled(settings.height)?,
            settings.background,
        )?;
        let resolved = (k > 1)
            .then(|| ColorBuffer::new(settings.width, settings.height, settings.background))
            .transpose()?;
        if let CameraSource::Node(node) = settings.camera
            && scene.nodes().get(node.0).is_none_or(|n| n.camera.is_none())
        {
            return Err(RenderError::NoCamera(node));
        }
        #[allow(clippy::cast_precision_loss)]
        let aspect = settings.width.max(1) as f32 / settings.height.max(1) as f32;
        // Framing shows the model; the near and far planes also enclose the
        // scenery so that it is not cut off.
        let framed = match settings.camera {
            CameraSource::Framed { direction, fov_y } => Some(
                Camera::framing(
                    animated_bounds(scene, clip.map(|(c, _)| c)),
                    direction,
                    Vec3::Y,
                    aspect,
                    fov_y,
                )
                .enclosing(scene.bounds()),
            ),
            _ => None,
        };
        let animated: Vec<NodeId> = clip
            .map(|(c, _)| c.channels.iter().map(|ch| ch.target).collect())
            .unwrap_or_default();
        let moves = |id: NodeId| {
            let mut node = Some(id);
            while let Some(n) = node {
                if animated.contains(&n) {
                    return true;
                }
                node = scene.nodes()[n.0].parent();
            }
            false
        };
        let fixed_camera = !matches!(settings.camera, CameraSource::Node(_));
        let fixed = (0..scene.nodes().len())
            .map(|i| fixed_camera && scene.is_scenery(NodeId(i)) && !moves(NodeId(i)))
            .collect();
        let renderer = Renderer::with_threads(settings.threads);
        Ok(Self {
            fixed,
            drawn: None,
            damage: Rect::default(),
            resolved,
            scene,
            clip,
            settings,
            framed,
            renderer,
            target,
        })
    }

    /// The last frame rendered by [`render_at`](Self::render_at) (the
    /// background before the first).
    #[must_use]
    pub fn frame(&self) -> &ColorBuffer {
        self.resolved.as_ref().unwrap_or(&self.target.color)
    }

    /// The output pixels that the last [`render_at`](Self::render_at)
    /// changed: every pixel outside is identical to the frame before. The
    /// first frame (and the one after a failed frame) is entirely damaged.
    /// Presenting only this rectangle saves copying unchanged pixels.
    #[must_use]
    pub fn damage(&self) -> Rect {
        self.damage
    }

    /// The camera computed for [`CameraSource::Framed`] (`None` for other
    /// camera sources).
    #[must_use]
    pub fn framed_camera(&self) -> Option<Camera> {
        self.framed
    }

    /// The settings in use.
    #[must_use]
    pub fn settings(&self) -> &FrameSettings {
        &self.settings
    }

    /// Renders the frame at playback time `t` (seconds).
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Shade`] if drawing fails.
    pub fn render_at(&mut self, t: f64) -> Result<&ColorBuffer, RenderError> {
        let pose = match self.clip {
            Some((clip, wrap)) => Pose::evaluate(self.scene, clip, clip.local_time(t, wrap)),
            None => Pose::rest(self.scene),
        };
        let world = pose.world(self.scene);
        let camera = match self.settings.camera {
            CameraSource::Fixed(c) => c,
            CameraSource::Node(node) => self
                .scene
                .camera(node, &world)
                .ok_or(RenderError::NoCamera(node))?,
            CameraSource::Framed { .. } => self.framed.expect("computed in new"),
        };
        let s = &self.settings;
        // Outside what the previous frame's moving geometry drew, the target
        // still holds the background and fixed scenery: restore just that
        // area (clear it, redraw the scenery within it), then draw the
        // moving geometry, which depth-tests against the scenery everywhere.
        let full = Rect::from_size(self.target.width(), self.target.height());
        let previous = self.drawn.take();
        let restore = previous.unwrap_or(full);
        self.target.clear_rect(restore, s.background);
        let params = DrawParams {
            camera: &camera,
            lighting: &s.lighting,
            model: s.shading,
            materials: &self.scene.materials,
        };
        let mut state = RenderState::new(Viewport::new(self.target.width(), self.target.height()));
        state.cull = s.cull;
        let fixed = &self.fixed;
        if fixed.contains(&true) && !restore.is_empty() {
            let scenery = RenderState {
                scissor: Some(restore),
                ..state
            };
            draw_nodes(
                &mut self.renderer,
                &mut self.target,
                &scenery,
                &params,
                self.scene,
                &world,
                |id| fixed[id.0],
            )?;
        }
        let stats = draw_nodes(
            &mut self.renderer,
            &mut self.target,
            &state,
            &params,
            self.scene,
            &world,
            |id| !fixed[id.0],
        )?;
        self.drawn = Some(stats.bounds);
        let changed = previous.map_or(full, |rect| rect.union(&stats.bounds));
        // In output pixels: every block that overlaps a changed target pixel.
        let k = s.antialias;
        self.damage = Rect {
            x0: changed.x0 / k,
            y0: changed.y0 / k,
            x1: changed.x1.div_ceil(k),
            y1: changed.y1.div_ceil(k),
        };
        match &mut self.resolved {
            Some(out) => {
                downsample_rect(&self.target.color, k, out, self.damage);
                Ok(out)
            }
            None => Ok(&self.target.color),
        }
    }

    /// Renders `count` frames at `fps` starting at `start` seconds (no
    /// frames unless `fps` is positive and finite).
    pub fn frames(
        &mut self,
        start: f64,
        fps: f64,
        count: u64,
    ) -> impl Iterator<Item = Result<ColorBuffer, RenderError>> + '_ {
        let count = if fps.is_finite() && fps > 0.0 {
            count
        } else {
            0
        };
        (0..count).map(move |i| self.render_at(frame_time(start, i, fps)).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::primitives::cube;
    use crate::scene::{LocalTransform, Node, Projection};

    /// A cube wrapped in a root node, spinning once every 4 s.
    fn spinning_cube() -> (Scene, Clip) {
        let mut scene = Scene::new();
        let mesh = scene.add_mesh(cube(1.0));
        scene
            .add_node(
                None,
                Node::new("cube", LocalTransform::default()).with_mesh(mesh),
            )
            .unwrap();
        let (scene, root) = scene.wrapped_in_root(Node::new("spin", LocalTransform::default()));
        (scene, Clip::turntable(root, Vec3::Y, 4.0))
    }

    #[test]
    fn frame_counts_and_times() {
        assert_eq!(frame_count(10.0 / 3.0, 30.0), 100);
        assert_eq!(frame_count(4.0, 30.0), 120);
        assert_eq!(frame_count(0.0, 30.0), 1);
        assert_eq!(frame_count(f64::NAN, 30.0), 1);
        assert_eq!(frame_time(1.0, 15, 30.0), 1.5);
        // Indexed times do not accumulate rounding error like repeated sums.
        let summed = (0..999).fold(0.0f64, |t, _| t + 1.0 / 30.0);
        assert_eq!(frame_time(0.0, 999, 30.0), 999.0 / 30.0);
        assert_ne!(summed, 999.0 / 30.0, "control: summing drifts");
    }

    #[test]
    fn rendering_is_deterministic_and_animated() {
        let (scene, clip) = spinning_cube();
        let mut a = AnimationRenderer::new(
            &scene,
            Some((&clip, WrapMode::Loop)),
            FrameSettings::new(32, 24),
        )
        .unwrap();
        let mut b = AnimationRenderer::new(
            &scene,
            Some((&clip, WrapMode::Loop)),
            FrameSettings::new(32, 24),
        )
        .unwrap();
        let frames_a: Vec<_> = a.frames(0.0, 2.0, 5).collect::<Result<_, _>>().unwrap();
        let frames_b: Vec<_> = b.frames(0.0, 2.0, 5).collect::<Result<_, _>>().unwrap();
        assert_eq!(frames_a, frames_b, "deterministic");
        assert_eq!((frames_a[0].width(), frames_a[0].height()), (32, 24));
        assert_ne!(
            frames_a[0], frames_a[1],
            "half a second apart, the cube has turned"
        );
        // A quarter turn of a cube looks the same; a full loop is identical.
        assert_eq!(
            frames_a[0], frames_a[4],
            "t = 0 and t = 2 s (half turn) of a symmetric cube"
        );
        assert_eq!(a.render_at(4.0).unwrap(), &frames_a[0], "looped");
    }

    #[test]
    fn camera_sources() {
        let (mut scene, _) = spinning_cube();
        let projection = Projection::Perspective {
            fov_y: 1.0,
            z_near: 0.1,
            z_far: 50.0,
        };
        let cam_t = LocalTransform::Matrix(
            Camera::look_at(Vec3::new(0.0, 0.0, 6.0), Vec3::ZERO, Vec3::Y, projection).world,
        );
        let cam = scene
            .add_node(None, Node::new("cam", cam_t).with_camera(projection))
            .unwrap();
        let fixed = Camera::look_at(Vec3::new(0.0, 0.0, 6.0), Vec3::ZERO, Vec3::Y, projection);
        let mut by_node = AnimationRenderer::new(
            &scene,
            None,
            FrameSettings {
                camera: CameraSource::Node(cam),
                ..FrameSettings::new(16, 16)
            },
        )
        .unwrap();
        let mut by_fixed = AnimationRenderer::new(
            &scene,
            None,
            FrameSettings {
                camera: CameraSource::Fixed(fixed),
                ..FrameSettings::new(16, 16)
            },
        )
        .unwrap();
        assert_eq!(
            by_node.render_at(0.0).unwrap().clone(),
            *by_fixed.render_at(0.0).unwrap()
        );
        let err = AnimationRenderer::new(
            &scene,
            None,
            FrameSettings {
                camera: CameraSource::Node(NodeId(0)),
                ..FrameSettings::new(16, 16)
            },
        )
        .unwrap_err();
        assert_eq!(err, RenderError::NoCamera(NodeId(0)));
        assert!(matches!(
            AnimationRenderer::new(&scene, None, FrameSettings::new(1 << 20, 1)),
            Err(RenderError::Size(_))
        ));
    }

    #[test]
    fn framing_covers_the_whole_animation() {
        use crate::scene::Transform;
        // A cube bouncing 10 units up: framed at rest it would leave the view.
        let mut scene = Scene::new();
        let mesh = scene.add_mesh(cube(1.0));
        let node = scene
            .add_node(
                None,
                Node::new("cube", LocalTransform::Trs(Transform::IDENTITY)).with_mesh(mesh),
            )
            .unwrap();
        let clip = Clip::bounce(node, Vec3::ZERO, Vec3::Y * 10.0, 2.0);
        let bounds = animated_bounds(&scene, Some(&clip));
        assert!(bounds.max.y >= 11.0 && bounds.min.y <= -1.0, "{bounds:?}");
        let mut r = AnimationRenderer::new(
            &scene,
            Some((&clip, WrapMode::Loop)),
            FrameSettings::new(32, 32),
        )
        .unwrap();
        for t in [0.0, 0.5, 1.0, 1.5] {
            let visible = r
                .render_at(t)
                .unwrap()
                .pixels()
                .iter()
                .filter(|p| **p != Rgba8::BLACK)
                .count();
            assert!(visible > 4, "t = {t}: the cube left the frame");
        }
        assert_eq!(r.frames(0.0, 0.0, 10).count(), 0, "fps must be positive");
    }

    #[test]
    fn antialiasing_softens_edges_only() {
        let (scene, _) = spinning_cube();
        let render = |antialias| {
            let mut r = AnimationRenderer::new(
                &scene,
                None,
                FrameSettings {
                    antialias,
                    ..FrameSettings::new(48, 48)
                },
            )
            .unwrap();
            r.render_at(0.0).unwrap().clone()
        };
        let (hard, soft) = (render(1), render(4));
        let distinct = |img: &ColorBuffer| {
            img.pixels()
                .iter()
                .map(|p| p.to_array())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        };
        assert_eq!(distinct(&hard), 2, "unlit white cube on black");
        assert!(distinct(&soft) > 4, "edges get intermediate values");
        // Away from edges the images agree.
        assert_eq!(hard.get(24, 24), soft.get(24, 24));
        assert_eq!(hard.get(0, 0), soft.get(0, 0));
        assert_eq!(
            AnimationRenderer::new(
                &scene,
                None,
                FrameSettings {
                    antialias: 0,
                    ..FrameSettings::new(8, 8)
                }
            )
            .unwrap_err(),
            RenderError::Antialias(0)
        );
        assert!(matches!(
            AnimationRenderer::new(
                &scene,
                None,
                FrameSettings {
                    antialias: 8,
                    ..FrameSettings::new(4096, 8)
                }
            ),
            Err(RenderError::Size(_))
        ));
    }

    #[test]
    fn thread_count_does_not_change_frames() {
        let (scene, clip) = spinning_cube();
        let frame = |threads| {
            let settings = FrameSettings {
                threads,
                antialias: 2,
                ..FrameSettings::new(40, 30)
            };
            AnimationRenderer::new(&scene, Some((&clip, WrapMode::Loop)), settings)
                .unwrap()
                .render_at(0.7)
                .unwrap()
                .clone()
        };
        let serial = frame(1);
        assert_eq!(frame(4), serial);
        assert_eq!(frame(0), serial, "0 means one thread");
    }

    #[test]
    fn damage_covers_every_change_and_less_than_the_frame() {
        let (scene, clip) = spinning_cube();
        for antialias in [1, 2, 3] {
            let settings = || FrameSettings {
                antialias,
                ..FrameSettings::new(61, 37)
            };
            let mut incremental =
                AnimationRenderer::new(&scene, Some((&clip, WrapMode::Loop)), settings()).unwrap();
            let mut shown = ColorBuffer::new(61, 37, Rgba8::new(1, 2, 3, 4)).unwrap();
            let mut partial = 0;
            for i in 0..40 {
                let t = f64::from(i) * 0.37;
                let image = incremental.render_at(t).unwrap().clone();
                let damage = incremental.damage();
                if i == 0 {
                    assert_eq!(
                        damage,
                        Rect::from_size(61, 37),
                        "the first frame is fully damaged"
                    );
                }
                partial += usize::from(damage.area() < 61 * 37);
                downsample_rect(&image, 1, &mut shown, damage);
                let fresh =
                    AnimationRenderer::new(&scene, Some((&clip, WrapMode::Loop)), settings())
                        .unwrap()
                        .render_at(t)
                        .unwrap()
                        .clone();
                assert_eq!(shown, fresh, "frame {i}, antialias {antialias}");
            }
            assert!(
                partial > 30,
                "damage is usually smaller than the frame ({partial})"
            );
        }
    }

    #[test]
    fn fixed_scenery_is_drawn_once_and_restored_only_where_the_model_moved() {
        let model = crate::Model {
            scene: spinning_cube().0,
            clips: vec![spinning_cube().1],
            warnings: Vec::new(),
        }
        .with_floor(Rgba8::new(90, 140, 60, 255), 3.0);
        let clip = (&model.clips[0], WrapMode::Loop);
        for antialias in [1, 2, 3] {
            let settings = || FrameSettings {
                antialias,
                background: Rgba8::new(135, 206, 235, 255),
                ..FrameSettings::new(64, 40)
            };
            let mut incremental =
                AnimationRenderer::new(&model.scene, Some(clip), settings()).unwrap();
            assert_eq!(
                incremental.fixed.iter().filter(|&&f| f).count(),
                1,
                "the floor is fixed"
            );
            let mut shown = ColorBuffer::new(64, 40, Rgba8::TRANSPARENT).unwrap();
            for i in 0..40 {
                let t = f64::from(i) * 0.29;
                let image = incremental.render_at(t).unwrap().clone();
                let damage = incremental.damage();
                downsample_rect(&image, 1, &mut shown, damage);
                let fresh = AnimationRenderer::new(&model.scene, Some(clip), settings())
                    .unwrap()
                    .render_at(t)
                    .unwrap()
                    .clone();
                assert_eq!(shown, fresh, "frame {i}, antialias {antialias}");
                if i == 0 {
                    let floor_pixels = fresh.pixels().iter().filter(|p| p.b < 100).count();
                    assert!(
                        floor_pixels > 64 * 40 / 4,
                        "the floor is visible ({floor_pixels})"
                    );
                } else {
                    assert!(
                        damage.area() < 64 * 40 / 2,
                        "frame {i}: the floor is not redrawn ({damage:?})"
                    );
                }
            }
        }
    }
}
