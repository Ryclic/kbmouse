use crate::{
    config::{Config, LabelStyle, PostHint},
    geometry::Rect,
    labels,
};
use std::time::{Duration, Instant};

const SMOOTH_HOLD_DELAY: Duration = Duration::from_millis(75);
const SMOOTH_ACCELERATION: Duration = Duration::from_millis(380);
const FRAME_TIME: f32 = 1.0 / 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Hint,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Show(Scene),
    Hide,
    MoveTo(i32, i32),
    MoveBy(i32, i32),
    Click(MouseButton),
    Button(MouseButton, bool),
    Scroll(i32),
    Batch(Vec<Action>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    pub bounds: Rect,
    pub cells: Vec<Cell>,
    pub typed: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub bounds: Rect,
    pub label: String,
    pub matched: bool,
}

pub struct Engine {
    config: Config,
    mode: Mode,
    screen: Rect,
    hint_bounds: Rect,
    selected_cell: Option<Rect>,
    typed: String,
    dragging: bool,
    leader_started: Option<Instant>,
    leader_chord_used: bool,
    transient_normal: bool,
    directions: DirectionState,
    movement_started: Option<Instant>,
    last_movement_tick: Option<Instant>,
    movement_remainder: (f32, f32),
}

#[derive(Default)]
struct DirectionState {
    left: bool,
    down: bool,
    up: bool,
    right: bool,
}

#[derive(Clone, Copy)]
enum Direction {
    Left,
    Down,
    Up,
    Right,
}

impl DirectionState {
    fn any(&self) -> bool {
        self.left || self.down || self.up || self.right
    }

    fn vector(&self) -> (f32, f32) {
        let x = i8::from(self.right) - i8::from(self.left);
        let y = i8::from(self.down) - i8::from(self.up);
        if x != 0 && y != 0 {
            (
                x as f32 * std::f32::consts::FRAC_1_SQRT_2,
                y as f32 * std::f32::consts::FRAC_1_SQRT_2,
            )
        } else {
            (x as f32, y as f32)
        }
    }
}

