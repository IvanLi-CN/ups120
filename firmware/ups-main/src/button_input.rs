//! High-level button input module for ups-main.
//!
//! This module implements a small, self-contained state machine for each
//! physical key. It encapsulates debouncing and gesture recognition
//! (single-click / double-click / long-press / long-press repeat) and
//! exposes a unified set of log events to higher layers:
//!
//! - `button event: <name> pressed` / `released`
//! - `button click: <name> single`
//! - `button click: <name> double`
//! - `button click: <name> repeat`
//! - `button long:  <name> start` / `end (xxx ms)`
//!
//! Typical integration pattern:
//! - create one [`ButtonState`] per physical key with an appropriate
//!   [`ButtonConfig`] (per-key gesture policy);
//! - in the main loop, sample each GPIO at a fixed cadence (e.g. every
//!   10–20 ms) and feed `now_ms` plus the physical level into
//!   [`ButtonState::update`];
//! - react to the defmt log events in a higher-level input or UI layer.
//!
//! Timing assumptions:
//! - `now_ms` must be monotonic and expressed in milliseconds;
//! - the `update` cadence must be significantly shorter than the shortest
//!   human press you want to observe (e.g. 20 ms for ~50 ms taps);
//! - this module does not own the GPIO; the caller is responsible for
//!   configuring input mode and pull-ups.

use defmt::info;

/// Per-button configuration for debouncing and gesture detection.
///
/// All time values are expressed in milliseconds of wall-clock time,
/// using the same timestamp basis as the main loop (`Instant::now()`).
pub struct ButtonConfig {
    pub name: &'static str,
    /// Whether to emit `button long: <name> start/end` events.
    pub enable_long: bool,
    /// Whether to distinguish double-clicks.
    /// When enabled, single-click confirmation is delayed until the
    /// double-click window has expired.
    pub enable_double: bool,
    /// When true and `repeat_click_interval_ms` is set, a held button
    /// will generate periodic `button click: <name> repeat` events after
    /// the long-press threshold.
    pub long_generates_clicks: bool,
    /// Optional repeat-click interval while the button is held.
    ///
    /// Behavior matrix:
    /// - `enable_long = true`, `repeat_click_interval_ms = Some(x)`:
    ///   - press -> after `long_press_ms` emit `long start`;
    ///   - while held, every `x` ms emit `button click: <name> repeat`;
    ///   - release -> `long end`.
    /// - `enable_long = false`, `repeat_click_interval_ms = Some(x)`:
    ///   - press -> after `x` ms start emitting `repeat` events while held;
    ///   - no `long` events are produced.
    pub repeat_click_interval_ms: Option<u64>,
    /// Debounce window: physical state must remain stable for at least
    /// this long before a press/release transition is committed.
    pub debounce_ms: u64,
    /// Minimum duration to treat a press as a long-press.
    pub long_press_ms: u64,
    /// Maximum gap between two click candidates to consider them a
    /// double-click. When `enable_double` is true, single-click
    /// confirmation is delayed until this window has expired.
    pub double_click_gap_ms: u64,
}

impl ButtonConfig {
    /// Construct a config with sensible defaults for human-operated keys.
    ///
    /// Defaults:
    /// - long-press enabled at 800 ms;
    /// - double-click enabled with a 200 ms gap;
    /// - no long-press repeat clicks;
    /// - 30 ms debounce.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            enable_long: true,
            enable_double: true,
            long_generates_clicks: false,
            repeat_click_interval_ms: None,
            debounce_ms: 30,
            long_press_ms: 800,
            double_click_gap_ms: 200,
        }
    }
}

/// Runtime state for a single button.
///
/// A `ButtonState` instance is pure state: it does not own the GPIO and
/// only consumes booleans plus a millisecond timestamp. All observable
/// behavior is via defmt log messages; there is no callback mechanism.
///
/// Gesture semantics:
/// - Press/release edges are always logged as `button event: <name> ...`.
/// - If `enable_long` is true and the button remains pressed longer than
///   `long_press_ms`, `button long: <name> start` is logged once, and
///   `button long: <name> end (xxx ms)` is logged on release.
/// - Clicks:
///   - when `enable_double` is true, short presses become "click
///     candidates"; if a second click candidate arrives within
///     `double_click_gap_ms`, `button click: <name> double` is logged;
///     otherwise, once the gap has expired, a
///     `button click: <name> single` is logged.
///   - when `enable_double` is false, a short press immediately produces
///     `button click: <name> single` on release, without any delay.
/// - When `repeat_click_interval_ms` is set, additional
///   `button click: <name> repeat` events are generated while the button
///   remains pressed (see [`ButtonConfig`] for the exact rules).
pub struct ButtonState {
    cfg: ButtonConfig,
    /// Debounced logical level (true = pressed).
    stable_pressed: bool,
    last_physical: bool,
    last_bounce_ms: u64,
    press_start_ms: Option<u64>,
    long_reported: bool,
    click_pending: bool,
    last_click_ms: u64,
    last_repeat_ms: Option<u64>,
}

