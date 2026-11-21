use jarvis_commands::planner;
use jarvis_core::hotkey::display_chord;
use jarvis_core::insights::weekday_short;
use jarvis_core::models;
use jarvis_core::setup::{SetupItem, SetupStatus};
use jarvis_core::ThemeChoice;
use jarvis_models::manager::{ModelKind, ModelPhase};
use jarvis_speech::DictationState;
use rune::tok::{self, radius, space, type_, Theme};
use rune::{col, el, oklch, row, text, text_input, Color, Cx, El, ScrollSpec, TextStyle};

use crate::{HotkeyTarget, Jarvis, Notice, NoticeTone, Section, SettingsPage};

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
        Section::Voice => voice_page(state, p),
        Section::Routines => routines_page(state, p),
        Section::Insights => insights_page(state, p),
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
                    text(format!("v{} · phase 7", state.version))
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
    let complete = state.setup.essentials_complete();
    let (headline, detail) = if state.setup_running {
        (
            "Checking this machine",
            "Looking at microphone access, models and acceleration.".to_string(),
        )
    } else if complete {
        (
            "Ask Jarvis",
            "Run local commands. The intent model and Ollama are only consulted when needed."
                .to_string(),
        )
    } else {
        (
            "Finish setup",
            format!(
                "{} of {} essentials are ready. Local commands still work.",
                state.setup.essentials_ready, state.setup.essentials_total
            ),
        )
    };

    let mut page = col()
        .key("assistant-page")
        .w_full()
        .max_w(860.0)
        .gap(16.0)
        .child(
            row()
                .w_full()
                .gap(16.0)
                .center_y()
                .child(orb_sized(p, 88.0))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(5.0)
                        .child(
                            text(headline)
                                .font(TextStyle::new(28.0, 650.0))
                                .color(p.t.text),
                        )
                        .child(text(detail).font(type_::BODY).color(p.t.text_secondary))
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(hotkey_chip(
                                    p,
                                    "assistant-command-hotkey",
                                    &display_chord(&state.settings.command_hotkey),
                                ))
                                .child(
                                    text(if state.command.hotkey_error.is_some() {
                                        if cfg!(windows) {
                                            "Command hotkey unavailable"
                                        } else {
                                            "Hotkey available on Windows"
                                        }
                                    } else {
                                        "Global command"
                                    })
                                    .font(type_::LABEL)
                                    .color(p.t.text_secondary),
                                ),
                        ),
                ),
        )
        .child(command_card(state, p));

    match state.command.records.first() {
        Some(record) => page = page.child(command_result_card(record, p)),
        None => page = page.child(suggestions_card(p)),
    }
    if state.command.records.len() > 1 {
        page = page.child(history_card(state, p));
    }
    if !complete {
        page = page.child(setup_hint_card(p));
    }
    page
}

fn command_card(state: &Jarvis, p: Paint) -> El {
    let busy = state.command.busy;
    let has_input = !state.command.input.text.trim().is_empty();
    let listening = state.speech.mode == crate::app::SpeechMode::Command
        && state.speech.state == DictationState::Listening;
    let mut card = card(p, "assistant-command")
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(section_heading(p, "Command"))
                .child(
                    row().gap(8.0).center_y().child(
                        text("Type, or speak with the hotkey")
                            .font(type_::LABEL)
                            .color(p.t.text_secondary),
                    ),
                ),
        )
        .child(
            row()
                .key("assistant-command-field")
                .w_full()
                .h(46.0)
                .px(12.0)
                .radius(radius::MD)
                .fill(p.t.control_off)
                .border(p.hair, p.t.card_border)
                .center_y()
                .child(
                    text_input(&state.command.input, TextStyle::new(15.0, 400.0))
                        .key("assistant-command-input")
                        .single_line()
                        .placeholder(
                            "Try: open chrome and set volume to 30",
                            p.t.text_secondary.alpha(0.38),
                        )
                        .w_full()
                        .h(40.0)
                        .on_edit(|state: &mut Jarvis| &mut state.command.input)
                        .on_submit(|state: &mut Jarvis, _tx| state.submit_command()),
                ),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(primary_button(
                    p,
                    "assistant-run",
                    if busy { "Working..." } else { "Run" },
                    !busy && has_input,
                    |state| state.submit_command(),
                ))
                .child(secondary_button(
                    p,
                    "assistant-command-mic",
                    if listening {
                        "Stop command"
                    } else {
                        "Speak command"
                    },
                    !busy && (listening || !state.speech.state.is_active()),
                    |state| state.begin_command_dictation(),
                ))
                .child(
                    text(if busy {
                        state.command.status.clone()
                    } else {
                        "Rules first · local model and Ollama fallback".into()
                    })
                    .font(type_::LABEL)
                    .color(p.t.text_secondary),
                ),
        );

    if state.command.hotkey_error.is_some() && cfg!(windows) {
        card = card.child(row().w_full().justify_end().child(link_button(
            p,
            "assistant-retry-command-hotkey",
            "Retry command hotkey",
            |state| state.register_command_hotkey(),
        )));
    }
    card
}

