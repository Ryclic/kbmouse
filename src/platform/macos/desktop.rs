//! Menu-bar lifetime and Spotlight reopen handling, independent of input capture.
use anyhow::{Context, Result};
use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DeclaredClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::{NSAppleEventManager, NSObject, NSObjectProtocol};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn show(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.request_repaint();
    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}
struct State {
    ctx: egui::Context,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    struct ReopenHandler;
    unsafe impl NSObjectProtocol for ReopenHandler {}
    impl ReopenHandler {
        #[unsafe(method(handleReopen:withReplyEvent:))]
        fn reopen(&self, _event: &AnyObject, _reply: &AnyObject) {
            show(&self.ivars().ctx);
        }
    }
);
// The standard Apple event sent by Launch Services when an app is reopened.
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");

pub struct Desktop {
    _icon: tray_icon::TrayIcon,
    _reopen_handler: Retained<ReopenHandler>,
    quit: Arc<AtomicBool>,
}
impl Desktop {
    pub fn new(ctx: &egui::Context) -> Result<Self> {
        use tray_icon::{
            TrayIconBuilder,
            menu::{Menu, MenuEvent, MenuItem},
        };
        let menu = Menu::new();
        let settings = MenuItem::new("Open settings", true, None);
        let quit_item = MenuItem::new("Quit kbmouse", true, None);
        menu.append(&settings)?;
        menu.append(&quit_item)?;
        let image =
            image::load_from_memory(include_bytes!("../../../assets/logo.png"))?.into_rgba8();
        let image = image::imageops::resize(&image, 22, 22, image::imageops::FilterType::Lanczos3);
        let icon = TrayIconBuilder::new()
            .with_tooltip("kbmouse")
            .with_icon(tray_icon::Icon::from_rgba(image.into_raw(), 22, 22)?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .context("could not create menu bar icon")?;
        let quit = Arc::new(AtomicBool::new(false));
        let event_ctx = ctx.clone();
        tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                tray_icon::TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }
            ) {
                show(&event_ctx);
            }
        }));
        let event_ctx = ctx.clone();
        let quit_flag = quit.clone();
        let quit_id = quit_item.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == quit_id {
                quit_flag.store(true, Ordering::Release);
            }
            show(&event_ctx);
        }));
        let mtm =
            MainThreadMarker::new().context("menu bar must be initialized on the main thread")?;
        let handler: Retained<ReopenHandler> = unsafe {
            msg_send![
                super(ReopenHandler::alloc(mtm).set_ivars(State { ctx: ctx.clone() })),
                init
            ]
        };
        let app = NSApplication::sharedApplication(mtm);
        // Do not replace NSApplication's delegate: winit owns and downcasts it.
        // Handle only Launch Services' reopen event through Apple's event manager.
        unsafe {
            NSAppleEventManager::sharedAppleEventManager()
                .setEventHandler_andSelector_forEventClass_andEventID(
                    &handler,
                    sel!(handleReopen:withReplyEvent:),
                    CORE_EVENT_CLASS,
                    REOPEN_APPLICATION,
                );
        }
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        Ok(Self {
            _icon: icon,
            _reopen_handler: handler,
            quit,
        })
    }
    pub fn take_quit(&self) -> bool {
        self.quit.swap(false, Ordering::AcqRel)
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        NSAppleEventManager::sharedAppleEventManager()
            .removeEventHandlerForEventClass_andEventID(CORE_EVENT_CLASS, REOPEN_APPLICATION);
    }
}
