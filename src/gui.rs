use crate::config::{Config, LabelStyle, PostHint};
use anyhow::{Context, Result};
use eframe::egui::{self, Color32, RichText};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Appearance,
    Controls,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MovementPreset {
    Vim,
    ArrowIjkl,
    Custom,
}

pub fn run(
    config_path: PathBuf,
    config: Config,
    config_tx: crossbeam_channel::Sender<Config>,
) -> Result<()> {
    let logo = logo_pixels(64)?;
    let window_logo = logo.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 640.0])
            .with_min_inner_size([620.0, 520.0])
            .with_title("kbmouse settings")
            .with_icon(egui::IconData {
                rgba: window_logo,
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "kbmouse settings",
        options,
        Box::new(move |creation| {
            Ok(Box::new(SettingsApp::new(
                creation,
                config_path,
                config,
                config_tx,
                logo,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!("settings window failed: {error}"))
}

struct SettingsApp {
    config_path: PathBuf,
    config_tx: crossbeam_channel::Sender<Config>,
    draft: Config,
    logo: egui::TextureHandle,
    page: Page,
    status: String,
    status_error: bool,
    #[cfg(windows)]
    quitting: bool,
    #[cfg(windows)]
    tray: Option<TrayState>,
}

impl SettingsApp {
    fn new(
        creation: &eframe::CreationContext<'_>,
        config_path: PathBuf,
        config: Config,
        config_tx: crossbeam_channel::Sender<Config>,
        logo: Vec<u8>,
    ) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(14, 20, 31);
        visuals.window_fill = Color32::from_rgb(20, 28, 42);
        visuals.selection.bg_fill = Color32::from_rgb(14, 165, 233);
        visuals.hyperlink_color = Color32::from_rgb(56, 189, 248);
        creation.egui_ctx.set_visuals(visuals);
        creation.egui_ctx.set_zoom_factor(1.2);
        let logo = creation.egui_ctx.load_texture(
            "kbmouse-logo",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &logo),
            egui::TextureOptions::LINEAR,
        );

        let mut style = (*creation.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        creation.egui_ctx.set_style_of(egui::Theme::Dark, style);

        #[cfg(windows)]
        let (tray, status, status_error) = match TrayState::new() {
            Ok(tray) => (Some(tray), "Settings loaded".to_owned(), false),
            Err(error) => (None, format!("Tray unavailable: {error}"), true),
        };
        #[cfg(not(windows))]
        let (status, status_error) = ("Settings loaded".to_owned(), false);

        Self {
            config_path,
            config_tx,
            draft: config,
            logo,
            page: Page::General,
            status,
            status_error,
            #[cfg(windows)]
            quitting: false,
            #[cfg(windows)]
            tray,
        }
    }

    fn save(&mut self) {
        match self.draft.save(&self.config_path) {
            Ok(()) => {
                if self.config_tx.send(self.draft.clone()).is_ok() {
                    self.status = "Saved and applied.".into();
                    self.status_error = false;
                } else {
                    self.status = "Saved, but the input runtime is no longer running.".into();
                    self.status_error = true;
                }
            }
            Err(error) => {
                self.status = format!("Could not save: {error:#}");
                self.status_error = true;
            }
        }
    }

    fn sidebar(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("navigation")
            .default_size(220.0)
            .min_size(220.0)
            .max_size(220.0)
            .resizable(false)
            .frame(
                egui::Frame::side_top_panel(root.style())
                    .fill(Color32::from_rgb(10, 15, 24))
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(root, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::from_texture(&self.logo)
                            .fit_to_exact_size(egui::vec2(44.0, 44.0)),
                    );
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new("kbmouse")
                                    .size(26.0)
                                    .strong()
                                    .color(Color32::from_rgb(125, 211, 252)),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                        ui.label(RichText::new("Keyboard mouse").color(Color32::from_gray(145)));
                    });
                });
                ui.add_space(28.0);
                nav_button(ui, &mut self.page, Page::General, "General");
                nav_button(ui, &mut self.page, Page::Appearance, "Appearance");
                nav_button(ui, &mut self.page, Page::Controls, "Controls");
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new("Running in the background")
                            .small()
                            .color(Color32::from_rgb(74, 222, 128)),
                    );
                });
            });
    }

    fn general(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "General",
            "Choose how kbmouse activates and what happens after a hint.",
        );
        card(ui, |ui| {
            section_title(ui, "Activation");
            setting_row(
                ui,
                "Leader key",
                "Tap this key to display the hint grid.",
                |ui| {
                    egui::ComboBox::from_id_salt("leader")
                        .selected_text(&self.draft.leader)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.draft.leader,
                                "capslock".into(),
                                "Caps Lock",
                            );
                            ui.selectable_value(&mut self.draft.leader, "f9".into(), "F9");
                        });
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Hold for Normal Mode",
                "Hold the leader as a momentary mouse-control layer.",
                |ui| {
                    ui.checkbox(&mut self.draft.hold_leader_for_normal, "");
                },
            );
            ui.separator();
            ui.add_enabled_ui(self.draft.hold_leader_for_normal, |ui| {
                setting_row(
                    ui,
                    "Tap threshold",
                    "A shorter press opens the grid; a longer unused hold does nothing.",
                    |ui| {
                        ui.add(
                            egui::Slider::new(&mut self.draft.leader_tap_ms, 100..=500)
                                .suffix(" ms")
                                .show_value(true),
                        );
                    },
                );
            });
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            section_title(ui, "Hint behavior");
            setting_row(
                ui,
                "After selecting a hint",
                "Continue controlling the pointer, click immediately, or return to the keyboard.",
                |ui| {
                    egui::ComboBox::from_id_salt("post_hint")
                        .selected_text(match self.draft.post_hint {
                            PostHint::Normal => "Enter Normal Mode",
                            PostHint::Click => "Left click",
                            PostHint::Exit => "Return to keyboard",
                        })
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.draft.post_hint,
                                PostHint::Normal,
                                "Enter Normal Mode",
                            );
                            ui.selectable_value(
                                &mut self.draft.post_hint,
                                PostHint::Click,
                                "Left click",
                            );
                            ui.selectable_value(
                                &mut self.draft.post_hint,
                                PostHint::Exit,
                                "Return to keyboard",
                            );
                        });
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Exit after click",
                "Release keyboard capture after clicking in persistent Normal Mode.",
                |ui| {
                    ui.checkbox(&mut self.draft.exit_on_click, "");
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Span all monitors",
                "Use one grid across the virtual desktop instead of the active monitor.",
                |ui| {
                    ui.checkbox(&mut self.draft.span_all_monitors, "");
                },
            );
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            section_title(ui, "Grid");
            setting_row(
                ui,
                "Hint labels",
                "Use compact key sequences or recognizable three-letter words.",
                |ui| {
                    egui::ComboBox::from_id_salt("label_style")
                        .selected_text(match self.draft.label_style {
                            LabelStyle::Sequences => "Key sequences",
                            LabelStyle::Words => "Three-letter words",
                        })
                        .width(165.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.draft.label_style,
                                LabelStyle::Sequences,
                                "Key sequences",
                            );
                            ui.selectable_value(
                                &mut self.draft.label_style,
                                LabelStyle::Words,
                                "Three-letter words",
                            );
                        });
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Target cell size",
                "Automatic grids aim for cells around this size.",
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.target_cell_px, 50..=200).suffix(" px"),
                    );
                },
            );
            ui.separator();
            let mut automatic = self.draft.grid_rows.is_none() && self.draft.grid_cols.is_none();
            setting_row(
                ui,
                "Automatic dimensions",
                "Adapt rows and columns to the active screen.",
                |ui| {
                    if ui.checkbox(&mut automatic, "").changed() {
                        if automatic {
                            self.draft.grid_rows = None;
                            self.draft.grid_cols = None;
                        } else {
                            self.draft.grid_rows = Some(10);
                            self.draft.grid_cols = Some(18);
                        }
                    }
                },
            );
            if !automatic {
                ui.horizontal(|ui| {
                    ui.label("Rows");
                    ui.add(
                        egui::DragValue::new(self.draft.grid_rows.get_or_insert(10)).range(2..=40),
                    );
                    ui.add_space(20.0);
                    ui.label("Columns");
                    ui.add(
                        egui::DragValue::new(self.draft.grid_cols.get_or_insert(18)).range(2..=60),
                    );
                });
            }
        });
    }

    fn appearance(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Appearance",
            "Tune the overlay for readability without obscuring your desktop.",
        );
        card(ui, |ui| {
            section_title(ui, "Overlay");
            setting_row(
                ui,
                "Backdrop opacity",
                "Controls how strongly the screen is dimmed.",
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.backdrop_opacity, 20..=220)
                            .show_value(true),
                    );
                },
            );
            ui.separator();
            setting_row(ui, "Label size", "Text size in screen pixels.", |ui| {
                ui.add(egui::Slider::new(&mut self.draft.font_size, 12..=48).suffix(" px"));
            });
            ui.separator();
            setting_row(
                ui,
                "High-contrast labels",
                "Use heavier key text with a dark backing badge for readability.",
                |ui| {
                    ui.checkbox(&mut self.draft.high_contrast_labels, "");
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Crisp labels",
                "Disable font smoothing for sharper, fully saturated glyph pixels.",
                |ui| {
                    ui.checkbox(&mut self.draft.crisp_labels, "");
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Label glow",
                "Add an accent-colored halo around hint text.",
                |ui| {
                    ui.checkbox(&mut self.draft.label_glow, "");
                },
            );
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            section_title(ui, "Colors");
            color_setting(ui, "Background", &mut self.draft.background_color);
            ui.separator();
            color_setting(ui, "Grid lines", &mut self.draft.grid_color);
            ui.separator();
            color_setting(ui, "Labels", &mut self.draft.text_color);
            ui.separator();
            color_setting(ui, "Active match", &mut self.draft.accent_color);
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            "Controls",
            "Customize pointer movement and Normal Mode bindings.",
        );
        card(ui, |ui| {
            section_title(ui, "Movement");
            setting_row(
                ui,
                "Smooth acceleration",
                "Experimental: precise taps with velocity ramp-up while holding direction keys.",
                |ui| {
                    ui.checkbox(&mut self.draft.smooth_movement, "");
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Normal Mode speed",
                "Pixels moved by each key repeat after selecting a hint.",
                |ui| {
                    ui.add(egui::Slider::new(&mut self.draft.move_step, 1..=40).suffix(" px"));
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Held-leader speed",
                "Faster movement while Caps Lock is held in momentary Normal Mode.",
                |ui| {
                    ui.add(egui::Slider::new(&mut self.draft.hold_move_step, 4..=80).suffix(" px"));
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Scroll step",
                "Wheel delta for each scroll key.",
                |ui| {
                    ui.add(egui::Slider::new(&mut self.draft.scroll_step, 30..=480));
                },
            );
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            section_title(ui, "Normal Mode bindings");
            let current_preset = movement_preset(&self.draft);
            let mut selected_preset = current_preset;
            setting_row(
                ui,
                "Movement layout",
                "Apply a familiar directional-key preset.",
                |ui| {
                    egui::ComboBox::from_id_salt("movement_preset")
                        .selected_text(movement_preset_name(current_preset))
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut selected_preset,
                                MovementPreset::Vim,
                                "Vim HJKL",
                            );
                            ui.selectable_value(
                                &mut selected_preset,
                                MovementPreset::ArrowIjkl,
                                "Arrow-style IJKL",
                            );
                            if current_preset == MovementPreset::Custom {
                                ui.selectable_value(
                                    &mut selected_preset,
                                    MovementPreset::Custom,
                                    "Custom",
                                );
                            }
                        });
                },
            );
            if selected_preset != current_preset {
                apply_movement_preset(&mut self.draft, selected_preset);
            }
            ui.separator();
            egui::Grid::new("bindings")
                .num_columns(4)
                .spacing([14.0, 12.0])
                .show(ui, |ui| {
                    key_field(ui, "Left", &mut self.draft.keys.left);
                    key_field(ui, "Down", &mut self.draft.keys.down);
                    ui.end_row();
                    key_field(ui, "Up", &mut self.draft.keys.up);
                    key_field(ui, "Right", &mut self.draft.keys.right);
                    ui.end_row();
                    key_field(ui, "Left click", &mut self.draft.keys.left_click);
                    key_field(ui, "Middle click", &mut self.draft.keys.middle_click);
                    ui.end_row();
                    key_field(ui, "Right click", &mut self.draft.keys.right_click);
                    key_field(ui, "Drag", &mut self.draft.keys.drag);
                    ui.end_row();
                    key_field(ui, "Scroll up", &mut self.draft.keys.scroll_up);
                    key_field(ui, "Scroll down", &mut self.draft.keys.scroll_down);
                    ui.end_row();
                    key_field(ui, "Subdivide", &mut self.draft.keys.subdivide);
                });
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            section_title(ui, "Hint alphabet");
            ui.label(
                RichText::new("Unique characters used to generate fixed-length hint labels.")
                    .color(Color32::from_gray(155)),
            );
            ui.add_enabled_ui(self.draft.label_style == LabelStyle::Sequences, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.alphabet)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            });
            if self.draft.label_style == LabelStyle::Words {
                ui.label(
                    RichText::new("Word labels use the built-in three-letter dictionary.")
                        .small()
                        .color(Color32::from_gray(125)),
                );
            }
        });
    }

    #[cfg(windows)]
    fn handle_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &self.tray else {
            return;
        };
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                tray_icon::TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }
            ) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if event.id == tray.quit_id {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        #[cfg(windows)]
        self.handle_tray(&ctx);

        #[cfg(windows)]
        if !self.quitting
            && self.tray.is_some()
            && ctx.input(|input| input.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        self.sidebar(root);
        egui::Panel::bottom("actions")
            .frame(
                egui::Frame::side_top_panel(root.style())
                    .fill(Color32::from_rgb(14, 20, 31))
                    .inner_margin(egui::Margin::symmetric(24, 14)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let color = if self.status_error {
                        Color32::from_rgb(248, 113, 113)
                    } else {
                        Color32::from_gray(155)
                    };
                    ui.label(RichText::new(&self.status).color(color));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Save settings").strong())
                                    .fill(Color32::from_rgb(2, 132, 199)),
                            )
                            .clicked()
                        {
                            self.save();
                        }
                        if ui.button("Reset defaults").clicked() {
                            self.draft = Config::default();
                            self.status = "Defaults restored locally; save to apply.".into();
                            self.status_error = false;
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(root.style()).inner_margin(egui::Margin::same(28)))
            .show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::General => self.general(ui),
                    Page::Appearance => self.appearance(ui),
                    Page::Controls => self.controls(ui),
                });
            });

        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