fn command_result_card(record: &crate::CommandRecord, p: Paint) -> El {
    let color = if record.ok {
        success_color()
    } else {
        danger_color()
    };
    let mut card = card(p, "assistant-result").child(
        row()
            .w_full()
            .gap(16.0)
            .center_y()
            .justify_between()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(3.0)
                    .child(section_heading(p, "Last command"))
                    .child(
                        text(record.input.clone())
                            .font(TextStyle::new(17.0, 550.0))
                            .color(p.t.text),
                    ),
            )
            .child(
                row()
                    .key("assistant-result-source")
                    .h(22.0)
                    .px(8.0)
                    .radius(11.0)
                    .center()
                    .fill(color.alpha(0.14))
                    .child(text(record.source.clone()).font(type_::LABEL).color(color)),
            ),
    );

    if !record.actions.is_empty() {
        let mut plan = col().gap(2.0);
        for (index, action) in record.actions.iter().enumerate() {
            plan = plan.child(
                text(action.clone())
                    .key(format!("assistant-result-action-{index}"))
                    .font(type_::LABEL)
                    .color(p.t.text_secondary),
            );
        }
        card = card.child(section_heading(p, "Plan")).child(plan);
    }

    let mut messages = col().gap(6.0);
    for (index, message) in record.messages.iter().enumerate() {
        messages = messages.child(
            text(message.clone())
                .key(if index == 0 {
                    "assistant-result-message".to_string()
                } else {
                    format!("assistant-result-message-{index}")
                })
                .w_full()
                .font(if record.actions.is_empty() {
                    TextStyle::new(16.0, 450.0)
                } else {
                    type_::BODY
                })
                .color(p.t.text),
        );
    }
    card = card.child(messages);

    if !record.files.is_empty() {
        let mut files = col().gap(6.0);
        for (index, file) in record.files.iter().take(6).enumerate() {
            let open_key = format!("assistant-file-open-{index}");
            let reveal_key = format!("assistant-file-reveal-{index}");
            files = files.child(
                row()
                    .w_full()
                    .gap(10.0)
                    .center_y()
                    .justify_between()
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(1.0)
                            .child(text(file.name.clone()).font(type_::BODY).color(p.t.text))
                            .child(
                                text(file.path.clone())
                                    .font(type_::LABEL)
                                    .color(p.t.text_secondary)
                                    .truncate_middle(),
                            ),
                    )
                    .child(
                        row()
                            .gap(4.0)
                            .child(link_button(p, &open_key, "Open", move |state| {
                                state.open_command_file(index, false)
                            }))
                            .child(link_button(p, &reveal_key, "Reveal", move |state| {
                                state.open_command_file(index, true)
                            })),
                    ),
            );
        }
        card = card.child(section_heading(p, "Files")).child(files);
    }
    card
}

fn suggestions_card(p: Paint) -> El {
    card(p, "assistant-suggestions")
        .child(section_heading(p, "Try a command"))
        .child(
            row()
                .gap(8.0)
                .child(suggestion_button(p, "suggestion-open", "Open Chrome"))
                .child(suggestion_button(
                    p,
                    "suggestion-volume",
                    "Set volume to 30",
                )),
        )
        .child(
            row()
                .gap(8.0)
                .child(suggestion_button(p, "suggestion-time", "What time is it?"))
                .child(suggestion_button(
                    p,
                    "suggestion-files",
                    "Find PDFs in Downloads",
                )),
        )
        .child(
            text("Commands run on this PC. Free-form questions use Ollama when it is running.")
                .font(type_::LABEL)
                .color(p.t.text_secondary),
        )
}

fn suggestion_button(p: Paint, key: &str, label: &'static str) -> El {
    secondary_button(p, key, label, true, move |state| {
        state.run_command_text(label.to_string())
    })
}

fn history_card(state: &Jarvis, p: Paint) -> El {
    let mut rows = col().gap(8.0);
    for (index, record) in state.command.records.iter().skip(1).take(6).enumerate() {
        let color = if record.ok {
            success_color()
        } else {
            danger_color()
        };
        rows = rows.child(
            row()
                .key(format!("assistant-history-{index}"))
                .w_full()
                .gap(12.0)
                .center_y()
                .justify_between()
                .child(
                    text(record.input.clone())
                        .font(type_::BODY)
                        .color(p.t.text)
                        .max_w(520.0)
                        .truncate_middle(),
                )
                .child(text(record.source.clone()).font(type_::LABEL).color(color)),
        );
    }
    card(p, "assistant-history")
        .child(section_heading(p, "Recent"))
        .child(rows)
}

fn setup_hint_card(p: Paint) -> El {
    card(p, "assistant-setup").child(
        row()
            .w_full()
            .gap(16.0)
            .center_y()
            .justify_between()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(4.0)
                    .child(
                        text("Some setup items are still missing.")
                            .font(TextStyle::new(15.0, 600.0))
                            .color(warning_color()),
                    )
                    .child(
                        text("Models and microphone checks live in Setup.")
                            .font(type_::BODY)
                            .color(p.t.text_secondary),
                    ),
            )
            .child(secondary_button(
                p,
                "assistant-open-setup",
                "Open setup",
                true,
                |state| state.open_settings(SettingsPage::Setup),
            )),
    )
}

fn hotkey_chip(p: Paint, key: &str, label: &str) -> El {
    row()
        .key(key.to_string())
        .h(22.0)
        .px(8.0)
        .radius(11.0)
        .center()
        .fill(p.t.control_track)
        .child(
            text(label.to_string())
                .font(type_::MONO_CAPTION)
                .color(p.t.text_secondary),
        )
}

fn orb_sized(p: Paint, size: f32) -> El {
    let inner = size * 0.67;
    let core = size * 0.37;
    let dot = size * 0.09;
    el().key("orb")
        .size(size, size)
        .radius(size / 2.0)
        .fill(p.accent.alpha(0.07))
        .shadow(0.0, size * 0.2, p.accent.alpha(0.14))
        .center()
        .child(
            el().size(inner, inner)
                .radius(inner / 2.0)
                .fill(p.accent.alpha(0.13))
                .center()
                .child(
                    el().size(core, core)
                        .radius(core / 2.0)
                        .fill(p.accent)
                        .shadow(0.0, size * 0.09, p.accent.alpha(0.45))
                        .center()
                        .child(
                            el().size(dot, dot)
                                .radius(dot / 2.0)
                                .fill(Color::WHITE.alpha(0.85)),
                        ),
                ),
        )
}

