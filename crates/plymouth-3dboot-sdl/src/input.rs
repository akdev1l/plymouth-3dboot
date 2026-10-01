// SPDX-License-Identifier: GPL-3.0-or-later
//! Mapping SDL events to application input.

use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseWheelDirection;

/// Input delivered to [`crate::App::update`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum InputEvent {
    /// Close the application (window close, Escape or Q).
    Quit,
    /// Pause or resume animation (Space).
    TogglePause,
    /// Reset the view and animation (R).
    Reset,
    /// Play animation faster (+ or =).
    Faster,
    /// Play animation slower (-).
    Slower,
    /// Rotate the view: mouse movement in pixels while the left button is
    /// held.
    Orbit {
        /// Horizontal movement (+ is right).
        dx: f32,
        /// Vertical movement (+ is down).
        dy: f32,
    },
    /// Zoom: mouse-wheel steps (+ is towards the user's "scroll up", i.e. in).
    Zoom(f32),
}

/// Maps one SDL event to an [`InputEvent`], or `None` if it is not used.
///
/// Key repeats are ignored for toggles so holding a key does not flicker.
#[must_use]
pub fn map_event(event: &Event) -> Option<InputEvent> {
    match event {
        Event::Quit { .. } => Some(InputEvent::Quit),
        Event::KeyDown {
            keycode: Some(key),
            repeat,
            ..
        } => match key {
            Keycode::Escape | Keycode::Q => Some(InputEvent::Quit),
            Keycode::Space if !repeat => Some(InputEvent::TogglePause),
            Keycode::R if !repeat => Some(InputEvent::Reset),
            Keycode::Plus | Keycode::Equals | Keycode::KpPlus => Some(InputEvent::Faster),
            Keycode::Minus | Keycode::KpMinus => Some(InputEvent::Slower),
            _ => None,
        },
        Event::MouseMotion {
            mousestate,
            xrel,
            yrel,
            ..
        } if mousestate.left() => Some(InputEvent::Orbit {
            dx: *xrel,
            dy: *yrel,
        }),
        Event::MouseWheel { y, direction, .. } => {
            let sign = if matches!(direction, MouseWheelDirection::Flipped) {
                -1.0
            } else {
                1.0
            };
            (*y != 0.0).then_some(InputEvent::Zoom(y * sign))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl3::keyboard::Mod;
    use sdl3::mouse::MouseState;

    fn key(keycode: Keycode, repeat: bool) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 1,
            keycode: Some(keycode),
            scancode: None,
            keymod: Mod::NOMOD,
            repeat,
            which: 0,
            raw: 0,
        }
    }

    fn motion(buttons: u32, xrel: f32, yrel: f32) -> Event {
        Event::MouseMotion {
            timestamp: 0,
            window_id: 1,
            which: 0,
            mousestate: MouseState::from_sdl_state(buttons),
            x: 10.0,
            y: 10.0,
            xrel,
            yrel,
        }
    }

    fn wheel(y: f32, direction: MouseWheelDirection) -> Event {
        Event::MouseWheel {
            timestamp: 0,
            window_id: 1,
            which: 0,
            x: 0.0,
            y,
            direction,
            mouse_x: 0.0,
            mouse_y: 0.0,
            integer_x: 0,
            integer_y: 0,
        }
    }

    #[test]
    fn quit_sources() {
        assert_eq!(
            map_event(&Event::Quit { timestamp: 0 }),
            Some(InputEvent::Quit)
        );
        assert_eq!(
            map_event(&key(Keycode::Escape, false)),
            Some(InputEvent::Quit)
        );
        assert_eq!(map_event(&key(Keycode::Q, true)), Some(InputEvent::Quit));
    }

    #[test]
    fn toggles_ignore_key_repeat() {
        assert_eq!(
            map_event(&key(Keycode::Space, false)),
            Some(InputEvent::TogglePause)
        );
        assert_eq!(map_event(&key(Keycode::Space, true)), None);
        assert_eq!(map_event(&key(Keycode::R, false)), Some(InputEvent::Reset));
        assert_eq!(map_event(&key(Keycode::R, true)), None);
    }

    #[test]
    fn speed_keys_repeat() {
        for k in [Keycode::Plus, Keycode::Equals, Keycode::KpPlus] {
            assert_eq!(map_event(&key(k, true)), Some(InputEvent::Faster));
        }
        for k in [Keycode::Minus, Keycode::KpMinus] {
            assert_eq!(map_event(&key(k, false)), Some(InputEvent::Slower));
        }
        assert_eq!(map_event(&key(Keycode::A, false)), None);
    }

    #[test]
    fn dragging_with_the_left_button_orbits() {
        // SDL_BUTTON_LMASK = 1, SDL_BUTTON_RMASK = 4.
        assert_eq!(
            map_event(&motion(1, 3.0, -2.0)),
            Some(InputEvent::Orbit { dx: 3.0, dy: -2.0 })
        );
        assert_eq!(map_event(&motion(0, 3.0, -2.0)), None);
        assert_eq!(map_event(&motion(4, 3.0, -2.0)), None);
    }

    #[test]
    fn wheel_zooms_respecting_flipped_direction() {
        assert_eq!(
            map_event(&wheel(1.0, MouseWheelDirection::Normal)),
            Some(InputEvent::Zoom(1.0))
        );
        assert_eq!(
            map_event(&wheel(1.0, MouseWheelDirection::Flipped)),
            Some(InputEvent::Zoom(-1.0))
        );
        assert_eq!(map_event(&wheel(0.0, MouseWheelDirection::Normal)), None);
    }
}