fn nav_button(ui: &mut egui::Ui, selected: &mut Page, page: Page, label: &str) {
    let active = *selected == page;
    let button = egui::Button::new(RichText::new(label).size(15.0))
        .fill(if active {
            Color32::from_rgb(24, 53, 74)
        } else {
            Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .min_size(egui::vec2(150.0, 38.0));
    if ui.add(button).clicked() {
        *selected = page;
    }
}

fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(28.0).strong());
    ui.label(RichText::new(subtitle).color(Color32::from_gray(155)));
    ui.add_space(20.0);
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .size(16.0)
            .strong()
            .color(Color32::from_rgb(186, 230, 253)),
    );
    ui.add_space(4.0);
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(20, 28, 42))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(18))
        .show(ui, content);
}

fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            ui.label(
                RichText::new(description)
                    .small()
                    .color(Color32::from_gray(145)),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

fn color_setting(ui: &mut egui::Ui, title: &str, value: &mut String) {
    setting_row(ui, title, "Hex RGB color", |ui| {
        ui.add_sized([105.0, 28.0], egui::TextEdit::singleline(value));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            6.0,
            parse_hex_color(value).unwrap_or(Color32::DARK_GRAY),
        );
    });
}

fn key_field(ui: &mut egui::Ui, title: &str, value: &mut String) {
    ui.label(title);
    ui.add_sized(
        [80.0, 26.0],
        egui::TextEdit::singleline(value).font(egui::TextStyle::Monospace),
    );
}

