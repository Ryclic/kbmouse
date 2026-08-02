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
        while let Ok(config) = config_updates.try_recv() {
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
