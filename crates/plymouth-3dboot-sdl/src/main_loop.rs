// SPDX-License-Identifier: GPL-3.0-or-later
//! The frame loop: native (blocking) or Emscripten (browser-driven).
//!
//! Application logic implements [`App`] and is driven by a [`Runner`], a
//! pure state machine that is fed timestamps and input events, so it can be
//! tested without SDL. [`run`] connects a `Runner` to SDL and a [`Presenter`].

use plymouth_3dboot::target::ColorBuffer;

use crate::presenter::{Presenter, SdlError};

/// Input delivered to [`App::update`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum InputEvent {
    /// The user asked to close the application.
    Quit,
}

/// Whether the loop should keep running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    /// Render this frame and continue.
    Continue,
    /// Stop the loop (nothing more is rendered).
    Quit,
}

/// An application driven by the frame loop.
pub trait App {
    /// Advances the application by `dt` seconds, given the input received
    /// since the previous frame.
    fn update(&mut self, events: &[InputEvent], dt: f64) -> Control;

    /// Renders a frame for an output of `size` (physical pixels).
    fn render(&mut self, size: (u32, u32)) -> &ColorBuffer;
}

/// Frame-loop settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunOptions {
    /// Stop after this many frames (for tests and benchmarks).
    pub max_frames: Option<u64>,
    /// Upper bound for `dt`, so a stall (debugger, hidden tab) does not
    /// cause a huge jump.
    pub max_dt: f64,
    /// Native only: sleep to keep at most this many frames per second.
    /// (In the browser, frames follow `requestAnimationFrame`.)
    pub fps_cap: Option<f64>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_frames: None,
            max_dt: 0.25,
            fps_cap: Some(60.0),
        }
    }
}

/// Drives an [`App`] from timestamps and events.
#[derive(Debug)]
pub struct Runner<A> {
    app: A,
    options: RunOptions,
    last_time: Option<f64>,
    frames: u64,
}

impl<A: App> Runner<A> {
    /// A runner that has not yet run any frame.
    pub fn new(app: A, options: RunOptions) -> Self {
        Self {
            app,
            options,
            last_time: None,
            frames: 0,
        }
    }

    /// Runs the update for a frame at time `now` (seconds, monotonic).
    /// The first frame has `dt = 0`. Returns [`Control::Quit`] if the app
    /// quits or the frame limit has been reached (the update is not run
    /// then).
    pub fn step(&mut self, now: f64, events: &[InputEvent]) -> Control {
        if self
            .options
            .max_frames
            .is_some_and(|max| self.frames >= max)
        {
            return Control::Quit;
        }
        let dt = self
            .last_time
            .map_or(0.0, |last| (now - last).clamp(0.0, self.options.max_dt));
        self.last_time = Some(now);
        let control = self.app.update(events, dt);
        if control == Control::Continue {
            self.frames += 1;
        }
        control
    }

    /// Number of frames updated so far (those that continued).
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The application.
    pub fn app(&self) -> &A {
        &self.app
    }

    /// The application, mutably.
    pub fn app_mut(&mut self) -> &mut A {
        &mut self.app
    }

    /// Consumes the runner, returning the application.
    pub fn into_app(self) -> A {
        self.app
    }
}

/// SDL events mapped to [`InputEvent`]s (unmapped events are dropped).
fn map_event(event: &sdl3::event::Event) -> Option<InputEvent> {
    match event {
        sdl3::event::Event::Quit { .. } => Some(InputEvent::Quit),
        _ => None,
    }
}

/// Per-frame work shared by the native and browser loops: poll events,
/// update, render and present. Returns whether to continue.
struct Driver<A> {
    runner: Runner<A>,
    presenter: Presenter,
    events: sdl3::EventPump,
    start: std::time::Instant,
    events_buf: Vec<InputEvent>,
}

impl<A: App> Driver<A> {
    fn frame(&mut self) -> Result<Control, SdlError> {
        self.events_buf.clear();
        self.events_buf
            .extend(self.events.poll_iter().filter_map(|e| map_event(&e)));
        let now = self.start.elapsed().as_secs_f64();
        if self.runner.step(now, &self.events_buf) == Control::Quit {
            return Ok(Control::Quit);
        }
        let size = self.presenter.output_size()?;
        let frame = self.runner.app_mut().render(size);
        self.presenter.show(frame)?;
        Ok(Control::Continue)
    }
}

