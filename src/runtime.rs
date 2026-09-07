use crate::{
    config::Config,
    engine::{Action, Engine, MouseButton},
    platform::Backend,
};
use anyhow::Result;
use crossbeam_channel::Receiver;
use std::time::{Duration, Instant};

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(8);

pub fn run<B: Backend>(
    mut backend: B,
    config: Config,
    one_shot: bool,
    config_updates: Receiver<Config>,
) -> Result<()> {
    let mut engine = Engine::new(config, backend.screen_bounds());
    if one_shot {
        backend.set_active(true);
        execute(&mut backend, engine.activate())?;
    }

    loop {
        loop {
            let config = match config_updates.try_recv() {
                Ok(config) => config,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    backend.set_active(false);
                    execute(&mut backend, engine.cancel())?;
                    return Ok(());
                }
            };
            let cleanup = engine.apply_config(config.clone());
            backend.set_active(false);
            execute(&mut backend, cleanup)?;
            backend.apply_config(&config)?;
            engine.set_screen(backend.screen_bounds());
        }
        if let Some(event) = backend.next_event(INPUT_POLL_INTERVAL)? {
            engine.set_screen(backend.screen_bounds());
            let action = engine.handle_key(&event.key, event.pressed);
            backend.set_active(engine.mode() != crate::engine::Mode::Idle);
            execute(&mut backend, action)?;
        }
        execute(&mut backend, engine.tick(Instant::now()))?;
        if one_shot && engine.mode() == crate::engine::Mode::Idle {
            return Ok(());
        }
    }
}

fn execute<B: Backend>(backend: &mut B, action: Action) -> Result<()> {
    match action {
        Action::None => {}
        Action::Show(scene) => backend.show(&scene)?,
        Action::Hide => backend.hide()?,
        Action::MoveTo(x, y) => backend.move_to(x, y)?,
        Action::MoveBy(dx, dy) => backend.move_by(dx, dy)?,
        Action::Snap => backend.snap_to_clickable()?,
        Action::Click(button) => {
            backend.button(button, true)?;
            backend.button(button, false)?;
        }
        Action::Button(button, down) => backend.button(button, down)?,
        Action::Scroll(amount) => backend.scroll(amount)?,
        Action::Batch(actions) => {
            for action in actions {
                execute(backend, action)?;
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn _assert_mouse_button_send(_: MouseButton) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{engine::Scene, geometry::Rect, platform::KeyEvent};
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    struct ShutdownBackend {
        events: VecDeque<KeyEvent>,
        settings: Option<crossbeam_channel::Sender<Config>>,
        buttons: Rc<RefCell<Vec<bool>>>,
    }
    impl Backend for ShutdownBackend {
        fn screen_bounds(&self) -> Rect {
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            }
        }
        fn next_event(&mut self, _: Duration) -> Result<Option<KeyEvent>> {
            let event = self.events.pop_front();
            if event.is_none() {
                self.settings.take();
            }
            Ok(event)
        }
        fn apply_config(&mut self, _: &Config) -> Result<()> {
            Ok(())
        }
        fn set_active(&mut self, _: bool) {}
        fn show(&mut self, _: &Scene) -> Result<()> {
            Ok(())
        }
        fn hide(&mut self) -> Result<()> {
            Ok(())
        }
        fn move_to(&mut self, _: i32, _: i32) -> Result<()> {
            Ok(())
        }
        fn move_by(&mut self, _: i32, _: i32) -> Result<()> {
            Ok(())
        }
        fn snap_to_clickable(&mut self) -> Result<()> {
            Ok(())
        }
        fn button(&mut self, _: MouseButton, down: bool) -> Result<()> {
            self.buttons.borrow_mut().push(down);
            Ok(())
        }
        fn scroll(&mut self, _: i32) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn closing_settings_releases_drag_and_stops_runtime() {
        let config = Config::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let buttons = Rc::new(RefCell::new(Vec::new()));
        let backend = ShutdownBackend {
            events: VecDeque::from([
                KeyEvent {
                    key: config.leader.clone(),
                    pressed: true,
                },
                KeyEvent {
                    key: config.keys.left_click.clone(),
                    pressed: true,
                },
            ]),
            settings: Some(tx),
            buttons: buttons.clone(),
        };
        run(backend, config, false, rx).unwrap();
        assert_eq!(*buttons.borrow(), vec![true, false]);
    }
}
