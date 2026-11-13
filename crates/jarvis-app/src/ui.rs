use jarvis_core::models;
use jarvis_core::setup::{SetupItem, SetupStatus};
use jarvis_core::ThemeChoice;
use jarvis_models::manager::{ModelKind, ModelPhase};
use rune::tok::{self, radius, space, type_, Theme};
use rune::{col, el, oklch, row, text, Color, Cx, El, ScrollSpec, TextStyle};

use crate::{Jarvis, Notice, NoticeTone, Section, SettingsPage};

const PAGE_PAD: f32 = 28.0;

#[derive(Clone, Copy)]
struct Paint {
    t: Theme,
    accent: Color,
    hair: f32,
    dark: bool,
}

impl Paint {
    fn new(state: &Jarvis, cx: &Cx<Jarvis>) -> Self {
        let dark = state.dark(cx.system_dark());
        Self {
            t: tok::theme(dark),
            accent: tok::accent(state.settings.accent as usize, dark),
            hair: 1.0 / cx.env().scale,
            dark,
        }
    }
}

pub fn view(state: &Jarvis, cx: &mut Cx<Jarvis>) -> El {
    let p = Paint::new(state, cx);
    row()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(p.t.bg)
        .child(sidebar(state, p))
        .child(
            el().key("sidebar-divider")
                .w(1.0)
                .h_full()
                .fill(p.t.separator),
        )
        .child(main(state, p))
}

fn main(state: &Jarvis, p: Paint) -> El {
    let page = match state.section {
        Section::Assistant => assistant_page(state, p),
        Section::Voice | Section::Routines | Section::Insights => {
            placeholder_page(state.section, p)
        }
        Section::Appearance => appearance_page(state, p),
        Section::Settings => settings_page(state, p),
    };

    let mut column = col()
        .key("main")
        .grow(1.0)
        .min_h(0.0)
        .h_full()
        .child(header(state, p));

    if let Some(notice) = &state.notice {
        column = column.child(notice_bar(p, notice));
    }
    if let Some(error) = &state.setup_error {
        column = column.child(error_bar(p, error));
    }

    column.child(
        col()
            .key("page-scroll")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("page")
                    .w_full()
                    .px(PAGE_PAD)
                    .py(space::S6)
                    .child(page),
            ),
    )
}

fn header(state: &Jarvis, p: Paint) -> El {
    let title = match state.section {
        Section::Settings => state.settings_page.label(),
        section => section.label(),
    };
    row()
        .key("header")
        .h(64.0)
        .w_full()
        .px(PAGE_PAD)
        .center_y()
        .justify_between()
        .gap(16.0)
        .child(
            col()
                .gap(2.0)
                .min_w(0.0)
                .child(
                    text("JARVIS / WINDOWS")
                        .font(type_::SECTION)
                        .color(p.t.text_secondary),
                )
                .child(
                    text(title)
                        .font(TextStyle::new(20.0, 600.0))
                        .color(p.t.text),
                ),
        )
        .child(header_badge(state, p))
}

fn header_badge(state: &Jarvis, p: Paint) -> El {
    let (label, color) = if state.setup_running {
        ("Checking setup".to_string(), p.accent)
    } else if state.setup.essentials_complete() {
        ("Setup complete".to_string(), success_color())
    } else {
        (
            format!(
                "{} of {} ready",
                state.setup.essentials_ready, state.setup.essentials_total
            ),
            warning_color(),
        )
    };

    row()
        .key("header-badge")
        .h(28.0)
        .px(10.0)
        .radius(14.0)
        .gap(8.0)
        .center_y()
        .fill(color.alpha(0.13))
        .child(el().size(6.0, 6.0).radius(3.0).fill(color).a11y_hidden())
        .child(text(label).font(type_::LABEL).color(color))
}

fn notice_bar(p: Paint, notice: &Notice) -> El {
    let color = match notice.tone {
        NoticeTone::Info => p.accent,
        NoticeTone::Warning => warning_color(),
        NoticeTone::Error => danger_color(),
    };
    row()
        .key("notice")
        .w_full()
        .px(PAGE_PAD)
        .pt(space::S3)
        .gap(18.0)
        .center_y()
        .justify_between()
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .center_y()
                .child(el().size(8.0, 8.0).radius(4.0).fill(color).a11y_hidden())
                .child(
                    text(notice.message.clone())
                        .font(type_::BODY)
                        .color(p.t.text),
                ),
        )
        .child(link_button(
            p,
            "notice-dismiss",
            "Dismiss",
            Jarvis::dismiss_notice,
        ))
}

