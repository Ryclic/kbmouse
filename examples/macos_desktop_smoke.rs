#[cfg(target_os = "macos")]
#[path = "../src/branding.rs"]
mod branding;

// Run `cargo run --example macos_desktop_smoke` on macOS to check native startup
// without keyboard-capture permissions. The window closes automatically.
#[cfg(target_os = "macos")]
#[path = "../src/platform/macos/desktop.rs"]
mod desktop;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use eframe::egui;
    struct Smoke {
        desktop: desktop::Desktop,
        started: std::time::Instant,
    }
    impl eframe::App for Smoke {
        fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
            root.label("kbmouse menu bar startup check — closes automatically");
            if self.started.elapsed() >= std::time::Duration::from_secs(3)
                || self.desktop.take_quit()
            {
                root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    eframe::run_native(
        "kbmouse native smoke check",
        Default::default(),
        Box::new(|cc| {
            let desktop = desktop::Desktop::new(&cc.egui_ctx)?;
            Ok(Box::new(Smoke {
                desktop,
                started: std::time::Instant::now(),
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This native smoke check requires macOS.");
}