fn settings_page(state: &Jarvis, p: Paint) -> El {
    let content = match state.settings_page {
        SettingsPage::Setup => setup_page(state, p),
        SettingsPage::General => general_page(state, p),
        SettingsPage::Updates => updates_page(state, p),
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

fn voice_page(state: &Jarvis, p: Paint) -> El {
    let speech = &state.speech;
    let color = match speech.state {
        DictationState::Idle => p.t.text_secondary,
        DictationState::Listening => p.accent,
        DictationState::Processing => warning_color(),
        DictationState::Ready => success_color(),
        DictationState::Error => danger_color(),
    };
    let speech_ready = matches!(
        state.models.speech.phase,
        ModelPhase::Installed | ModelPhase::Ready
    );
    let level = speech.level.clamp(0.0, 1.0);
    let level_fill = (level * 220.0).max(if level > 0.0 { 4.0 } else { 0.0 });
    let engine = if speech.model_loaded {
        speech
            .backend
            .as_ref()
            .map(|backend| format!("Whisper · {backend}"))
            .unwrap_or_else(|| "Whisper loaded".into())
    } else {
        speech.model_detail.clone()
    };
    let transcript = if !speech.partial.is_empty() {
        Some(speech.partial.clone())
    } else {
        speech.transcript.clone()
    };
    let primary = if !speech_ready {
        primary_button(p, "voice-toggle", "Install speech model", true, |state| {
            state.open_settings(SettingsPage::Setup)
        })
    } else {
        let label = match speech.state {
            DictationState::Listening => "Stop and transcribe",
            DictationState::Processing => "Cancel transcription",
            _ => "Start dictation",
        };
        primary_button(p, "voice-toggle", label, true, |state| {
            state.toggle_dictation()
        })
    };

    col()
        .key("voice-page")
        .w_full()
        .max_w(820.0)
        .gap(16.0)
        .child(
            card(p, "voice-card")
                .child(section_heading(p, "Dictation"))
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .child(el().size(10.0, 10.0).radius(5.0).fill(color).a11y_hidden())
                        .child(
                            row()
                                .h(22.0)
                                .px(8.0)
                                .radius(11.0)
                                .center()
                                .fill(color.alpha(0.14))
                                .child(
                                    text(speech.state.label())
                                        .font(type_::LABEL)
                                        .color(color),
                                ),
                        )
                        .child(
                            text(speech.detail.clone())
                                .font(type_::BODY)
                                .color(p.t.text_secondary),
                        ),
                )
                .child(
                    row()
                        .key("voice-level")
                        .gap(12.0)
                        .center_y()
                        .child(
                            row()
                                .h(8.0)
                                .w(220.0)
                                .radius(4.0)
                                .fill(p.t.control_off)
                                .child(
                                    el().key("voice-level-fill")
                                        .h(8.0)
                                        .w(level_fill)
                                        .radius(4.0)
                                        .fill(if speech.state == DictationState::Listening {
                                            p.accent
                                        } else {
                                            p.t.text_secondary
                                        })
                                        .motion(tok::motion::SNAPPY),
                                ),
                        )
                        .child(
                            text(if speech.state == DictationState::Listening {
                                "Live input"
                            } else {
                                "Input level"
                            })
                            .font(type_::LABEL)
                            .color(p.t.text_secondary),
                        ),
                )
                .child(
                    row().gap(8.0).pt(4.0).child(primary).child(secondary_button(
                        p,
                        "voice-cancel",
                        "Cancel",
                        speech.state.is_active(),
                        |state| state.cancel_dictation(),
                    ))
                    .child(secondary_button(
                        p,
                        "voice-copy",
                        "Copy transcript",
                        speech.transcript.is_some(),
                        |state| state.copy_transcript(),
                    )),
                )
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .justify_between()
                        .center_y()
                        .child(
                            text("Hotkey")
                                .font(type_::BODY)
                                .color(p.t.text_secondary),
                        )
                        .child(
                            row()
                                .key("voice-hotkey")
                                .gap(8.0)
                                .center_y()
                                .child(
                                    row()
                                        .h(24.0)
                                        .px(8.0)
                                        .radius(radius::SM)
                                        .fill(p.t.control_track)
                                        .center()
                                        .child(
                                            text(display_chord(&state.settings.dictation_hotkey))
                                                .font(type_::MONO_CAPTION)
                                                .color(p.t.text),
                                        ),
                                )
                                .child(if let Some(error) = &speech.hotkey_error {
                                    text(if cfg!(windows) {
                                        format!("Unavailable · {error}")
                                    } else {
                                        "Windows only".to_string()
                                    })
                                    .font(type_::LABEL)
                                    .color(warning_color())
                                } else {
                                    text("Global")
                                        .font(type_::LABEL)
                                        .color(success_color())
                                }),
                        ),
                )
                .child(info_row(
                    p,
                    "Microphone",
                    speech
                        .device
                        .as_deref()
                        .unwrap_or("Default input device"),
                ))
                .child(info_row(p, "Engine", &engine))
                .child(
                    if !speech_ready {
                        row()
                            .w_full()
                            .gap(10.0)
                            .center_y()
                            .justify_between()
                            .child(
                                text("The speech model is not ready yet.")
                                    .font(type_::LABEL)
                                    .color(warning_color()),
                            )
                            .child(secondary_button(
                                p,
                                "voice-open-setup",
                                "Open Setup",
                                true,
                                |state| state.open_settings(SettingsPage::Setup),
                            ))
                    } else {
                        row()
                    },
                )
                .child(if speech.hotkey_error.is_some() && cfg!(windows) {
                    row().w_full().justify_end().child(link_button(
                        p,
                        "voice-retry-hotkey",
                        "Retry hotkey",
                        |state| state.register_hotkey(),
                    ))
                } else {
                    row()
                }),
        )
        .child(
            card(p, "voice-transcript-card")
                .child(section_heading(p, "Transcript"))
                .child(
                    text(
                        transcript
                            .clone()
                            .unwrap_or_else(|| {
                                "Start dictation to see your words here.".into()
                            }),
                    )
                    .key("voice-transcript-text")
                    .w_full()
                    .font(if speech.partial.is_empty() {
                        type_::BODY
                    } else {
                        TextStyle::new(16.0, 450.0)
                    })
                    .color(if transcript.is_some() {
                        p.t.text
                    } else {
                        p.t.text_secondary
                    }),
                )
                .child(if let Some(insertion) = &speech.insertion {
                    text(insertion.clone())
                        .font(type_::LABEL)
                        .color(p.t.text_secondary)
                } else {
                    row()
                }),
        )
        .child(
            card(p, "voice-help-card")
                .child(section_heading(p, "How dictation works"))
                .child(
                    text(format!(
                        "Press {} anywhere to start, then press it again to stop. The transcript is pasted into the window you were using.",
                        display_chord(&state.settings.dictation_hotkey)
                    ))
                        .font(type_::BODY)
                        .color(p.t.text_secondary),
                )
                .child(
                    text("If Windows blocks synthetic input into an elevated app, Jarvis leaves the transcript on the clipboard instead.")
                        .font(type_::LABEL)
                        .color(p.t.text_secondary),
                )
                .child(
                    text("Audio stays on this machine. Whisper runs locally.")
                        .font(type_::LABEL)
                        .color(p.t.text_secondary),
                ),
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
        .child(startup_card(state, p))
        .child(hotkeys_card(state, p))
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

fn startup_card(state: &Jarvis, p: Paint) -> El {
    let windows = cfg!(windows);
    let tray_status = if !windows {
        (
            "System tray is available on Windows.".to_string(),
            p.t.text_secondary,
        )
    } else if let Some(error) = &state.tray_error {
        (format!("Tray unavailable · {error}"), warning_color())
    } else if state.tray.is_some() {
        ("System tray icon active.".to_string(), success_color())
    } else {
        (
            "System tray starts after the first frame.".to_string(),
            p.t.text_secondary,
        )
    };

    card(p, "startup-card")
        .child(section_heading(p, "Startup & tray"))
        .child(startup_row(
            p,
            "toggle-launch-at-login",
            "Launch at login",
            if windows {
                "Start Jarvis when you sign in."
            } else {
                "Windows only."
            },
            state.settings.launch_at_login,
            windows,
            |state| {
                let next = !state.settings.launch_at_login;
                state.set_launch_at_login(next);
            },
        ))
        .child(startup_row(
            p,
            "toggle-close-to-tray",
            "Close to tray",
            if windows {
                "Closing the window leaves Jarvis running in the tray."
            } else {
                "Windows only."
            },
            state.settings.close_to_tray,
            windows,
            |state| {
                let next = !state.settings.close_to_tray;
                state.set_close_to_tray(next);
            },
        ))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    el().size(7.0, 7.0)
                        .radius(3.5)
                        .fill(tray_status.1)
                        .a11y_hidden(),
                )
                .child(text(tray_status.0).font(type_::LABEL).color(tray_status.1)),
        )
}