fn error_bar(p: Paint, message: &str) -> El {
    row()
        .key("setup-error")
        .w_full()
        .px(PAGE_PAD)
        .pt(space::S3)
        .gap(10.0)
        .center_y()
        .child(
            el().size(8.0, 8.0)
                .radius(4.0)
                .fill(danger_color())
                .a11y_hidden(),
        )
        .child(text(message.to_string()).font(type_::BODY).color(p.t.text))
}

fn sidebar(state: &Jarvis, p: Paint) -> El {
    let mut nav = col().key("nav").w_full().gap(2.0);
    for section in Section::ALL {
        nav = nav.child(nav_item(state, p, section));
    }

    col()
        .key("sidebar")
        .w(228.0)
        .h_full()
        .px(14.0)
        .py(18.0)
        .child(brand(p))
        .child(el().key("brand-space").h(18.0))
        .child(nav)
        .child(el().key("nav-spacer").grow(1.0))
        .child(sidebar_footer(state, p))
}

fn brand(p: Paint) -> El {
    row()
        .key("brand")
        .w_full()
        .px(8.0)
        .gap(11.0)
        .center_y()
        .child(
            el().key("logo")
                .size(30.0, 30.0)
                .radius(9.0)
                .fill(p.accent)
                .center()
                .shadow(1.0, 6.0, p.accent.alpha(0.35))
                .child(
                    text("J")
                        .font(TextStyle::new(15.0, 700.0))
                        .color(Color::WHITE),
                ),
        )
        .child(
            col()
                .gap(1.0)
                .min_w(0.0)
                .child(
                    text("JARVIS")
                        .font(TextStyle::new(14.0, 700.0))
                        .color(p.t.text),
                )
                .child(
                    text("Windows build")
                        .font(type_::LABEL)
                        .color(p.t.text_secondary),
                ),
        )
}

fn nav_item(state: &Jarvis, p: Paint, section: Section) -> El {
    let active = state.section == section;
    let key = format!("nav-{}", section.key());
    let mut item = row()
        .key(key)
        .h(38.0)
        .w_full()
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(radius::MD)
        .focusable(true)
        .role("button")
        .label(section.label())
        .on_press(move |state: &mut Jarvis| state.navigate(section));

    item = if active {
        item.fill(p.accent.alpha(0.14))
            .hover(p.accent.alpha(0.18))
            .press(p.accent.alpha(0.22))
    } else {
        item.hover(p.t.hover).press(p.t.press)
    };

    item.child(
        el().size(6.0, 6.0)
            .radius(3.0)
            .fill(if active {
                p.accent
            } else {
                p.t.text_secondary.alpha(0.35)
            })
            .a11y_hidden(),
    )
    .child(text(section.label()).font(type_::BODY).color(if active {
        p.t.text
    } else {
        p.t.text_secondary
    }))
}

fn sidebar_footer(state: &Jarvis, p: Paint) -> El {
    col()
        .key("sidebar-footer")
        .w_full()
        .gap(8.0)
        .child(
            el().key("footer-rule")
                .h(p.hair)
                .w_full()
                .fill(p.t.separator),
        )
        .child(
            row()
                .px(8.0)
                .justify_between()
                .gap(10.0)
                .child(
                    text(format!("v{} · phase 2", state.version))
                        .font(TextStyle::new(11.0, 500.0))
                        .color(p.t.text_secondary),
                )
                .child(
                    text("local")
                        .font(TextStyle::new(11.0, 500.0))
                        .color(p.t.text_secondary.alpha(0.7)),
                ),
        )
}

fn assistant_page(state: &Jarvis, p: Paint) -> El {
    let (headline, color, detail) = if state.setup_running {
        (
            "Checking this machine",
            p.accent,
            "Looking at microphone access, models and acceleration.".to_string(),
        )
    } else if state.setup.essentials_complete() {
        (
            "Jarvis is ready",
            success_color(),
            "Speech and intent models are installed. The voice pipeline arrives in the next phases."
                .to_string(),
        )
    } else {
        (
            "Finish setup",
            warning_color(),
            format!(
                "{} of {} essentials are ready. Open setup to see what is missing.",
                state.setup.essentials_ready, state.setup.essentials_total
            ),
        )
    };

    col()
        .key("assistant-page")
        .w_full()
        .items_center()
        .gap(26.0)
        .py(space::S6)
        .child(orb(p))
        .child(
            col()
                .items_center()
                .gap(8.0)
                .child(
                    text("Jarvis for Windows")
                        .font(TextStyle::new(30.0, 650.0))
                        .color(p.t.text),
                )
                .child(
                    text("Speech, intent and command routing stay on this machine.")
                        .font(TextStyle::new(14.0, 400.0))
                        .color(p.t.text_secondary),
                ),
        )
        .child(
            col()
                .key("assistant-cards")
                .w_full()
                .max_w(640.0)
                .gap(12.0)
                .child(
                    card(p, "assistant-status").child(
                        row()
                            .gap(16.0)
                            .center_y()
                            .child(
                                col()
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .gap(4.0)
                                    .child(
                                        row()
                                            .gap(8.0)
                                            .center_y()
                                            .child(
                                                el().size(8.0, 8.0)
                                                    .radius(4.0)
                                                    .fill(color)
                                                    .a11y_hidden(),
                                            )
                                            .child(
                                                text(headline)
                                                    .font(TextStyle::new(15.0, 600.0))
                                                    .color(p.t.text),
                                            ),
                                    )
                                    .child(
                                        text(detail).font(type_::BODY).color(p.t.text_secondary),
                                    ),
                            )
                            .child(secondary_button(
                                p,
                                "assistant-open-setup",
                                if state.setup.essentials_complete() {
                                    "Review setup"
                                } else {
                                    "Open setup"
                                },
                                true,
                                |state| state.open_settings(SettingsPage::Setup),
                            )),
                    ),
                )
                .child(capability_row(p)),
        )
}

