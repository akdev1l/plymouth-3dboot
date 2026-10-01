// SPDX-License-Identifier: GPL-3.0-or-later
//! The viewer's application logic (independent of SDL, unit-testable).

use plymouth_3dboot::anim::{Clip, Pose, WrapMode};
use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::math::{Aabb, Vec3, Viewport};
use plymouth_3dboot::pipeline::{RenderState, Renderer};
use plymouth_3dboot::raster::CullMode;
use plymouth_3dboot::scene::{Camera, Scene};
use plymouth_3dboot::shading::{DrawParams, Lighting, ShadingModel, draw_scene};
use plymouth_3dboot::target::{ColorBuffer, Framebuffer};
use plymouth_3dboot_sdl::{App, Control, InputEvent};

/// Vertical field of view of the viewer camera (radians).
const FOV_Y: f32 = 0.7;
/// Radians of orbit per pixel of mouse drag.
const ORBIT_SPEED: f32 = 0.01;
/// Pitch limit, short of straight up/down where the up vector degenerates.
const MAX_PITCH: f32 = 1.5;

/// Viewer settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewerOptions {
    /// Shading model.
    pub shading: ShadingModel,
    /// Largest internal frame dimension; bigger windows are upscaled.
    pub max_resolution: u32,
    /// Background colour.
    pub background: Rgba8,
}

impl Default for ViewerOptions {
    fn default() -> Self {
        Self {
            shading: ShadingModel::Unlit,
            max_resolution: 720,
            background: Rgba8::new(0, 0, 0, 255),
        }
    }
}

/// Orbit-camera and playback state driven by input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    /// Rotation around the vertical axis (radians).
    pub yaw: f32,
    /// Elevation (radians, positive looks down from above).
    pub pitch: f32,
    /// Zoom factor: 1 frames the model, larger is closer.
    pub zoom: f32,
    /// Whether animation time is frozen.
    pub paused: bool,
    /// Playback speed multiplier.
    pub speed: f64,
    /// Animation time in seconds.
    pub time: f64,
}

impl Default for ViewState {
    /// The initial three-quarter view.
    fn default() -> Self {
        Self {
            yaw: 0.54,
            pitch: 0.35,
            zoom: 1.0,
            paused: false,
            speed: 1.0,
            time: 0.0,
        }
    }
}

impl ViewState {
    /// Applies one input event; returns `false` for [`InputEvent::Quit`].
    pub fn apply(&mut self, event: &InputEvent) -> bool {
        match *event {
            InputEvent::Quit => return false,
            InputEvent::TogglePause => self.paused = !self.paused,
            InputEvent::Reset => *self = Self::default(),
            InputEvent::Faster => self.speed = (self.speed * 2.0).min(16.0),
            InputEvent::Slower => self.speed = (self.speed / 2.0).max(1.0 / 16.0),
            InputEvent::Orbit { dx, dy } => {
                self.yaw -= dx * ORBIT_SPEED;
                self.pitch = (self.pitch + dy * ORBIT_SPEED).clamp(-MAX_PITCH, MAX_PITCH);
            }
            InputEvent::Zoom(steps) => {
                self.zoom = (self.zoom * libm::powf(1.1, steps)).clamp(0.1, 20.0)
            }
            _ => {}
        }
        true
    }

    /// Advances animation time by `dt` (unless paused).
    pub fn advance(&mut self, dt: f64) {
        if !self.paused {
            self.time += dt * self.speed;
        }
    }

    /// Direction from the camera towards the target.
    #[must_use]
    pub fn view_direction(&self) -> Vec3 {
        let (sy, cy) = (libm::sinf(self.yaw), libm::cosf(self.yaw));
        let (sp, cp) = (libm::sinf(self.pitch), libm::cosf(self.pitch));
        -Vec3::new(sy * cp, sp, cy * cp)
    }
}

/// The internal frame size for an output size: the output scaled down so
/// that neither side exceeds `max` (at least 1×1).
#[must_use]
pub fn frame_size((w, h): (u32, u32), max: u32) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    let largest = w.max(h);
    if largest <= max {
        return (w, h);
    }
    let scale = |v: u32| {
        u32::try_from(u64::from(v) * u64::from(max) / u64::from(largest))
            .unwrap_or(1)
            .max(1)
    };
    (scale(w), scale(h))
}

/// The model viewer.
pub struct Viewer {
    scene: Scene,
    clip: Option<Clip>,
    bounds: Aabb,
    options: ViewerOptions,
    lighting: Lighting,
    renderer: Renderer,
    target: Framebuffer,
    /// Current view and playback state.
    pub state: ViewState,
}

impl Viewer {
    /// A viewer showing `scene`, playing `clip` (looped) if given.
    #[must_use]
    pub fn new(scene: Scene, clip: Option<Clip>, options: ViewerOptions) -> Self {
        let bounds = scene.bounds();
        Self {
            scene,
            clip,
            bounds,
            options,
            lighting: Lighting::default(),
            renderer: Renderer::new(),
            target: Framebuffer::new(1, 1, options.background).expect("1x1"),
            state: ViewState::default(),
        }
    }

    /// The camera for the current view state and aspect ratio.
    #[must_use]
    pub fn camera(&self, aspect: f32) -> Camera {
        // Zooming in frames a smaller box around the same centre.
        let c = self.bounds.center();
        let half = self.bounds.size() * 0.5 / self.state.zoom;
        let zoomed = if self.bounds.is_empty() {
            self.bounds
        } else {
            Aabb::new(c - half, c + half)
        };
        Camera::framing(
            zoomed,
            self.state.view_direction(),
            Vec3::Y,
            aspect,
            Some(FOV_Y),
        )
    }
}