fn startup_row(
    p: Paint,
    key: &str,
    title: &str,
    detail: &str,
    on: bool,
    enabled: bool,
    on_press: impl Fn(&mut Jarvis) + 'static,
) -> El {
    row()
        .w_full()
        .gap(16.0)
        .center_y()
        .justify_between()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(title).font(type_::BODY).color(p.t.text))
                .child(text(detail).font(type_::LABEL).color(p.t.text_secondary)),
        )
        .child(toggle_switch(p, key, on, enabled, on_press))
}

fn hotkeys_card(state: &Jarvis, p: Paint) -> El {
    let mut card = card(p, "hotkeys-card")
        .child(section_heading(p, "Hotkeys"))
        .child(
            text("Click Record, then press a chord. A modifier plus a letter, digit, F-key or Space works.")
                .font(type_::LABEL)
                .color(p.t.text_secondary),
        )
        .child(hotkey_setting_row(
            state,
            p,
            HotkeyTarget::Dictation,
            &state.settings.dictation_hotkey,
            &state.speech.hotkey_error,
        ))
        .child(hotkey_setting_row(
            state,
            p,
            HotkeyTarget::Command,
            &state.settings.command_hotkey,
            &state.command.hotkey_error,
        ));

    if let Some(message) = &state.hotkey_message {
        card = card.child(text(message).font(type_::LABEL).color(
            if state.hotkey_target.is_some() {
                warning_color()
            } else {
                success_color()
            },
        ));
    }
    card
}

fn hotkey_setting_row(
    state: &Jarvis,
    p: Paint,
    target: HotkeyTarget,
    chord: &str,
    error: &Option<String>,
) -> El {
    let recording = state.hotkey_target == Some(target);
    let description = match target {
        HotkeyTarget::Dictation => "Start and stop dictation.",
        HotkeyTarget::Command => "Speak a command to the Assistant.",
    };
    let mut column = col().w_full().gap(4.0).child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .justify_between()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(target.label()).font(type_::BODY).color(p.t.text))
                    .child(
                        text(description)
                            .font(type_::LABEL)
                            .color(p.t.text_secondary),
                    ),
            )
            .child(if recording {
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text("Press a chord…")
                            .font(type_::LABEL)
                            .color(warning_color()),
                    )
                    .child(link_button(p, "cancel-hotkey", "Cancel", |state| {
                        state.cancel_hotkey_capture()
                    }))
            } else {
                row()
                    .gap(8.0)
                    .center_y()
                    .child(hotkey_chip(
                        p,
                        &format!("hotkey-{}", target.setting()),
                        &display_chord(chord),
                    ))
                    .child(secondary_button(
                        p,
                        &format!("record-hotkey-{}", target.setting()),
                        "Record",
                        true,
                        move |state| state.begin_hotkey_capture(target),
                    ))
            }),
    );
    if let Some(error) = error {
        column = column.child(text(error).font(type_::LABEL).color(warning_color()));
    }
    column
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