fn orb(p: Paint) -> El {
    el().key("orb")
        .size(196.0, 196.0)
        .radius(98.0)
        .fill(p.accent.alpha(0.07))
        .shadow(0.0, 40.0, p.accent.alpha(0.14))
        .center()
        .child(
            el().size(132.0, 132.0)
                .radius(66.0)
                .fill(p.accent.alpha(0.13))
                .center()
                .child(
                    el().size(72.0, 72.0)
                        .radius(36.0)
                        .fill(p.accent)
                        .shadow(0.0, 18.0, p.accent.alpha(0.45))
                        .center()
                        .child(
                            el().size(18.0, 18.0)
                                .radius(9.0)
                                .fill(Color::WHITE.alpha(0.85)),
                        ),
                ),
        )
}

fn capability_row(p: Paint) -> El {
    row()
        .w_full()
        .gap(12.0)
        .child(capability_card(
            p,
            "Speech",
            "Whisper large-v3-turbo",
            "Local transcription",
        ))
        .child(capability_card(
            p,
            "Intent",
            "t5-small · int8 ONNX",
            "Managed release",
        ))
        .child(capability_card(
            p,
            "Acceleration",
            "Vulkan + DirectML",
            "GPU first, CPU fallback",
        ))
}

fn capability_card(p: Paint, title: &str, value: &str, detail: &str) -> El {
    col()
        .grow(1.0)
        .min_w(0.0)
        .padding(14.0)
        .gap(5.0)
        .fill(p.t.card)
        .radius(radius::MD)
        .border(p.hair, p.t.card_border)
        .child(text(title).font(type_::SECTION).color(p.t.text_secondary))
        .child(text(value).font(type_::BODY).color(p.t.text))
        .child(text(detail).font(type_::LABEL).color(p.t.text_secondary))
}

fn settings_page(state: &Jarvis, p: Paint) -> El {
    let content = match state.settings_page {
        SettingsPage::Setup => setup_page(state, p),
        SettingsPage::General => general_page(state, p),
        SettingsPage::About => about_page(state, p),
    };

    col()
        .key("settings-page")
        .w_full()
        .gap(20.0)
        .child(settings_tabs(state, p))
        .child(content)
}

fn settings_tabs(state: &Jarvis, p: Paint) -> El {
    let mut tabs = row().key("settings-tabs").gap(4.0);
    for page in SettingsPage::ALL {
        let active = state.settings_page == page;
        let mut tab = el()
            .key(format!("settings-tab-{}", page.key()))
            .h(30.0)
            .px(13.0)
            .radius(radius::MD)
            .center()
            .focusable(true)
            .role("button")
            .label(page.label())
            .on_press(move |state: &mut Jarvis| state.open_settings(page));
        tab = if active {
            tab.fill(p.accent.alpha(0.16))
        } else {
            tab.hover(p.t.hover)
        };
        tabs = tabs.child(
            tab.child(text(page.label()).font(type_::LABEL).color(if active {
                p.accent
            } else {
                p.t.text_secondary
            })),
        );
    }
    tabs
}

fn setup_page(state: &Jarvis, p: Paint) -> El {
    col()
        .key("setup-page")
        .w_full()
        .max_w(820.0)
        .gap(16.0)
        .child(setup_hero(state, p))
        .child(setup_items(state, p))
        .child(setup_finish(state, p))
}