fn parse_hex_color(value: &str) -> Option<Color32> {
    let rgb = u32::from_str_radix(value.trim_start_matches('#'), 16).ok()?;
    Some(Color32::from_rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    ))
}

fn movement_preset(config: &Config) -> MovementPreset {
    let keys = &config.keys;
    if keys.left == "h" && keys.down == "j" && keys.up == "k" && keys.right == "l" {
        MovementPreset::Vim
    } else if keys.left == "j" && keys.down == "k" && keys.up == "i" && keys.right == "l" {
        MovementPreset::ArrowIjkl
    } else {
        MovementPreset::Custom
    }
}

fn movement_preset_name(preset: MovementPreset) -> &'static str {
    match preset {
        MovementPreset::Vim => "Vim HJKL",
        MovementPreset::ArrowIjkl => "Arrow-style IJKL",
        MovementPreset::Custom => "Custom",
    }
}

fn apply_movement_preset(config: &mut Config, preset: MovementPreset) {
    match preset {
        MovementPreset::Vim => {
            config.keys.left = "h".into();
            config.keys.down = "j".into();
            config.keys.up = "k".into();
            config.keys.right = "l".into();
        }
        MovementPreset::ArrowIjkl => {
            config.keys.left = "j".into();
            config.keys.down = "k".into();
            config.keys.up = "i".into();
            config.keys.right = "l".into();
        }
        MovementPreset::Custom => {}
    }
}