fn updates_page(state: &Jarvis, p: Paint) -> El {
    use crate::updates::UpdatePhase;

    let updates = &state.updates;
    let busy = updates.phase.is_busy();
    let status_color = match updates.phase {
        UpdatePhase::ReadyToInstall | UpdatePhase::UpToDate => success_color(),
        UpdatePhase::Available => p.accent,
        UpdatePhase::Error => warning_color(),
        _ => p.t.text_secondary,
    };
    let status_detail = format!("{} · current v{}", updates.phase.label(), updates.current);

    let mut release_card = card(p, "updates-release")
        .child(section_heading(p, "Release"))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    el().size(7.0, 7.0)
                        .radius(3.5)
                        .fill(status_color)
                        .a11y_hidden(),
                )
                .child(text(status_detail).font(type_::LABEL).color(status_color)),
        );

    if let Some(latest) = updates.latest_version() {
        release_card = release_card.child(info_row(p, "Available", &format!("v{latest}")));
        if let Some(asset) = updates.asset() {
            let kind = asset.kind().map(|kind| kind.label()).unwrap_or("Artifact");
            release_card = release_card.child(info_row(
                p,
                kind,
                &format!(
                    "{} · {}",
                    asset.name,
                    crate::updates::human_bytes(asset.bytes)
                ),
            ));
        }
        if let Some(release) = &updates.latest {
            if !release.published_at.is_empty() {
                release_card = release_card.child(info_row(p, "Published", &release.published_at));
            }
        }
    }

    if let Some(error) = &updates.error {
        release_card = release_card.child(
            text(error.clone())
                .font(type_::LABEL)
                .color(warning_color()),
        );
    }

    if matches!(
        updates.phase,
        UpdatePhase::Downloading | UpdatePhase::Verifying
    ) {
        let fraction = if updates.phase == UpdatePhase::Verifying {
            1.0
        } else {
            updates.download.fraction()
        };
        release_card = release_card.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(progress_bar(p, "update-progress", 260.0, fraction))
                .child(
                    text(updates.download.label())
                        .font(type_::LABEL)
                        .color(p.t.text_secondary),
                ),
        );
    }

    if let Some(signature) = &updates.verified {
        let tone = if signature.trusted {
            success_color()
        } else {
            warning_color()
        };
        release_card = release_card.child(
            text(format!("Signature · {}", signature.detail))
                .font(type_::LABEL)
                .color(tone),
        );
    }

    let can_check = !busy && state.settings.check_for_updates;
    let can_download = updates.can_download() && !busy;
    let can_install = updates.can_install() && !busy;

    let mut actions = row().key("update-actions").gap(8.0).pt(4.0);
    actions = actions.child(secondary_button(
        p,
        "update-check",
        if updates.latest.is_some() {
            "Check again"
        } else {
            "Check for updates"
        },
        can_check,
        Jarvis::check_for_updates,
    ));
    if updates.phase == UpdatePhase::Available || updates.can_download() {
        actions = actions.child(primary_button(
            p,
            "update-download",
            "Download",
            can_download,
            Jarvis::download_update,
        ));
    }
    if updates.can_install() {
        actions = actions.child(primary_button(
            p,
            "update-install",
            "Install and restart",
            can_install,
            Jarvis::install_update,
        ));
        actions = actions.child(secondary_button(
            p,
            "update-reveal",
            "Show download",
            true,
            Jarvis::reveal_update,
        ));
    }
    release_card = release_card.child(actions);

    let mut channel_segments = row().key("update-channels").gap(4.0);
    for channel in jarvis_core::UpdateChannel::ALL {
        let active = state.settings.update_channel == channel;
        let mut segment = el()
            .key(format!("update-channel-{}", channel.label().to_lowercase()))
            .h(28.0)
            .px(14.0)
            .radius(radius::SM)
            .center()
            .focusable(true)
            .role("button")
            .label(channel.label())
            .on_press(move |state: &mut Jarvis| state.set_update_channel(channel));
        segment = if active {
            segment.fill(p.t.indicator)
        } else {
            segment.hover(p.t.hover)
        };
        channel_segments = channel_segments.child(segment.child(
            text(channel.label()).font(type_::LABEL).color(if active {
                p.t.text
            } else {
                p.t.text_secondary
            }),
        ));
    }

    col()
        .key("updates-page")
        .w_full()
        .max_w(820.0)
        .gap(16.0)
        .child(release_card)
        .child(
            card(p, "updates-settings")
                .child(section_heading(p, "Updates"))
                .child(startup_row(
                    p,
                    "toggle-check-for-updates",
                    "Check automatically",
                    "Once a day after start-up, from GitHub releases.",
                    state.settings.check_for_updates,
                    true,
                    |state| {
                        let next = !state.settings.check_for_updates;
                        state.set_check_for_updates(next);
                    },
                ))
                .child(startup_row(
                    p,
                    "toggle-auto-download",
                    "Download automatically",
                    "Only checksum-listed artifacts with a verified signature.",
                    state.settings.auto_download_updates,
                    state.settings.check_for_updates,
                    |state| {
                        let next = !state.settings.auto_download_updates;
                        state.set_auto_download_updates(next);
                    },
                ))
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .py(7.0)
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(2.0)
                                .child(text("Channel").font(type_::BODY).color(p.t.text))
                                .child(
                                    text(state.settings.update_channel.detail())
                                        .font(type_::LABEL)
                                        .color(p.t.text_secondary),
                                ),
                        )
                        .child(channel_segments),
                )
                .child(
                    text("Downloads are verified against SHA256SUMS.txt, then the Authenticode signature, before Install appears.")
                        .font(type_::LABEL)
                        .color(p.t.text_secondary)
                        .wrap(),
                ),
        )
        .child(updates_trust_card(state, p))
}