fn setup_hero(state: &Jarvis, p: Paint) -> El {
    let fraction = if state.setup.essentials_total == 0 {
        0.0
    } else {
        state.setup.essentials_ready as f32 / state.setup.essentials_total as f32
    };

    card(p, "setup-hero")
        .child(
            row()
                .justify_between()
                .center_y()
                .gap(20.0)
                .child(
                    col()
                        .gap(5.0)
                        .min_w(0.0)
                        .child(
                            text("First-run setup")
                                .font(TextStyle::new(22.0, 650.0))
                                .color(p.t.text),
                        )
                        .child(
                            text(
                                "Jarvis keeps speech, intent and command history on this machine.",
                            )
                            .font(type_::BODY)
                            .color(p.t.text_secondary),
                        ),
                )
                .child(if state.setup_running {
                    secondary_button(p, "refresh-setup", "Checking...", false, |_| {})
                } else {
                    secondary_button(p, "refresh-setup", "Check again", true, |state| {
                        state.refresh_setup()
                    })
                }),
        )
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(progress_bar(p, "setup-progress", 260.0, fraction))
                .child(
                    text(format!(
                        "{} of {} essentials ready",
                        state.setup.essentials_ready, state.setup.essentials_total
                    ))
                    .font(type_::LABEL)
                    .color(p.t.text_secondary),
                ),
        )
}

fn progress_bar(p: Paint, key: &str, width: f32, fraction: f32) -> El {
    let fraction = fraction.clamp(0.0, 1.0);
    row()
        .key(key.to_string())
        .h(6.0)
        .w(width)
        .radius(3.0)
        .fill(p.t.control_off)
        .child(
            el().h(6.0)
                .w((width * fraction).max(if fraction > 0.0 { 6.0 } else { 0.0 }))
                .radius(3.0)
                .fill(p.accent)
                .motion(tok::motion::SNAPPY),
        )
}

fn setup_items(state: &Jarvis, p: Paint) -> El {
    let mut list = col().key("setup-list").w_full();

    for (index, item) in state.setup.items.iter().enumerate() {
        if index > 0 {
            list = list.child(
                el().key(format!("setup-separator-{index}"))
                    .h(p.hair)
                    .w_full()
                    .fill(p.t.separator),
            );
        }
        list = list.child(setup_row(state, p, item));
    }

    card(p, "setup-items").child(list)
}

fn setup_row(state: &Jarvis, p: Paint, item: &SetupItem) -> El {
    let kind = match item.kind {
        jarvis_core::SetupKind::SpeechModel => Some(ModelKind::Speech),
        jarvis_core::SetupKind::IntentModel => Some(ModelKind::Intent),
        _ => None,
    };
    let update = kind.map(|kind| state.models.get(kind));
    let model_owns_display = update.is_some_and(|update| {
        update.phase.is_busy()
            || matches!(
                update.phase,
                ModelPhase::Ready | ModelPhase::Error | ModelPhase::Cancelled
            )
    });
    let status = if model_owns_display {
        update
            .map(|update| model_status(update.phase))
            .unwrap_or(item.status)
    } else {
        item.status
    };
    let status_label = if model_owns_display {
        update
            .map(|update| update.phase.label())
            .unwrap_or(item.status.label())
    } else {
        item.status.label()
    };
    let detail = if model_owns_display {
        update
            .map(|update| update.detail.clone())
            .unwrap_or_else(|| item.detail.clone())
    } else {
        item.detail.clone()
    };
    let color = status_color(p, status);
    let key = format!("setup-row-{}", item.key());

    let mut line = row()
        .key(key)
        .w_full()
        .px(16.0)
        .py(14.0)
        .gap(14.0)
        .center_y()
        .child(el().size(10.0, 10.0).radius(5.0).fill(color).a11y_hidden());

    let mut text_column = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    text(item.title())
                        .font(TextStyle::new(13.5, 550.0))
                        .color(p.t.text),
                )
                .child(
                    row()
                        .h(20.0)
                        .px(7.0)
                        .radius(10.0)
                        .center()
                        .fill(color.alpha(0.14))
                        .child(text(status_label).font(type_::LABEL).color(color)),
                ),
        )
        .child(
            text(detail)
                .w_full()
                .font(type_::LABEL)
                .color(p.t.text_secondary),
        );

    if let Some(update) = update {
        if update.phase.is_busy() {
            text_column = text_column.child(
                row()
                    .gap(10.0)
                    .center_y()
                    .child(progress_bar(
                        p,
                        &format!("model-progress-{}", update.kind.key()),
                        220.0,
                        update.fraction,
                    ))
                    .child(
                        text(format!("{:.0}%", update.fraction * 100.0))
                            .font(type_::LABEL)
                            .color(p.t.text_secondary),
                    ),
            );
        }
    }

    line = line.child(text_column);

    if let Some(kind) = kind {
        line = line.child(model_controls(state, p, kind));
    } else if let Some(label) = item.action.label() {
        let action = item.action;
        let key = format!("setup-action-{}", item.key());
        line = line.child(secondary_button(p, &key, label, true, move |state| {
            state.run_action(action)
        }));
    }

    line
}