impl App for Viewer {
    fn update(&mut self, events: &[InputEvent], dt: f64) -> Control {
        for e in events {
            if !self.state.apply(e) {
                return Control::Quit;
            }
        }
        self.state.advance(dt);
        Control::Continue
    }

    fn render(&mut self, output: (u32, u32)) -> &ColorBuffer {
        let (w, h) = frame_size(output, self.options.max_resolution);
        if (self.target.width(), self.target.height()) != (w, h) {
            self.target =
                Framebuffer::new(w, h, self.options.background).expect("frame size is capped");
        }
        self.target.clear(self.options.background);
        #[allow(clippy::cast_precision_loss)]
        let camera = self.camera(w as f32 / h as f32);
        let params = DrawParams {
            camera: &camera,
            lighting: &self.lighting,
            model: self.options.shading,
            materials: &self.scene.materials,
        };
        let mut state = RenderState::new(Viewport::new(w, h));
        state.cull = CullMode::Back;
        let pose = match &self.clip {
            Some(clip) => Pose::evaluate(
                &self.scene,
                clip,
                clip.local_time(self.state.time, WrapMode::Loop),
            ),
            None => Pose::rest(&self.scene),
        };
        let world = pose.world(&self.scene);
        if let Err(e) = draw_scene(
            &mut self.renderer,
            &mut self.target,
            &state,
            &params,
            &self.scene,
            &world,
        ) {
            eprintln!("draw failed: {e}");
        }
        &self.target.color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_state_reacts_to_input() {
        let mut s = ViewState::default();
        assert!(s.apply(&InputEvent::TogglePause));
        assert!(s.paused);
        s.apply(&InputEvent::Faster);
        s.apply(&InputEvent::Faster);
        assert_eq!(s.speed, 4.0);
        s.apply(&InputEvent::Orbit {
            dx: 10.0,
            dy: 1000.0,
        });
        assert_eq!(s.pitch, MAX_PITCH, "pitch is clamped");
        assert!((s.yaw - (0.54 - 0.1)).abs() < 1e-6);
        s.apply(&InputEvent::Zoom(2.0));
        assert!((s.zoom - 1.21).abs() < 1e-5);
        s.apply(&InputEvent::Reset);
        assert_eq!(s, ViewState::default());
        assert!(!s.apply(&InputEvent::Quit));
    }

    #[test]
    fn time_advances_with_speed_unless_paused() {
        let mut s = ViewState {
            speed: 2.0,
            ..ViewState::default()
        };
        s.advance(0.5);
        assert_eq!(s.time, 1.0);
        s.paused = true;
        s.advance(0.5);
        assert_eq!(s.time, 1.0);
    }

    #[test]
    fn view_direction_is_unit_and_points_at_the_target() {
        let s = ViewState {
            yaw: 0.0,
            pitch: 0.0,
            ..ViewState::default()
        };
        assert!(s.view_direction().abs_diff_eq(Vec3::NEG_Z, 1e-6));
        let d = ViewState::default().view_direction();
        assert!(
            (d.length() - 1.0).abs() < 1e-6 && d.y < 0.0,
            "looks down from above: {d}"
        );
    }

    #[test]
    fn frame_size_is_capped_preserving_aspect() {
        assert_eq!(frame_size((640, 480), 720), (640, 480));
        assert_eq!(frame_size((1920, 1080), 720), (720, 405));
        assert_eq!(frame_size((1080, 1920), 720), (405, 720));
        assert_eq!(frame_size((0, 0), 720), (1, 1));
    }

    #[test]
    fn playback_follows_time_pause_and_speed() {
        use plymouth_3dboot::scene::{LocalTransform, Node};
        let (scene, root) = crate::model::embedded_n64()
            .unwrap()
            .wrapped_in_root(Node::new("spin", LocalTransform::default()));
        let clip = Clip::turntable(root, Vec3::Y, 4.0);
        let options = ViewerOptions {
            max_resolution: 48,
            ..ViewerOptions::default()
        };
        let mut v = Viewer::new(scene, Some(clip), options);
        let start = v.render((48, 48)).clone();
        v.update(&[], 0.5);
        let moved = v.render((48, 48)).clone();
        assert_ne!(start, moved, "the model turns over time");
        v.update(&[InputEvent::TogglePause], 0.5);
        assert_eq!(v.render((48, 48)), &moved, "paused");
        v.update(&[InputEvent::TogglePause, InputEvent::Faster], 0.0);
        v.update(&[], 1.75);
        // 0.5 s + 1.75 s at double speed = 4 s: a full turn.
        assert!((v.state.time - 4.0).abs() < 1e-9);
        assert_eq!(
            v.render((48, 48)),
            &start,
            "back to the start after one period"
        );
    }

    #[test]
    fn update_quits_and_renders_at_capped_size() {
        let mut v = Viewer::new(
            crate::model::embedded_n64().unwrap(),
            None,
            ViewerOptions {
                max_resolution: 64,
                ..ViewerOptions::default()
            },
        );
        assert_eq!(v.update(&[], 0.1), Control::Continue);
        let frame = v.render((128, 96));
        assert_eq!((frame.width(), frame.height()), (64, 48));
        assert!(
            frame
                .pixels()
                .iter()
                .any(|p| *p != Rgba8::new(0, 0, 0, 255)),
            "the model is visible"
        );
        assert_eq!(v.update(&[InputEvent::Quit], 0.1), Control::Quit);
    }
}