fn logo_pixels(size: u32) -> Result<Vec<u8>> {
    let decoded = image::load_from_memory(include_bytes!("../assets/logo.png"))
        .context("failed to decode embedded logo.png")?
        .into_rgba8();
    Ok(
        image::imageops::resize(&decoded, size, size, image::imageops::FilterType::Lanczos3)
            .into_raw(),
    )
}

#[cfg(windows)]
struct TrayState {
    _icon: tray_icon::TrayIcon,
    quit_id: tray_icon::menu::MenuId,
}

#[cfg(windows)]
impl TrayState {
    fn new() -> Result<Self> {
        use tray_icon::{
            TrayIconBuilder,
            menu::{Menu, MenuItem},
        };

        let menu = Menu::new();
        let quit = MenuItem::new("Quit kbmouse", true, None);
        menu.append(&quit).context("failed to build tray menu")?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("kbmouse")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(tray_icon_image()?)
            .build()
            .context("failed to create tray icon")?;
        Ok(Self {
            _icon: icon,
            quit_id: quit.id().clone(),
        })
    }
}

#[cfg(windows)]
fn tray_icon_image() -> Result<tray_icon::Icon> {
    let size = 32u32;
    tray_icon::Icon::from_rgba(logo_pixels(size)?, size, size)
        .context("failed to create tray icon image")
}