fn model_status(phase: ModelPhase) -> SetupStatus {
    match phase {
        ModelPhase::Missing => SetupStatus::Missing,
        ModelPhase::Installed | ModelPhase::Ready => SetupStatus::Ready,
        ModelPhase::Downloading | ModelPhase::Verifying | ModelPhase::Loading => {
            SetupStatus::Checking
        }
        ModelPhase::Error => SetupStatus::Incomplete,
        ModelPhase::Cancelled => SetupStatus::Warning,
    }
}

fn model_controls(state: &Jarvis, p: Paint, kind: ModelKind) -> El {
    let update = state.models.get(kind);
    let key = kind.key();
    let mut controls = row().gap(6.0).center_y();

    match update.phase {
        ModelPhase::Missing => {
            let download_key = format!("download-{key}-model");
            controls = controls.child(secondary_button(
                p,
                &download_key,
                "Download",
                true,
                move |state| state.install_model(kind),
            ));
        }
        ModelPhase::Installed => {
            let load_key = format!("load-{key}-model");
            controls = controls.child(secondary_button(p, &load_key, "Load", true, move |state| {
                state.load_model(kind)
            }));
        }
        ModelPhase::Downloading | ModelPhase::Verifying | ModelPhase::Loading => {
            let cancel_key = format!("cancel-{key}-model");
            controls = controls.child(link_button(p, &cancel_key, "Cancel", move |state| {
                state.cancel_model(kind)
            }));
        }
        ModelPhase::Ready => {
            if let Some(backend) = &update.backend {
                controls = controls.child(
                    row()
                        .h(20.0)
                        .px(7.0)
                        .radius(10.0)
                        .center()
                        .fill(p.accent.alpha(0.12))
                        .child(
                            text(format!("Loaded · {backend}"))
                                .font(type_::LABEL)
                                .color(p.accent),
                        ),
                );
            }
            let load_key = format!("load-{key}-model");
            controls = controls.child(secondary_button(
                p,
                &load_key,
                "Load again",
                true,
                move |state| state.load_model(kind),
            ));
        }
        ModelPhase::Error => {
            let retry_key = format!("download-{key}-model");
            let load_key = format!("load-{key}-model");
            controls = controls
                .child(secondary_button(
                    p,
                    &retry_key,
                    "Retry",
                    true,
                    move |state| state.install_model(kind),
                ))
                .child(secondary_button(p, &load_key, "Load", true, move |state| {
                    state.load_model(kind)
                }));
        }
        ModelPhase::Cancelled => {
            let resume_key = format!("download-{key}-model");
            controls = controls.child(secondary_button(
                p,
                &resume_key,
                "Resume",
                true,
                move |state| state.install_model(kind),
            ));
        }
    }

    controls
}

fn setup_finish(state: &Jarvis, p: Paint) -> El {
    let hint = if state.settings.setup_completed {
        "Setup is already marked complete. You can run the checks again at any time."
    } else if state.setup.essentials_complete() {
        "Everything essential is ready. Finish setup to open the assistant."
    } else {
        "You can finish now and complete the remaining checks later."
    };

    card(p, "setup-finish").child(
        row()
            .justify_between()
            .center_y()
            .gap(24.0)
            .child(
                col()
                    .gap(4.0)
                    .min_w(0.0)
                    .child(
                        text("Ready to use Jarvis")
                            .font(TextStyle::new(15.0, 600.0))
                            .color(p.t.text),
                    )
                    .child(
                        text(hint.to_string())
                            .font(type_::LABEL)
                            .color(p.t.text_secondary),
                    ),
            )
            .child(primary_button(
                p,
                "finish-setup",
                "Finish setup",
                true,
                |state| state.finish_setup(),
            )),
    )
}

fn general_page(state: &Jarvis, p: Paint) -> El {
    let speech_path = models::whisper_model_path(&state.paths);
    let intent_path = state.paths.intent_dir.display().to_string();

    col()
        .key("general-page")
        .w_full()
        .max_w(820.0)
        .gap(16.0)
        .child(
            card(p, "storage-card")
                .child(section_heading(p, "Storage"))
                .child(info_row(
                    p,
                    "Data folder",
                    &state.paths.data_dir.display().to_string(),
                ))
                .child(info_row(
                    p,
                    "Speech model",
                    &speech_path.display().to_string(),
                ))
                .child(info_row(p, "Intent model", &intent_path))
                .child(row().gap(8.0).pt(4.0).child(secondary_button(
                    p,
                    "open-data-folder",
                    "Open data folder",
                    true,
                    Jarvis::open_data_folder,
                ))),
        )
        .child(
            card(p, "models-card")
                .child(section_heading(p, "Models & services"))
                .child(info_row(
                    p,
                    "Speech model",
                    "ggerganov/whisper.cpp · large-v3-turbo",
                ))
                .child(info_row(p, "Intent model", models::INTENT_MODEL_REPO))
                .child(info_row(p, "Local answers", "Ollama · optional"))
                .child(
                    row()
                        .gap(8.0)
                        .pt(4.0)
                        .child(secondary_button(
                            p,
                            "open-intent-model",
                            "Intent model on Hugging Face",
                            true,
                            Jarvis::open_intent_model_page,
                        ))
                        .child(secondary_button(
                            p,
                            "open-ollama",
                            "Get Ollama",
                            true,
                            Jarvis::open_ollama_page,
                        )),
                ),
        )
}