fn updates_trust_card(state: &Jarvis, p: Paint) -> El {
    let mut card = card(p, "updates-trust")
        .child(section_heading(p, "Signing"))
        .child(info_row(
            p,
            "Policy",
            if state.updates.signature_required {
                "Signed artifacts only"
            } else {
                "Unsigned artifacts allowed"
            },
        ))
        .child(info_row(
            p,
            "Publisher pin",
            &jarvis_core::update::publisher_pin().unwrap_or_else(|| "Not pinned".into()),
        ));

    if let Some(checked) = &state.updates.last_checked {
        card = card.child(info_row(p, "Last checked", checked));
    }

    card.child(
        text("Pin releases to one signer with JARVIS_UPDATE_PUBLISHER.")
            .font(type_::LABEL)
            .color(p.t.text_secondary),
    )
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
                .child(info_row(p, "Build", "Phase 7 · signed updates"))
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

fn routines_page(state: &Jarvis, p: Paint) -> El {
    let can_add =
        !state.routine_name.text.trim().is_empty() && !state.routine_command.text.trim().is_empty();
    let mut page = col()
        .key("routines-page")
        .w_full()
        .max_w(900.0)
        .gap(16.0)
        .child(
            text("Save a compound command once and run it whenever you need it.")
                .font(type_::BODY)
                .color(p.t.text_secondary),
        )
        .child(
            card(p, "routine-new")
                .child(section_heading(p, "New routine"))
                .child(
                    text("Name it, then give it a command. Steps run in order through the same safety checks as typed commands.")
                        .font(type_::LABEL)
                        .color(p.t.text_secondary),
                )
                .child(
                    row()
                        .w_full()
                        .gap(10.0)
                        .child(
                            row()
                                .key("routine-name-field")
                                .w(240.0)
                                .h(42.0)
                                .px(10.0)
                                .radius(radius::MD)
                                .fill(p.t.control_off)
                                .border(p.hair, p.t.card_border)
                                .center_y()
                                .child(
                                    text_input(
                                        &state.routine_name,
                                        TextStyle::new(14.0, 400.0),
                                    )
                                    .single_line()
                                    .placeholder(
                                        "Routine name",
                                        p.t.text_secondary.alpha(0.38),
                                    )
                                    .w_full()
                                    .on_edit(|state: &mut Jarvis| {
                                        &mut state.routine_name
                                    }),
                                ),
                        )
                        .child(
                            row()
                                .key("routine-command-field")
                                .grow(1.0)
                                .min_w(0.0)
                                .h(42.0)
                                .px(10.0)
                                .radius(radius::MD)
                                .fill(p.t.control_off)
                                .border(p.hair, p.t.card_border)
                                .center_y()
                                .child(
                                    text_input(
                                        &state.routine_command,
                                        TextStyle::new(14.0, 400.0),
                                    )
                                    .single_line()
                                    .placeholder(
                                        "open chrome and open vscode",
                                        p.t.text_secondary.alpha(0.38),
                                    )
                                    .w_full()
                                    .on_edit(|state: &mut Jarvis| {
                                        &mut state.routine_command
                                    })
                                    .on_submit(|state: &mut Jarvis, _tx| {
                                        state.add_routine()
                                    }),
                                ),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .gap(10.0)
                        .center_y()
                        .child(primary_button(
                            p,
                            "routine-add",
                            "Add routine",
                            can_add,
                            |state| state.add_routine(),
                        ))
                        .child(
                            text(if let Some(error) = &state.routine_error {
                                error.clone()
                            } else {
                                "Runs the whole command, including conjunctions.".into()
                            })
                            .font(type_::LABEL)
                            .color(if state.routine_error.is_some() {
                                danger_color()
                            } else {
                                p.t.text_secondary
                            }),
                        ),
                ),
        );

    if state.routines.routines.is_empty() {
        page = page.child(
            card(p, "routines-list").child(
                col()
                    .w_full()
                    .items_center()
                    .gap(6.0)
                    .py(8.0)
                    .child(
                        text("No routines yet")
                            .font(TextStyle::new(15.0, 600.0))
                            .color(p.t.text),
                    )
                    .child(
                        text("Try “Focus time” with “open chrome and open vscode”.")
                            .font(type_::BODY)
                            .color(p.t.text_secondary),
                    ),
            ),
        );
    } else {
        let mut rows = col().w_full().gap(10.0);
        for (index, routine) in state.routines.routines.iter().enumerate() {
            let steps = planner::split_by_conjunction(&routine.command);
            let summary = if steps.len() > 1 {
                steps.join("  →  ")
            } else {
                routine.command.clone()
            };
            rows = rows.child(
                row()
                    .key(format!("routine-{index}"))
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .child(
                        el().size(30.0, 30.0)
                            .radius(9.0)
                            .fill(p.accent.alpha(0.12))
                            .center()
                            .child(el().size(8.0, 8.0).radius(4.0).fill(p.accent)),
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(2.0)
                            .child(
                                text(routine.name.clone())
                                    .font(TextStyle::new(15.0, 550.0))
                                    .color(p.t.text),
                            )
                            .child(
                                text(summary)
                                    .font(type_::LABEL)
                                    .color(p.t.text_secondary)
                                    .truncate_middle(),
                            ),
                    )
                    .child(
                        row()
                            .gap(6.0)
                            .child(secondary_button(
                                p,
                                &format!("routine-run-{index}"),
                                "Run",
                                true,
                                move |state| state.run_routine(index),
                            ))
                            .child(link_button(
                                p,
                                &format!("routine-delete-{index}"),
                                "Delete",
                                move |state| state.delete_routine(index),
                            )),
                    ),
            );
        }
        page = page.child(
            card(p, "routines-list")
                .child(section_heading(p, "Saved routines"))
                .child(rows),
        );
    }

    page.child(
        text(format!(
            "Routines are stored locally in {} and run through the command pipeline.",
            state.paths.relative_to_data(&state.paths.routines_file)
        ))
        .font(type_::LABEL)
        .color(p.t.text_secondary),
    )
}

fn insights_page(state: &Jarvis, p: Paint) -> El {
    let insights = &state.insights;
    let rate = insights
        .success_rate()
        .map_or_else(|| "—".to_string(), |rate| format!("{rate}%"));
    let model = insights.intent + insights.ollama;
    let days = insights.last_days(7);
    let max_commands = days.iter().map(|day| day.commands).max().unwrap_or(0);

    let chart = if insights.total == 0 {
        col()
            .w_full()
            .items_center()
            .gap(5.0)
            .py(20.0)
            .child(
                text("No commands yet")
                    .font(TextStyle::new(15.0, 600.0))
                    .color(p.t.text),
            )
            .child(
                text(format!(
                    "Try “open notepad” or press {}.",
                    display_chord(&state.settings.command_hotkey)
                ))
                .font(type_::BODY)
                .color(p.t.text_secondary),
            )
    } else {
        let mut bars = row().key("insights-chart-bars").w_full().gap(10.0);
        for (index, day) in days.iter().enumerate() {
            let height = if day.commands == 0 {
                4.0
            } else {
                12.0 + (day.commands as f32 / max_commands.max(1) as f32) * 96.0
            };
            let active = index + 1 == days.len();
            bars = bars.child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .items_center()
                    .gap(6.0)
                    .child(
                        col().h(112.0).justify_end().child(
                            el().size(28.0, height)
                                .radius(4.0)
                                .fill(p.accent.alpha(if active { 1.0 } else { 0.6 })),
                        ),
                    )
                    .child(
                        text(weekday_short(&day.day))
                            .font(type_::LABEL)
                            .color(p.t.text_secondary),
                    )
                    .child(
                        text(day.commands.to_string())
                            .font(type_::MONO_CAPTION)
                            .color(p.t.text),
                    ),
            );
        }
        bars
    };

    let total = insights.branch_total();
    let mut branches = col().w_full().gap(8.0);
    for (label, count, color) in [
        ("Rules", insights.rules, p.accent),
        ("Intent model", insights.intent, p.t.text),
        ("Ollama", insights.ollama, p.t.text_secondary),
        ("Local", insights.local, p.t.text_secondary),
    ] {
        let width = if total == 0 {
            0.0
        } else {
            (count as f32 / total as f32) * 200.0
        };
        branches = branches.child(
            row()
                .w_full()
                .key(format!(
                    "insights-branch-{}",
                    label.to_lowercase().replace(' ', "-")
                ))
                .gap(10.0)
                .center_y()
                .child(
                    text(label)
                        .w(88.0)
                        .font(type_::BODY)
                        .color(p.t.text_secondary),
                )
                .child(
                    row()
                        .w(200.0)
                        .h(6.0)
                        .radius(3.0)
                        .fill(p.t.control_track)
                        .child(
                            el().w(width.max(if count > 0 { 2.0 } else { 0.0 }))
                                .h(6.0)
                                .radius(3.0)
                                .fill(color),
                        ),
                )
                .child(
                    text(count.to_string())
                        .font(type_::MONO_CAPTION)
                        .color(p.t.text_secondary),
                ),
        );
    }

    col()
        .key("insights-page")
        .w_full()
        .max_w(940.0)
        .gap(16.0)
        .child(
            text("A local view of commands, successes and which branch handled them.")
                .font(type_::BODY)
                .color(p.t.text_secondary),
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .child(stat_tile(
                    p,
                    "insights-stat-commands",
                    "Commands",
                    insights.total.to_string(),
                    "all time",
                    p.t.text,
                ))
                .child(stat_tile(
                    p,
                    "insights-stat-success",
                    "Success",
                    rate,
                    "completed",
                    success_color(),
                )),
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .child(stat_tile(
                    p,
                    "insights-stat-rules",
                    "Rules",
                    insights.rules.to_string(),
                    "deterministic",
                    p.accent,
                ))
                .child(stat_tile(
                    p,
                    "insights-stat-model",
                    "Model",
                    model.to_string(),
                    "intent + Ollama",
                    p.t.text,
                )),
        )
        .child(
            card(p, "insights-chart")
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .child(section_heading(p, "Commands per day"))
                        .child(
                            text("Last 7 days")
                                .font(type_::LABEL)
                                .color(p.t.text_secondary),
                        ),
                )
                .child(chart),
        )
        .child(
            card(p, "insights-branches")
                .child(section_heading(p, "Branches"))
                .child(branches),
        )
        .child(
            card(p, "insights-note")
                .child(section_heading(p, "Local by default"))
                .child(
                    text("Usage is written to insights.json on this PC. No command text, audio or model request leaves the machine.")
                        .w_full()
                        .font(type_::BODY)
                        .color(p.t.text_secondary),
                )
                .child(
                    text(format!(
                        "{} succeeded · {} failed",
                        insights.succeeded, insights.failed
                    ))
                    .font(type_::MONO_CAPTION)
                    .color(p.t.text_secondary),
                ),
        )
        .child(
            text("Insights are recorded when a command finishes; failed plans count too.")
                .font(type_::LABEL)
                .color(p.t.text_secondary),
        )
}