/// Runs `app` until it quits (or `options.max_frames` is reached),
/// presenting each rendered frame.
///
/// Natively this blocks and returns the app. On Emscripten it registers the
/// loop with the browser and returns `Ok(None)` immediately; the page keeps
/// running it.
///
/// # Errors
///
/// Returns [`SdlError`] if the event pump cannot be created or a frame
/// cannot be presented (natively; in the browser, errors are logged and stop
/// the loop).
pub fn run<A: App + 'static>(
    presenter: Presenter,
    app: A,
    options: RunOptions,
) -> Result<Option<A>, SdlError> {
    let events = presenter
        .sdl()
        .event_pump()
        .map_err(|e| SdlError(e.to_string()))?;
    let driver = Driver {
        runner: Runner::new(app, options),
        presenter,
        events,
        start: std::time::Instant::now(),
        events_buf: Vec::new(),
    };
    run_driver(driver, options)
}

#[cfg(not(target_os = "emscripten"))]
fn run_driver<A: App + 'static>(
    mut driver: Driver<A>,
    options: RunOptions,
) -> Result<Option<A>, SdlError> {
    let frame_time = options
        .fps_cap
        .filter(|f| *f > 0.0)
        .map(|f| std::time::Duration::from_secs_f64(1.0 / f));
    loop {
        let started = std::time::Instant::now();
        if driver.frame()? == Control::Quit {
            return Ok(Some(driver.runner.into_app()));
        }
        if let Some(remaining) = frame_time.and_then(|t| t.checked_sub(started.elapsed())) {
            std::thread::sleep(remaining);
        }
    }
}

#[cfg(target_os = "emscripten")]
fn run_driver<A: App + 'static>(
    driver: Driver<A>,
    _options: RunOptions,
) -> Result<Option<A>, SdlError> {
    let mut driver = driver;
    crate::emscripten::set_main_loop(move || match driver.frame() {
        Ok(Control::Continue) => true,
        Ok(Control::Quit) => false,
        Err(e) => {
            eprintln!("frame failed: {e}");
            false
        }
    });
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plymouth_3dboot::color::Rgba8;

    /// Records what it was given; quits on `InputEvent::Quit`.
    struct Recorder {
        dts: Vec<f64>,
        frame: ColorBuffer,
    }

    impl Recorder {
        fn new() -> Self {
            Self {
                dts: Vec::new(),
                frame: ColorBuffer::new(1, 1, Rgba8::BLACK).unwrap(),
            }
        }
    }

    impl App for Recorder {
        fn update(&mut self, events: &[InputEvent], dt: f64) -> Control {
            self.dts.push(dt);
            if events.contains(&InputEvent::Quit) {
                Control::Quit
            } else {
                Control::Continue
            }
        }

        fn render(&mut self, _: (u32, u32)) -> &ColorBuffer {
            &self.frame
        }
    }

    #[test]
    fn dt_comes_from_timestamps_and_is_clamped() {
        let mut r = Runner::new(
            Recorder::new(),
            RunOptions {
                max_dt: 0.1,
                ..RunOptions::default()
            },
        );
        for t in [10.0, 10.016, 10.05, 12.0, 11.0] {
            assert_eq!(r.step(t, &[]), Control::Continue);
        }
        let dts = &r.app().dts;
        assert_eq!(dts[0], 0.0, "first frame");
        assert!((dts[1] - 0.016).abs() < 1e-9 && (dts[2] - 0.034).abs() < 1e-9);
        assert_eq!(dts[3], 0.1, "stall clamped to max_dt");
        assert_eq!(dts[4], 0.0, "clock going backwards gives dt = 0");
        assert_eq!(r.frames(), 5);
    }

    #[test]
    fn quit_event_stops_the_loop() {
        let mut r = Runner::new(Recorder::new(), RunOptions::default());
        assert_eq!(r.step(0.0, &[]), Control::Continue);
        assert_eq!(r.step(0.1, &[InputEvent::Quit]), Control::Quit);
        assert_eq!(r.frames(), 1);
    }

    #[test]
    fn frame_limit_stops_without_updating() {
        let mut r = Runner::new(
            Recorder::new(),
            RunOptions {
                max_frames: Some(2),
                ..RunOptions::default()
            },
        );
        assert_eq!(r.step(0.0, &[]), Control::Continue);
        assert_eq!(r.step(0.1, &[]), Control::Continue);
        assert_eq!(r.step(0.2, &[]), Control::Quit);
        assert_eq!(r.into_app().dts.len(), 2);
    }
}
