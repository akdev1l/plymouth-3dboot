// SPDX-License-Identifier: GPL-3.0-or-later
//! Rendering (animated) scenes to images: the high-level entry point used by
//! the viewer and the C API.

use crate::anim::{Clip, Pose, WrapMode};
use crate::color::Rgba8;
use crate::math::{Vec3, Viewport};
use crate::pipeline::{RenderState, Renderer};
use crate::raster::CullMode;
use crate::scene::{Camera, NodeId, Scene};
use crate::shading::{DrawParams, Lighting, ShadeError, ShadingModel, draw_scene};
use crate::target::{ColorBuffer, Framebuffer, SizeError};

/// Where the camera comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraSource {
    /// A fixed camera.
    Fixed(Camera),
    /// The camera at a scene node (follows the node's animation).
    Node(NodeId),
    /// A camera framing the whole scene (at rest) from `direction`, with
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
}

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

/// Renders a scene, optionally animated by a clip, at arbitrary times.
#[derive(Debug)]
pub struct AnimationRenderer<'a> {
    scene: &'a Scene,
    clip: Option<(&'a Clip, WrapMode)>,
    settings: FrameSettings,
    /// The framing camera, computed once from the rest pose so that it does
    /// not jitter while the model moves.
    framed: Option<Camera>,
    renderer: Renderer,
    target: Framebuffer,
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
        let target = Framebuffer::new(settings.width, settings.height, settings.background)?;
        if let CameraSource::Node(node) = settings.camera
            && scene.nodes().get(node.0).is_none_or(|n| n.camera.is_none())
        {
            return Err(RenderError::NoCamera(node));
        }
        #[allow(clippy::cast_precision_loss)]
        let aspect = settings.width.max(1) as f32 / settings.height.max(1) as f32;
        let framed = match settings.camera {
            CameraSource::Framed { direction, fov_y } => Some(Camera::framing(
                scene.bounds(),
                direction,
                Vec3::Y,
                aspect,
                fov_y,
            )),
            _ => None,
        };
        Ok(Self {
            scene,
            clip,
            settings,
            framed,
            renderer: Renderer::new(),
            target,
        })
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
        self.target.clear(s.background);
        let params = DrawParams {
            camera: &camera,
            lighting: &s.lighting,
            model: s.shading,
            materials: &self.scene.materials,
        };
        let mut state = RenderState::new(Viewport::new(s.width, s.height));
        state.cull = s.cull;
        draw_scene(
            &mut self.renderer,
            &mut self.target,
            &state,
            &params,
            self.scene,
            &world,
        )?;
        Ok(&self.target.color)
    }

    /// Renders `count` frames at `fps` starting at `start` seconds.
    pub fn frames(
        &mut self,
        start: f64,
        fps: f64,
        count: u64,
    ) -> impl Iterator<Item = Result<ColorBuffer, RenderError>> + '_ {
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
}
