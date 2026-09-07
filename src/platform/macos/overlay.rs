use crate::{config::Config, engine::Scene, geometry::Rect};
use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
use core_graphics::display::CGDisplay;
use dispatch2::DispatchQueue;
use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly,
    rc::{Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSDictionary, NSPoint, NSRect, NSSize, NSString,
};
use std::{cell::RefCell, time::Duration};

thread_local! {
    // Accessed exclusively on the main thread; AppKit objects never cross threads.
    static PANEL: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
}

pub fn initialize(one_shot: bool) {
    let mtm = MainThreadMarker::new().expect("initialize macOS on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    if one_shot {
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
    }
}
pub fn pump_events(timeout: Duration) {
    autoreleasepool(|_| {
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            let until = NSDate::dateWithTimeIntervalSinceNow(timeout.as_secs_f64());
            let event = unsafe {
                app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    NSEventMask::Any,
                    Some(&until),
                    NSDefaultRunLoopMode,
                    true,
                )
            };
            if let Some(event) = event {
                app.sendEvent(&event);
            }
        } else {
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, timeout, true);
        }
    });
}
fn on_main(work: impl FnOnce(MainThreadMarker) + Send + 'static) {
    if let Some(mtm) = MainThreadMarker::new() {
        autoreleasepool(|_| work(mtm));
    } else {
        DispatchQueue::main().exec_async(move || {
            autoreleasepool(|_| {
                work(MainThreadMarker::new().expect("main dispatch queue"));
            })
        });
    }
}
pub fn hide() {
    on_main(|_| {
        PANEL.with(|panel| {
            if let Some(panel) = panel.borrow().as_ref() {
                panel.orderOut(None);
            }
        })
    });
}
pub fn show(scene: Scene, config: Config) {
    on_main(move |mtm| {
        // Rendering and window access both stay on the main thread. Only Rust scene
        // data is dispatched from the input thread, which remains free to capture keys.
        let image = draw(&scene, &config);
        PANEL.with(|slot| {
            let mut slot = slot.borrow_mut();
            let panel = slot.get_or_insert_with(|| {
                let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                    NSPanel::alloc(mtm),
                    NSRect::ZERO,
                    NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                    NSBackingStoreType::Buffered,
                    false,
                );
                // The retained Rust owner controls the panel's lifetime.
                unsafe {
                    panel.setReleasedWhenClosed(false);
                }
                panel.setOpaque(false);
                panel.setBackgroundColor(Some(&NSColor::clearColor()));
                panel.setIgnoresMouseEvents(true);
                panel.setHasShadow(false);
                panel.setHidesOnDeactivate(false);
                panel.setLevel(NSScreenSaverWindowLevel);
                panel.setCollectionBehavior(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::FullScreenAuxiliary,
                );
                panel
            });
            let top = CGDisplay::main().bounds().size.height;
            panel.setFrame_display(appkit_frame(scene.bounds, top), false);
            let view = NSImageView::initWithFrame(
                NSImageView::alloc(mtm),
                rect(
                    0.0,
                    0.0,
                    scene.bounds.width as f64,
                    scene.bounds.height as f64,
                ),
            );
            view.setImage(Some(&image));
            view.setImageScaling(NSImageScaling::ScaleAxesIndependently);
            panel.setContentView(Some(&view));
            panel.orderFrontRegardless();
        });
    });
}
fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
fn appkit_frame(bounds: Rect, primary_height: f64) -> NSRect {
    rect(
        bounds.x as f64,
        primary_height - bounds.y as f64 - bounds.height as f64,
        bounds.width as f64,
        bounds.height as f64,
    )
}
fn color(value: &str) -> Retained<NSColor> {
    let rgb = u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    NSColor::colorWithSRGBRed_green_blue_alpha(
        ((rgb >> 16) & 255) as f64 / 255.0,
        ((rgb >> 8) & 255) as f64 / 255.0,
        (rgb & 255) as f64 / 255.0,
        1.0,
    )
}
#[allow(deprecated)] // Preserve the existing NSImage drawing path while removing native sources.
fn draw(scene: &Scene, config: &Config) -> Retained<NSImage> {
    let image = NSImage::initWithSize(
        NSImage::alloc(),
        NSSize::new(scene.bounds.width as f64, scene.bounds.height as f64),
    );
    image.lockFocusFlipped(true);
    // Always unlock the drawing context, including during Rust unwinding.
    struct Focus<'a>(&'a NSImage);
    impl Drop for Focus<'_> {
        fn drop(&mut self) {
            self.0.unlockFocus();
        }
    }
    let focus = Focus(&image);
    if let Some(context) = NSGraphicsContext::currentContext() {
        context.setShouldAntialias(!config.crisp_labels);
    }
    let background = color(&config.background_color);
    let grid = color(&config.grid_color);
    let text = color(&config.text_color);
    let accent = color(&config.accent_color);
    background
        .colorWithAlphaComponent(config.backdrop_opacity.min(100) as f64 / 100.0)
        .setFill();
    NSBezierPath::bezierPathWithRect(rect(
        0.0,
        0.0,
        scene.bounds.width as f64,
        scene.bounds.height as f64,
    ))
    .fill();
    let font = unsafe {
        NSFont::monospacedSystemFontOfSize_weight(config.font_size as f64, NSFontWeightSemibold)
    };
    for cell in &scene.cells {
        let x = (cell.bounds.x - scene.bounds.x) as f64;
        let y = (cell.bounds.y - scene.bounds.y) as f64;
        let width = cell.bounds.width as f64;
        let height = cell.bounds.height as f64;
        grid.colorWithAlphaComponent(if cell.matched { 0.85 } else { 0.3 })
            .setStroke();
        NSBezierPath::bezierPathWithRect(rect(x + 0.5, y + 0.5, width - 1.0, height - 1.0))
            .stroke();
        let label = NSString::from_str(&cell.label);
        let foreground = if cell.matched {
            if scene.typed.is_empty() {
                &text
            } else {
                &accent
            }
        } else {
            &grid
        };
        let shadow = NSShadow::new();
        shadow.setShadowColor(Some(&accent));
        shadow.setShadowBlurRadius(4.0);
        // Attribute keys have the exact AppKit-prescribed value types.
        unsafe {
            let mut keys = vec![NSFontAttributeName, NSForegroundColorAttributeName];
            let mut objects: Vec<&AnyObject> = vec![&font, foreground];
            if config.label_glow && cell.matched {
                keys.push(NSShadowAttributeName);
                objects.push(&shadow);
            }
            let attrs = NSDictionary::from_slices(&keys, &objects);
            let size = label.sizeWithAttributes(Some(&attrs));
            let label_rect = rect(
                x + (width - size.width) / 2.0,
                y + (height - size.height) / 2.0,
                size.width,
                size.height,
            );
            if config.high_contrast_labels && cell.matched {
                background.setFill();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(
                        label_rect.origin.x - 5.0,
                        label_rect.origin.y - 2.0,
                        size.width + 10.0,
                        size.height + 4.0,
                    ),
                    3.0,
                    3.0,
                )
                .fill();
            }
            label.drawInRect_withAttributes(label_rect, Some(&attrs));
        }
    }
    drop(focus);
    image
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_negative_and_above_primary_coordinates_in_points() {
        let frame = appkit_frame(
            Rect {
                x: -1920,
                y: -1080,
                width: 1920,
                height: 1080,
            },
            900.0,
        );
        assert_eq!(frame.origin.x, -1920.0);
        assert_eq!(frame.origin.y, 900.0);
        assert_eq!(frame.size.width, 1920.0);
        let frame = appkit_frame(
            Rect {
                x: 0,
                y: 0,
                width: 1440,
                height: 900,
            },
            900.0,
        );
        assert_eq!(frame.origin.y, 0.0);
    }
}