impl Engine {
    pub fn new(config: Config, screen: Rect) -> Self {
        Self {
            config,
            mode: Mode::Idle,
            screen,
            hint_bounds: screen,
            selected_cell: None,
            typed: String::new(),
            dragging: false,
            leader_started: None,
            leader_chord_used: false,
            transient_normal: false,
            directions: DirectionState::default(),
            movement_started: None,
            last_movement_tick: None,
            movement_remainder: (0.0, 0.0),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn apply_config(&mut self, config: Config) -> Action {
        let cleanup = self.cancel();
        self.config = config;
        cleanup
    }

    pub fn set_screen(&mut self, bounds: Rect) {
        self.screen = bounds;
        if self.mode == Mode::Idle {
            self.hint_bounds = bounds;
        }
    }

    pub fn activate(&mut self) -> Action {
        self.reset_movement();
        self.transient_normal = false;
        self.leader_started = None;
        self.mode = Mode::Hint;
        self.hint_bounds = self.screen;
        self.selected_cell = None;
        self.typed.clear();
        self.show()
    }

    pub fn handle_key(&mut self, key: &str, pressed: bool) -> Action {
        let key = key.to_ascii_lowercase();
        if key == self.config.leader.to_ascii_lowercase() {
            if self.config.hold_leader_for_normal {
                return self.handle_dual_leader(pressed);
            }
            if !pressed {
                return Action::None;
            }
            return match self.mode {
                Mode::Idle => self.activate(),
                Mode::Hint => self.cancel(),
                Mode::Normal => self.activate(),
            };
        }
        if self.mode == Mode::Normal
            && let Some(direction) = self.direction_for_key(&key)
        {
            if pressed && self.transient_normal {
                self.leader_chord_used = true;
            }
            return self.direction_key(direction, pressed, Instant::now());
        }
        if !pressed {
            return Action::None;
        }
        if self.transient_normal {
            self.leader_chord_used = true;
        }
        match self.mode {
            Mode::Idle => Action::None,
            Mode::Hint => self.hint_key(&key),
            Mode::Normal => self.normal_key(&key),
        }
    }

    pub fn tick(&mut self, now: Instant) -> Action {
        if self.mode != Mode::Normal || !self.directions.any() {
            return Action::None;
        }
        let Some(started) = self.movement_started else {
            return Action::None;
        };
        let hold_delay = if self.config.smooth_movement {
            SMOOTH_HOLD_DELAY
        } else {
            Duration::from_millis(220)
        };
        if now.saturating_duration_since(started) < hold_delay {
            self.last_movement_tick = Some(now);
            return Action::None;
        }

        let previous = self.last_movement_tick.replace(now).unwrap_or(now);
        let delta_seconds = now
            .saturating_duration_since(previous)
            .as_secs_f32()
            .min(1.0 / 30.0);
        if delta_seconds == 0.0 {
            return Action::None;
        }

        let target_speed = self.current_move_step() as f32;
        let speed = if self.config.smooth_movement {
            let accelerating_for = now
                .saturating_duration_since(started + hold_delay)
                .as_secs_f32();
            let progress = (accelerating_for / SMOOTH_ACCELERATION.as_secs_f32()).clamp(0.0, 1.0);
            let starting_speed = target_speed.min(2.0);
            starting_speed + (target_speed - starting_speed) * progress.powf(1.5)
        } else {
            target_speed
        };
        let frame_scale = delta_seconds / FRAME_TIME;
        let (direction_x, direction_y) = self.directions.vector();
        self.movement_remainder.0 += direction_x * speed * frame_scale;
        self.movement_remainder.1 += direction_y * speed * frame_scale;
        let dx = self.movement_remainder.0.trunc() as i32;
        let dy = self.movement_remainder.1.trunc() as i32;
        self.movement_remainder.0 -= dx as f32;
        self.movement_remainder.1 -= dy as f32;
        if dx == 0 && dy == 0 {
            Action::None
        } else {
            Action::MoveBy(dx, dy)
        }
    }

    fn direction_for_key(&self, key: &str) -> Option<Direction> {
        let keys = &self.config.keys;
        if key == keys.left {
            Some(Direction::Left)
        } else if key == keys.down {
            Some(Direction::Down)
        } else if key == keys.up {
            Some(Direction::Up)
        } else if key == keys.right {
            Some(Direction::Right)
        } else {
            None
        }
    }

    fn direction_key(&mut self, direction: Direction, pressed: bool, now: Instant) -> Action {
        let state = match direction {
            Direction::Left => &mut self.directions.left,
            Direction::Down => &mut self.directions.down,
            Direction::Up => &mut self.directions.up,
            Direction::Right => &mut self.directions.right,
        };
        if *state == pressed {
            return Action::None;
        }
        *state = pressed;
        if !pressed {
            if !self.directions.any() {
                self.reset_movement();
            }
            return Action::None;
        }

        if self.movement_started.is_none() {
            self.movement_started = Some(now);
            self.last_movement_tick = Some(now);
            self.movement_remainder = (0.0, 0.0);
        }
        let step = if self.config.smooth_movement {
            self.current_move_step().min(3)
        } else {
            self.current_move_step()
        };
        match direction {
            Direction::Left => Action::MoveBy(-step, 0),
            Direction::Down => Action::MoveBy(0, step),
            Direction::Up => Action::MoveBy(0, -step),
            Direction::Right => Action::MoveBy(step, 0),
        }
    }

    fn current_move_step(&self) -> i32 {
        if self.transient_normal {
            self.config.hold_move_step
        } else {
            self.config.move_step
        }
    }

    fn handle_dual_leader(&mut self, pressed: bool) -> Action {
        if pressed {
            return match self.mode {
                Mode::Idle => {
                    self.reset_movement();
                    self.leader_started = Some(Instant::now());
                    self.leader_chord_used = false;
                    self.transient_normal = true;
                    self.mode = Mode::Normal;
                    Action::Hide
                }
                Mode::Hint => self.cancel(),
                Mode::Normal if self.transient_normal => Action::None,
                Mode::Normal => self.activate(),
            };
        }

        if !self.transient_normal {
            return Action::None;
        }
        self.transient_normal = false;
        let elapsed = self
            .leader_started
            .take()
            .map(|started| started.elapsed().as_millis())
            .unwrap_or(u128::MAX);
        if !self.leader_chord_used && elapsed <= self.config.leader_tap_ms as u128 {
            self.activate()
        } else {
            self.cancel()
        }
    }

    fn hint_key(&mut self, key: &str) -> Action {
        match key {
            "escape" => return self.cancel(),
            "backspace" => {
                self.typed.pop();
                return self.show();
            }
            _ => {}
        }
        if key.chars().count() != 1 {
            return Action::None;
        }
        self.typed.push(key.chars().next().unwrap());
        let (rows, cols) = self.grid_size();
        let Ok(labels) = self.generate_labels(rows * cols) else {
            return self.cancel();
        };
        let matching: Vec<usize> = labels
            .iter()
            .enumerate()
            .filter_map(|(index, label)| label.starts_with(&self.typed).then_some(index))
            .collect();
        if matching.is_empty() {
            self.typed.pop();
            return self.show();
        }
        if matching.len() == 1 && labels[matching[0]] == self.typed {
            let cell = self.hint_bounds.cell(rows, cols, matching[0]);
            let (x, y) = cell.center();
            self.selected_cell = Some(cell);
            self.typed.clear();
            return match self.config.post_hint {
                PostHint::Normal => {
                    self.mode = Mode::Normal;
                    Action::Batch(vec![Action::MoveTo(x, y), Action::Hide])
                }
                PostHint::Click => {
                    self.mode = Mode::Idle;
                    Action::Batch(vec![
                        Action::MoveTo(x, y),
                        Action::Hide,
                        Action::Click(MouseButton::Left),
                    ])
                }
                PostHint::Exit => {
                    self.mode = Mode::Idle;
                    Action::Batch(vec![Action::MoveTo(x, y), Action::Hide])
                }
            };
        }
        self.show()
    }

    fn normal_key(&mut self, key: &str) -> Action {
        if key == "escape" {
            return self.cancel();
        }
        if let (true, Some(selected_cell)) = (key == self.config.keys.subdivide, self.selected_cell)
        {
            self.mode = Mode::Hint;
            self.hint_bounds = selected_cell;
            self.typed.clear();
            return self.show();
        }
        let keys = &self.config.keys;
        if key == keys.scroll_up {
            Action::Scroll(self.config.scroll_step)
        } else if key == keys.scroll_down {
            Action::Scroll(-self.config.scroll_step)
        } else if key == keys.drag {
            self.dragging = !self.dragging;
            Action::Button(MouseButton::Left, self.dragging)
        } else if key == keys.left_click {
            self.click(MouseButton::Left)
        } else if key == keys.middle_click {
            self.click(MouseButton::Middle)
        } else if key == keys.right_click {
            self.click(MouseButton::Right)
        } else {
            Action::None
        }
    }

    fn click(&mut self, button: MouseButton) -> Action {
        let mut actions = Vec::new();
        if self.dragging {
            self.dragging = false;
            actions.push(Action::Button(MouseButton::Left, false));
        } else {
            actions.push(Action::Click(button));
        }
        if self.config.exit_on_click && !self.transient_normal {
            self.mode = Mode::Idle;
            self.reset_movement();
            actions.push(Action::Hide);
        }
        Action::Batch(actions)
    }

    fn cancel(&mut self) -> Action {
        self.mode = Mode::Idle;
        self.reset_movement();
        self.transient_normal = false;
        self.leader_started = None;
        self.leader_chord_used = false;
        self.typed.clear();
        self.hint_bounds = self.screen;
        if self.dragging {
            self.dragging = false;
            Action::Batch(vec![Action::Button(MouseButton::Left, false), Action::Hide])
        } else {
            Action::Hide
        }
    }

    fn reset_movement(&mut self) {
        self.directions = DirectionState::default();
        self.movement_started = None;
        self.last_movement_tick = None;
        self.movement_remainder = (0.0, 0.0);
    }

    fn grid_size(&self) -> (usize, usize) {
        let target = self.config.target_cell_px.max(1);
        let mut rows = self
            .config
            .grid_rows
            .unwrap_or_else(|| (self.hint_bounds.height / target).max(2) as usize);
        let mut cols = self
            .config
            .grid_cols
            .unwrap_or_else(|| (self.hint_bounds.width / target).max(2) as usize);
        if self.config.label_style == LabelStyle::Words {
            let capacity = labels::word_count();
            while rows * cols > capacity {
                if cols >= rows && cols > 2 {
                    cols -= 1;
                } else if rows > 2 {
                    rows -= 1;
                } else {
                    break;
                }
            }
        }
        (rows, cols)
    }

    fn show(&self) -> Action {
        let (rows, cols) = self.grid_size();
        let labels = self.generate_labels(rows * cols).unwrap_or_default();
        Action::Show(Scene {
            bounds: self.hint_bounds,
            cells: labels
                .into_iter()
                .enumerate()
                .map(|(index, label)| Cell {
                    bounds: self.hint_bounds.cell(rows, cols, index),
                    matched: label.starts_with(&self.typed),
                    label,
                })
                .collect(),
            typed: self.typed.clone(),
        })
    }

    fn generate_labels(&self, count: usize) -> anyhow::Result<Vec<String>> {
        match self.config.label_style {
            LabelStyle::Sequences => labels::generate(&self.config.alphabet, count),
            LabelStyle::Words => labels::generate_words(count),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        let config = Config {
            alphabet: "ab".into(),
            grid_rows: Some(2),
            grid_cols: Some(2),
            ..Config::default()
        };
        Engine::new(
            config,
            Rect {
                x: -100,
                y: 0,
                width: 200,
                height: 100,
            },
        )
    }

    #[test]
    fn leader_tap_opens_and_escape_closes() {
        let mut engine = engine();
        assert_eq!(engine.handle_key("capslock", true), Action::Hide);
        assert_eq!(engine.mode(), Mode::Normal);
        assert!(matches!(
            engine.handle_key("capslock", false),
            Action::Show(_)
        ));
        assert_eq!(engine.mode(), Mode::Hint);
        assert_eq!(engine.handle_key("escape", true), Action::Hide);
        assert_eq!(engine.mode(), Mode::Idle);
    }

    #[test]
    fn leader_hold_is_momentary_normal_mode() {
        let mut engine = engine();
        assert_eq!(engine.handle_key("capslock", true), Action::Hide);
        assert_eq!(engine.handle_key("h", true), Action::MoveBy(-24, 0));
        assert_eq!(engine.mode(), Mode::Normal);
        assert_eq!(engine.handle_key("capslock", false), Action::Hide);
        assert_eq!(engine.mode(), Mode::Idle);
    }

    #[test]
    fn full_label_moves_to_cell_and_enters_normal() {
        let mut engine = engine();
        engine.activate();
        assert!(matches!(engine.handle_key("a", true), Action::Show(_)));
        let action = engine.handle_key("b", true);
        assert_eq!(
            action,
            Action::Batch(vec![Action::MoveTo(50, 25), Action::Hide])
        );
        assert_eq!(engine.mode(), Mode::Normal);
        assert_eq!(engine.handle_key("h", true), Action::MoveBy(-8, 0));
    }

    #[test]
    fn invalid_prefix_does_not_destroy_progress() {
        let mut engine = engine();
        engine.activate();
        engine.handle_key("a", true);
        let Action::Show(scene) = engine.handle_key("z", true) else {
            panic!()
        };
        assert_eq!(scene.typed, "a");
    }

    #[test]
    fn space_subdivides_selected_cell() {
        let mut engine = engine();
        engine.activate();
        engine.handle_key("a", true);
        engine.handle_key("a", true);
        let Action::Show(scene) = engine.handle_key("space", true) else {
            panic!()
        };
        assert_eq!(
            scene.bounds,
            Rect {
                x: -100,
                y: 0,
                width: 100,
                height: 50
            }
        );
    }

    #[test]
    fn click_exits_normal_mode() {
        let mut engine = engine();
        engine.activate();
        engine.handle_key("a", true);
        engine.handle_key("a", true);
        assert_eq!(
            engine.handle_key("m", true),
            Action::Batch(vec![Action::Click(MouseButton::Left), Action::Hide])
        );
        assert_eq!(engine.mode(), Mode::Idle);
    }

    #[test]
    fn escape_releases_an_active_drag() {
        let mut engine = engine();
        engine.activate();
        engine.handle_key("a", true);
        engine.handle_key("a", true);
        assert_eq!(
            engine.handle_key("v", true),
            Action::Button(MouseButton::Left, true)
        );
        assert_eq!(
            engine.handle_key("escape", true),
            Action::Batch(vec![Action::Button(MouseButton::Left, false), Action::Hide])
        );
    }

    #[test]
    fn smooth_mode_keeps_taps_granular() {
        let mut engine = engine();
        engine.config.smooth_movement = true;
        engine.activate();
        engine.handle_key("a", true);
        engine.handle_key("a", true);
        assert_eq!(engine.handle_key("h", true), Action::MoveBy(-3, 0));
        assert_eq!(engine.handle_key("h", false), Action::None);
    }

    #[test]
    fn held_directions_accelerate_diagonally() {
        let mut engine = engine();
        engine.config.smooth_movement = true;
        engine.activate();
        engine.handle_key("a", true);
        engine.handle_key("a", true);
        engine.handle_key("h", true);
        engine.handle_key("k", true);
        let started = engine.movement_started.unwrap();
        let action = engine.tick(started + SMOOTH_HOLD_DELAY + SMOOTH_ACCELERATION);
        let Action::MoveBy(dx, dy) = action else {
            panic!("expected diagonal movement");
        };
        assert!(dx < 0 && dy < 0);
        assert!((dx.abs() - dy.abs()).abs() <= 1);
        engine.handle_key("h", false);
        engine.handle_key("k", false);
        assert_eq!(
            engine.tick(started + SMOOTH_HOLD_DELAY + SMOOTH_ACCELERATION * 2),
            Action::None
        );
    }

    #[test]
    fn word_mode_selects_cells_with_three_letter_words() {
        let mut engine = engine();
        engine.config.label_style = LabelStyle::Words;
        let Action::Show(scene) = engine.activate() else {
            panic!("expected word grid");
        };
        assert_eq!(scene.cells[0].label, "ace");
        engine.handle_key("a", true);
        engine.handle_key("c", true);
        assert_eq!(
            engine.handle_key("e", true),
            Action::Batch(vec![Action::MoveTo(-50, 25), Action::Hide])
        );
    }

    #[test]
    fn word_mode_caps_very_dense_grids() {
        let mut engine = engine();
        engine.config.label_style = LabelStyle::Words;
        engine.config.grid_rows = Some(40);
        engine.config.grid_cols = Some(60);
        let Action::Show(scene) = engine.activate() else {
            panic!("expected word grid");
        };
        assert!(scene.cells.len() <= labels::word_count());
        assert!(!scene.cells.is_empty());
    }

    #[test]
    fn applying_config_cleans_up_and_changes_behavior() {
        let mut engine = engine();
        engine.activate();
        engine.handle_key("a", true);
        engine.handle_key("a", true);
        engine.handle_key("v", true);
        let mut updated = engine.config.clone();
        updated.move_step = 13;
        assert_eq!(
            engine.apply_config(updated),
            Action::Batch(vec![Action::Button(MouseButton::Left, false), Action::Hide])
        );
        assert_eq!(engine.mode(), Mode::Idle);
        engine.activate();
        engine.handle_key("a", true);
        engine.handle_key("a", true);
        assert_eq!(engine.handle_key("h", true), Action::MoveBy(-13, 0));
    }
}