fn appearance_page(state: &Jarvis, p: Paint) -> El {
    let mut accents = row().key("accent-swatches").gap(10.0);
    for (index, name) in tok::ACCENT_NAMES.iter().enumerate() {
        let active = state.settings.accent as usize == index;
        let color = tok::accent(index, p.dark);
        let mut swatch = el()
            .key(format!("accent-{index}"))
            .size(22.0, 22.0)
            .radius(11.0)
            .fill(color)
            .focusable(true)
            .role("radio")
            .label(*name)
            .checked(active)
            .on_press(move |state: &mut Jarvis| state.set_accent(index as u8));
        if active {
            swatch = swatch.border(2.0, p.t.text);
        }
        accents = accents.child(swatch);
    }

    col()
        .key("appearance-page")
        .w_full()
        .max_w(820.0)
        .gap(16.0)
        .child(
            card(p, "appearance-card")
                .child(section_heading(p, "Appearance"))
                .child(
                    row()
                        .justify_between()
                        .center_y()
                        .gap(20.0)
                        .child(text("Theme").font(type_::BODY).color(p.t.text))
                        .child(theme_segments(state, p)),
                )
                .child(
                    row()
                        .justify_between()
                        .center_y()
                        .gap(20.0)
                        .child(text("Accent colour").font(type_::BODY).color(p.t.text))
                        .child(accents),
                ),
        )
        .child(
            card(p, "preview-card")
                .child(section_heading(p, "Preview"))
                .child(
                    row()
                        .gap(12.0)
                        .center_y()
                        .child(
                            el().size(34.0, 34.0)
                                .radius(17.0)
                                .fill(p.accent.alpha(0.16))
                                .center()
                                .child(el().size(10.0, 10.0).radius(5.0).fill(p.accent)),
                        )
                        .child(
                            col()
                                .gap(3.0)
                                .child(
                                    text("Jarvis is listening")
                                        .font(type_::BODY)
                                        .color(p.t.text),
                                )
                                .child(
                                    text("Local · private · offline")
                                        .font(type_::LABEL)
                                        .color(p.t.text_secondary),
                                ),
                        ),
                ),
        )
}

fn theme_segments(state: &Jarvis, p: Paint) -> El {
    let mut segments = row()
        .key("theme-segments")
        .gap(2.0)
        .padding(2.0)
        .radius(radius::MD)
        .fill(p.t.control_track);

    for theme in ThemeChoice::ALL {
        let active = state.settings.theme == theme;
        let key = format!("theme-{}", theme.label().to_lowercase());
        let mut segment = el()
            .key(key)
            .h(28.0)
            .px(14.0)
            .radius(radius::SM)
            .center()
            .focusable(true)
            .role("button")
            .label(theme.label())
            .on_press(move |state: &mut Jarvis| state.set_theme(theme));
        segment = if active {
            segment.fill(p.t.indicator)
        } else {
            segment.hover(p.t.hover)
        };
        segments = segments.child(segment.child(
            text(theme.label()).font(type_::LABEL).color(if active {
                p.t.text
            } else {
                p.t.text_secondary
            }),
        ));
    }

    segments
}

fn about_page(state: &Jarvis, p: Paint) -> El {
    col()
        .key("about-page")
        .w_full()
        .max_w(820.0)
        .gap(16.0)
        .child(
            card(p, "about-card")
                .child(section_heading(p, "About"))
                .child(info_row(p, "Application", "Jarvis for Windows"))
                .child(info_row(p, "Version", state.version))
                .child(info_row(p, "Build", "Phase 2 · model manager"))
                .child(info_row(p, "Platform", std::env::consts::OS))
                .child(info_row(
                    p,
                    "Data folder",
                    &state.paths.data_dir.display().to_string(),
                ))
                .child(
                    text("A native Rust port of the macOS Jarvis: Rune for the interface, whisper.cpp for speech, ONNX Runtime for the learned intent model.")
                        .font(type_::BODY)
                        .color(p.t.text_secondary),
                ),
        )
        .child(
            card(p, "about-links")
                .child(section_heading(p, "Links"))
                .child(info_row(
                    p,
                    "Intent model",
                    models::INTENT_MODEL_URL,
                ))
                .child(info_row(
                    p,
                    "Speech model",
                    models::WHISPER_MODEL_URL,
                ))
                .child(
                    row()
                        .gap(8.0)
                        .pt(4.0)
                        .child(secondary_button(
                            p,
                            "about-intent-model",
                            "Open intent model",
                            true,
                            Jarvis::open_intent_model_page,
                        ))
                        .child(secondary_button(
                            p,
                            "about-data-folder",
                            "Open data folder",
                            true,
                            Jarvis::open_data_folder,
                        )),
                ),
        )
}