impl ButtonState {
    /// Create a new state machine with the given configuration and an
    /// initial physical level.
    pub fn new(cfg: ButtonConfig, initial_pressed: bool, now_ms: u64) -> Self {
        Self {
            cfg,
            stable_pressed: initial_pressed,
            last_physical: initial_pressed,
            last_bounce_ms: now_ms,
            press_start_ms: if initial_pressed { Some(now_ms) } else { None },
            long_reported: false,
            click_pending: false,
            last_click_ms: 0,
            last_repeat_ms: None,
        }
    }

    /// Return the current debounced logical level of the button.
    ///
    /// Higher-level UI layers can use this to derive simple state
    /// transitions (e.g. screen navigation) without reimplementing
    /// debouncing or gesture logic.
    pub fn is_pressed(&self) -> bool {
        self.stable_pressed
    }

    /// Mutable access to the configuration for dynamic tuning at runtime.
    pub fn config_mut(&mut self) -> &mut ButtonConfig {
        &mut self.cfg
    }

    /// Feed a new physical sample into the state machine.
    ///
    /// `physical_pressed` should reflect the debounced electrical level
    /// (active-low wiring means you typically pass `gpio.is_low()`).
    pub fn update(&mut self, now_ms: u64, physical_pressed: bool) {
        // --- Debounce ---
        if physical_pressed != self.last_physical {
            self.last_physical = physical_pressed;
            self.last_bounce_ms = now_ms;
        }

        if physical_pressed != self.stable_pressed
            && now_ms.saturating_sub(self.last_bounce_ms) >= self.cfg.debounce_ms
        {
            self.stable_pressed = physical_pressed;
            if physical_pressed {
                self.on_press(now_ms);
            } else {
                self.on_release(now_ms);
            }
        }

        // --- Long-press start ---
        if self.cfg.enable_long && self.stable_pressed {
            if let Some(start) = self.press_start_ms {
                if !self.long_reported && now_ms.saturating_sub(start) >= self.cfg.long_press_ms {
                    self.long_reported = true;
                    info!("button long: {} start", self.cfg.name);
                    if self.cfg.long_generates_clicks && self.cfg.repeat_click_interval_ms.is_some()
                    {
                        self.last_repeat_ms = Some(now_ms);
                    }
                }
            }
        }

        // --- Repeat clicks while held (if configured) ---
        if self.stable_pressed {
            if let Some(interval) = self.cfg.repeat_click_interval_ms {
                if let Some(last) = self.last_repeat_ms {
                    if now_ms.saturating_sub(last) >= interval {
                        self.last_repeat_ms = Some(now_ms);
                        info!("button click: {} repeat", self.cfg.name);
                    }
                } else if !self.cfg.enable_long {
                    // No long-press semantics: start repeating after the first interval.
                    if let Some(start) = self.press_start_ms {
                        if now_ms.saturating_sub(start) >= interval {
                            self.last_repeat_ms = Some(now_ms);
                            info!("button click: {} repeat", self.cfg.name);
                        }
                    }
                }
            }
        }

        // --- Resolve pending single-click when double-click gap expires ---
        if self.cfg.enable_double && self.click_pending {
            if now_ms.saturating_sub(self.last_click_ms) > self.cfg.double_click_gap_ms {
                self.click_pending = false;
                info!("button click: {} single", self.cfg.name);
            }
        }
    }

    fn on_press(&mut self, now_ms: u64) {
        self.press_start_ms = Some(now_ms);
        self.long_reported = false;
        self.last_repeat_ms = None;
        info!("button event: {} pressed", self.cfg.name);
    }

    fn on_release(&mut self, now_ms: u64) {
        info!("button event: {} released", self.cfg.name);

        if let Some(start) = self.press_start_ms.take() {
            let duration_ms = now_ms.saturating_sub(start);

            if self.cfg.enable_long && duration_ms >= self.cfg.long_press_ms {
                info!(
                    "button long: {} end ({} ms)",
                    self.cfg.name, duration_ms as u32
                );
                // Long-press is a distinct gesture; do not emit click/double-click.
                self.click_pending = false;
                self.last_repeat_ms = None;
                return;
            }

            // Click/double-click path.
            if self.cfg.enable_double {
                if self.click_pending
                    && now_ms.saturating_sub(self.last_click_ms) <= self.cfg.double_click_gap_ms
                {
                    info!("button click: {} double", self.cfg.name);
                    self.click_pending = false;
                } else {
                    self.click_pending = true;
                    self.last_click_ms = now_ms;
                }
            } else {
                // Double-click disabled: emit single-click immediately.
                info!("button click: {} single", self.cfg.name);
                self.click_pending = false;
            }

            self.last_repeat_ms = None;
        }
    }
}