fn stat_tile(p: Paint, key: &str, label: &str, value: String, sub: &str, color: Color) -> El {
    col()
        .key(key)
        .grow(1.0)
        .min_w(0.0)
        .w_full()
        .padding(14.0)
        .gap(4.0)
        .fill(p.t.card)
        .radius(radius::MD)
        .border(p.hair, p.t.card_border)
        .child(
            text(label.to_uppercase())
                .font(type_::SECTION)
                .color(p.t.text_secondary),
        )
        .child(text(value).font(TextStyle::new(26.0, 650.0)).color(color))
        .child(text(sub).font(type_::LABEL).color(p.t.text_secondary))
}

fn toggle_switch(
    p: Paint,
    key: &str,
    on: bool,
    enabled: bool,
    on_press: impl Fn(&mut Jarvis) + 'static,
) -> El {
    let mut track = el()
        .key(key.to_string())
        .w(38.0)
        .h(22.0)
        .radius(11.0)
        .fill(if on { p.accent } else { p.t.control_off })
        .role("switch")
        .checked(on)
        .label(if on { "On" } else { "Off" });
    if enabled {
        track = track.focusable(true).on_press(on_press);
    }
    let mut inner = row().w_full().h_full().px(2.0).center_y();
    inner = if on {
        inner.justify_end()
    } else {
        inner.justify_start()
    };
    track.child(
        inner.child(el().size(18.0, 18.0).radius(9.0).fill(if enabled {
            Color::WHITE
        } else {
            p.t.text_secondary
        })),
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
    fn the_assistant_page_shows_the_command_bar() {
        let mut state = Jarvis::for_test();
        state.section = Section::Assistant;
        state.setup = Jarvis::demo_report(true);
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("assistant-page").is_some());
        assert!(runtime.find("assistant-command-input").is_some());
        assert!(runtime.find("assistant-run").is_some());
        assert!(runtime.find("assistant-command-mic").is_some());
        assert!(runtime.find("assistant-command-hotkey").is_some());
        assert!(runtime.find("assistant-suggestions").is_some());
    }

    #[test]
    fn a_command_result_renders_plan_messages_and_files() {
        let mut state = Jarvis::for_test();
        state.section = Section::Assistant;
        state.setup = Jarvis::demo_report(true);
        state.command.records.push(crate::CommandRecord {
            input: "find pdfs in downloads".into(),
            source: "Rules".into(),
            actions: vec!["Files: list in 'downloads' [pdf]".into()],
            messages: vec!["Found 2 matching file(s).".into()],
            files: vec![
                jarvis_platform::FileHit {
                    name: "review.pdf".into(),
                    path: r"C:\Users\Test\Downloads\review.pdf".into(),
                    size: 1200,
                    modified: "2026-11-20 09:00".into(),
                },
                jarvis_platform::FileHit {
                    name: "notes.pdf".into(),
                    path: r"C:\Users\Test\Downloads\notes.pdf".into(),
                    size: 800,
                    modified: "2026-11-19 12:00".into(),
                },
            ],
            ok: true,
        });
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("assistant-result").is_some());
        assert!(runtime.find("assistant-result-source").is_some());
        assert!(runtime.find("assistant-result-action-0").is_some());
        assert!(runtime.find("assistant-result-message").is_some());
        assert!(runtime.find("assistant-file-open-0").is_some());
        assert!(runtime.find("assistant-file-reveal-0").is_some());
    }

    #[test]
    fn routines_page_shows_saved_steps() {
        let mut state = Jarvis::for_test();
        state.section = Section::Routines;
        state.setup = Jarvis::demo_report(true);
        state
            .routines
            .add("Focus time", "open chrome and open vscode")
            .unwrap();
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("routines-page").is_some());
        assert!(runtime.find("routine-new").is_some());
        assert!(runtime.find("routine-name-field").is_some());
        assert!(runtime.find("routine-command-field").is_some());
        assert!(runtime.find("routine-add").is_some());
        assert!(runtime.find("routine-0").is_some());
        assert!(runtime.find("routine-run-0").is_some());
        assert!(runtime.find("routine-delete-0").is_some());
    }

    #[test]
    fn insights_page_shows_stats_chart_and_branches() {
        let mut state = Jarvis::for_test();
        state.section = Section::Insights;
        state.setup = Jarvis::demo_report(true);
        state.insights.record("Rules", true);
        state.insights.record("Intent model", false);
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("insights-page").is_some());
        assert!(runtime.find("insights-stat-commands").is_some());
        assert!(runtime.find("insights-stat-success").is_some());
        assert!(runtime.find("insights-chart").is_some());
        assert!(runtime.find("insights-chart-bars").is_some());
        assert!(runtime.find("insights-branch-rules").is_some());
        assert!(runtime.find("insights-branch-intent-model").is_some());
    }

    #[test]
    fn general_settings_shows_hotkeys_and_startup() {
        let mut state = Jarvis::for_test();
        state.section = Section::Settings;
        state.settings_page = SettingsPage::General;
        state.setup = Jarvis::demo_report(true);
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("hotkeys-card").is_some());
        assert!(runtime.find("hotkey-dictation").is_some());
        assert!(runtime.find("hotkey-command").is_some());
        assert!(runtime.find("record-hotkey-dictation").is_some());
        assert!(runtime.find("record-hotkey-command").is_some());
        assert!(runtime.find("startup-card").is_some());
        assert!(runtime.find("toggle-launch-at-login").is_some());
        assert!(runtime.find("toggle-close-to-tray").is_some());
    }

    #[test]
    fn the_voice_page_shows_dictation_controls() {
        let mut state = Jarvis::for_test();
        state.section = Section::Voice;
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("voice-page").is_some());
        assert!(runtime.find("voice-toggle").is_some());
        assert!(runtime.find("voice-cancel").is_some());
        assert!(runtime.find("voice-copy").is_some());
        assert!(runtime.find("voice-level").is_some());
        assert!(runtime.find("voice-hotkey").is_some());
        assert!(runtime.find("voice-transcript-card").is_some());
        assert!(runtime.find("voice-open-setup").is_some());
    }

    #[test]
    fn a_final_transcript_is_rendered_in_the_voice_page() {
        let mut state = Jarvis::for_test();
        state.section = Section::Voice;
        state.speech.state = DictationState::Ready;
        state.speech.detail = "Transcript ready.".into();
        state.speech.transcript = Some("hello from jarvis".into());
        state.speech.backend = Some("cpu".into());
        state.speech.model_loaded = true;
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("voice-transcript-text").is_some());
        assert!(runtime.find("voice-copy").is_some());
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
    fn the_updates_page_offers_check_and_reports_state() {
        let mut state = Jarvis::for_test();
        state.settings_page = SettingsPage::Updates;
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);

        assert!(runtime.find("updates-release").is_some());
        assert!(runtime.find("update-check").is_some());
        assert!(runtime.find("update-download").is_none());
        assert!(runtime.find("update-install").is_none());
        assert!(runtime.find("toggle-check-for-updates").is_some());
        assert!(runtime.find("update-channel-stable").is_some());
        assert!(runtime.find("update-channel-prerelease").is_some());
    }

    #[test]
    fn an_available_update_offers_download_then_install() {
        let mut state = Jarvis::for_test();
        state.settings_page = SettingsPage::Updates;
        state.updates.latest = Some(jarvis_core::Release {
            version: semver::Version::new(0, 2, 0),
            tag: "v0.2.0".into(),
            notes: "Notes".into(),
            notes_url: String::new(),
            published_at: "2026-01-24T00:00:00Z".into(),
            assets: vec![jarvis_core::ReleaseAsset {
                name: "Jarvis-0.2.0-x64-setup.exe".into(),
                url: "https://example.test/setup".into(),
                bytes: 24 * 1024 * 1024,
                sha256: Some("a".repeat(64)),
            }],
        });
        state.updates.phase = crate::updates::UpdatePhase::Available;
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);
        assert!(runtime.find("update-download").is_some());
        assert!(runtime.find("update-install").is_none());

        let mut state = runtime.state;
        state.updates.phase = crate::updates::UpdatePhase::ReadyToInstall;
        state.updates.installer = Some(std::path::PathBuf::from("/tmp/Jarvis-0.2.0-x64-setup.exe"));
        state.updates.verified = Some(jarvis_platform::SignatureStatus {
            trusted: true,
            publisher: Some("Jarvis contributors".into()),
            detail: "Signed by Jarvis contributors.".into(),
        });
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);
        assert!(runtime.find("update-install").is_some());
        assert!(runtime.find("update-reveal").is_some());
    }

    #[test]
    fn pressing_the_channel_segment_updates_the_setting() {
        let mut state = Jarvis::for_test();
        state.settings_page = SettingsPage::Updates;
        let mut runtime = Runtime::new(state, view, (1120.0, 760.0), Env::new(true, 1.0));
        runtime.frame(0.0);
        press(&mut runtime, "update-channel-prerelease");
        assert_eq!(
            runtime.state.settings.update_channel,
            jarvis_core::UpdateChannel::Prerelease
        );
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