fn placeholder_page(section: Section, p: Paint) -> El {
    col()
        .key(format!("placeholder-{}", section.key()))
        .w_full()
        .max_w(720.0)
        .gap(14.0)
        .child(
            text(section.label())
                .font(TextStyle::new(24.0, 650.0))
                .color(p.t.text),
        )
        .child(
            card(p, "placeholder-card")
                .child(
                    text("This screen arrives in a later build. Phase 2 ships the app shell, setup, verified model downloads and runtime warm-up.")
                        .font(type_::BODY)
                        .color(p.t.text_secondary),
                )
                .child(
                    row().pt(4.0).child(secondary_button(
                        p,
                        "placeholder-open-setup",
                        "Open setup",
                        true,
                        |state| state.open_settings(SettingsPage::Setup),
                    )),
                ),
        )
}

fn card(p: Paint, key: &str) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .padding(20.0)
        .gap(10.0)
        .fill(p.t.card)
        .radius(radius::LG)
        .border(p.hair, p.t.card_border)
        .shadow(1.0, 2.0, p.t.shadow)
        .shadow(8.0, 24.0, p.t.shadow_soft)
}

fn section_heading(p: Paint, label: &str) -> El {
    text(label.to_string())
        .font(type_::SECTION)
        .color(p.t.text_secondary)
}

fn info_row(p: Paint, label: &str, value: &str) -> El {
    row()
        .w_full()
        .py(7.0)
        .gap(24.0)
        .justify_between()
        .center_y()
        .child(
            text(label.to_string())
                .font(type_::BODY)
                .color(p.t.text_secondary),
        )
        .child(
            text(value.to_string())
                .font(type_::MONO_CAPTION)
                .color(p.t.text)
                .max_w(440.0)
                .truncate_middle(),
        )
}

fn secondary_button(
    p: Paint,
    key: &str,
    label: &str,
    enabled: bool,
    on_press: impl Fn(&mut Jarvis) + 'static,
) -> El {
    let mut button = el()
        .key(key.to_string())
        .h(30.0)
        .px(12.0)
        .radius(radius::MD)
        .center()
        .role("button")
        .label(label.to_string());

    if enabled {
        button = button
            .focusable(true)
            .border(p.hair, p.t.card_border)
            .hover(p.t.hover)
            .press(p.t.press)
            .on_press(on_press)
            .child(text(label.to_string()).font(type_::LABEL).color(p.t.text));
    } else {
        button = button.fill(p.t.control_off).child(
            text(label.to_string())
                .font(type_::LABEL)
                .color(p.t.text_secondary.alpha(0.7)),
        );
    }

    button
}

fn primary_button(
    p: Paint,
    key: &str,
    label: &str,
    enabled: bool,
    on_press: impl Fn(&mut Jarvis) + 'static,
) -> El {
    let mut button = el()
        .key(key.to_string())
        .h(32.0)
        .px(14.0)
        .radius(radius::MD)
        .center()
        .role("button")
        .label(label.to_string());

    if enabled {
        button = button
            .focusable(true)
            .fill(p.accent)
            .hover(p.accent.mix(Color::WHITE, 0.14))
            .press(p.accent.mix(Color::BLACK, 0.12))
            .on_press(on_press)
            .child(
                text(label.to_string())
                    .font(TextStyle::new(12.5, 600.0))
                    .color(Color::WHITE),
            );
    } else {
        button = button.fill(p.t.control_off).child(
            text(label.to_string())
                .font(TextStyle::new(12.5, 600.0))
                .color(p.t.text_secondary.alpha(0.7)),
        );
    }

    button
}

fn link_button(p: Paint, key: &str, label: &str, on_press: impl Fn(&mut Jarvis) + 'static) -> El {
    el().key(key.to_string())
        .h(26.0)
        .px(8.0)
        .radius(radius::SM)
        .center()
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .hover(p.t.hover)
        .on_press(on_press)
        .child(
            text(label.to_string())
                .font(type_::LABEL)
                .color(p.t.text_secondary),
        )
}

fn status_color(p: Paint, status: SetupStatus) -> Color {
    match status {
        SetupStatus::Ready => success_color(),
        SetupStatus::Missing => p.t.text_secondary,
        SetupStatus::Incomplete | SetupStatus::Warning => warning_color(),
        SetupStatus::Denied => danger_color(),
        SetupStatus::Checking => p.accent,
    }
}

fn success_color() -> Color {
    oklch(0.73, 0.15, 155.0)
}

fn warning_color() -> Color {
    oklch(0.79, 0.14, 82.0)
}

fn danger_color() -> Color {
    oklch(0.66, 0.2, 25.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jarvis_core::setup::{SetupAction, SetupReport};
    use jarvis_models::manager::ModelUpdate;
    use rune::{Env, Runtime};

    fn runtime() -> Runtime<Jarvis> {
        let mut runtime = Runtime::new(
            Jarvis::for_test(),
            view,
            (1120.0, 760.0),
            Env::new(true, 1.0),
        );
        runtime.frame(0.0);
        runtime
    }

    fn press(runtime: &mut Runtime<Jarvis>, key: &str) {
        let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("missing {key}"));
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);
    }

    #[test]
    fn the_first_frame_shows_setup_with_its_items() {
        let runtime = runtime();
        assert!(runtime.find("setup-hero").is_some());
        assert!(runtime.find("setup-row-microphone").is_some());
        assert!(runtime.find("setup-row-speech-model").is_some());
        assert!(runtime.find("setup-row-intent-model").is_some());
        assert!(runtime.find("setup-row-gpu").is_some());
        assert!(runtime.find("setup-row-ollama").is_some());
        assert!(runtime.find("download-speech-model").is_some());
        assert!(runtime.find("download-intent-model").is_some());
    }

    #[test]
    fn a_download_shows_progress_and_cancel() {
        let mut state = Jarvis::for_test();
        state.models.speech = ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Downloading,
            "742 MB of 1.5 GB",
        )
        .with_fraction(0.48);
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("model-progress-speech").is_some());
        assert!(runtime.find("cancel-speech-model").is_some());
        assert!(runtime.find("download-speech-model").is_none());
    }

    #[test]
    fn a_ready_model_offers_reloading_and_shows_its_backend() {
        let mut state = Jarvis::for_test();
        state.models.speech = ModelUpdate::new(
            ModelKind::Speech,
            ModelPhase::Ready,
            "Whisper loaded on vulkan and passed a silent warm-up.",
        )
        .with_backend("vulkan");
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("load-speech-model").is_some());
        assert!(runtime.find("model-progress-speech").is_none());
    }

    #[test]
    fn finish_setup_moves_to_the_assistant() {
        let mut runtime = runtime();
        press(&mut runtime, "finish-setup");
        assert_eq!(runtime.state.section, Section::Assistant);
        assert!(runtime.state.settings.setup_completed);
        assert!(runtime.find("assistant-page").is_some());
    }

    #[test]
    fn navigation_switches_sections() {
        let mut runtime = runtime();
        press(&mut runtime, "nav-appearance");
        assert_eq!(runtime.state.section, Section::Appearance);
        assert!(runtime.find("accent-swatches").is_some());

        press(&mut runtime, "accent-2");
        assert_eq!(runtime.state.settings.accent, 2);

        press(&mut runtime, "theme-light");
        assert_eq!(runtime.state.settings.theme, ThemeChoice::Light);
    }

    #[test]
    fn settings_tabs_render_each_page() {
        let mut runtime = runtime();
        press(&mut runtime, "settings-tab-general");
        assert!(runtime.find("storage-card").is_some());
        press(&mut runtime, "settings-tab-about");
        assert!(runtime.find("about-card").is_some());
        press(&mut runtime, "settings-tab-setup");
        assert!(runtime.find("setup-list").is_some());
    }

    #[test]
    fn denied_microphone_offers_the_settings_action() {
        let mut state = Jarvis::for_test();
        state.setup = SetupReport::new(vec![
            SetupItem {
                kind: jarvis_core::SetupKind::Microphone,
                detail: "blocked".into(),
                status: SetupStatus::Denied,
                action: SetupAction::OpenMicrophoneSettings,
            },
            SetupItem {
                kind: jarvis_core::SetupKind::SpeechModel,
                detail: "missing".into(),
                status: SetupStatus::Missing,
                action: SetupAction::None,
            },
            SetupItem {
                kind: jarvis_core::SetupKind::IntentModel,
                detail: "missing".into(),
                status: SetupStatus::Missing,
                action: SetupAction::None,
            },
            SetupItem {
                kind: jarvis_core::SetupKind::Gpu,
                detail: "none".into(),
                status: SetupStatus::Warning,
                action: SetupAction::None,
            },
            SetupItem {
                kind: jarvis_core::SetupKind::Ollama,
                detail: "missing".into(),
                status: SetupStatus::Missing,
                action: SetupAction::OpenOllamaWebsite,
            },
        ]);
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);
        assert!(runtime.find("setup-action-microphone").is_some());
    }
}
