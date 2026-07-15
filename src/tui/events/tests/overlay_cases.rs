use super::*;
use crate::tui::state::MetadataKind;

// ── Settings menu / theme picker routing ─────────────────────────────────────

#[test]
fn comma_opens_settings_in_browse_but_not_over_dialogs() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char(',')), true),
        Some(Action::Settings(SettingsAction::OpenMenu))
    );

    // With a dialog open the key belongs to that overlay, not settings.
    app.begin_edit_tags();
    assert_ne!(
        keyboard::key_to_action(&app, key(KeyCode::Char(',')), true),
        Some(Action::Settings(SettingsAction::OpenMenu))
    );
}

// ── Help cheatsheet ──────────────────────────────────────────────────────────

#[test]
fn question_mark_opens_help_from_browse_and_search_panes() {
    let mut app = app_with_entries(1);
    app.nav.focus = Focus::Entries;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('?')), true),
        Some(Action::Overlay(OverlayAction::OpenHelp))
    );

    // In search, `?` opens the cheatsheet from a result view but types into the
    // search field.
    app.begin_search();
    app.nav.focus = Focus::Reader;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('?')), true),
        Some(Action::Overlay(OverlayAction::OpenHelp))
    );
    app.nav.focus = Focus::Entries;
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('?')), true),
        Some(Action::Overlay(OverlayAction::InputKey(key(
            KeyCode::Char('?')
        ))))
    );
}

#[test]
fn help_overlay_scrolls_on_arrows_and_closes_on_any_other_key() {
    let mut app = app_with_entries(1);
    app.open_help();

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Down), true),
        Some(Action::Overlay(OverlayAction::HelpScroll(1)))
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::PageUp), true),
        Some(Action::Overlay(OverlayAction::HelpScroll(-10)))
    );
    // A quit key does not quit while the reference is up — it dismisses it.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('q')), true),
        Some(Action::Overlay(OverlayAction::Cancel))
    );
}

#[test]
fn help_hint_click_opens_the_cheatsheet() {
    let app = app_with_entries(1);
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::Help),
        Some(Action::Overlay(OverlayAction::OpenHelp))
    );
}

#[test]
fn wheel_over_help_scrolls_it_without_closing() {
    let mut app = app_with_entries(1);
    app.open_help();

    mouse_in_area(&mut app, mouse(MouseEventKind::ScrollDown, 5, 5), 80, 20);

    // The wheel bumps the reference's scroll and the overlay stays open — the
    // early-return keeps the event off the panes behind it.
    match app.overlay {
        crate::tui::state::Overlay::Help { scroll } => assert_eq!(scroll, 1),
        _ => panic!("help overlay closed on wheel"),
    }
}

#[test]
fn settings_menu_routes_enter_and_t_to_the_theme_picker() {
    let mut app = app_with_journals(&["work"]);
    app.open_settings_menu();

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::Settings(SettingsAction::OpenThemePicker))
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('t')), true),
        Some(Action::Settings(SettingsAction::OpenThemePicker))
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Esc), true),
        Some(Action::Overlay(OverlayAction::Cancel))
    );
}

#[test]
fn theme_picker_keys_route_to_dedicated_actions() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Up), true),
        Some(Action::Metadata(MetadataAction::MoveSelection(-1)))
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Down), true),
        Some(Action::Metadata(MetadataAction::MoveSelection(1)))
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::Settings(SettingsAction::ThemePickerConfirm))
    );
    // Esc reverts through the dedicated cancel, not the generic overlay close.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Esc), true),
        Some(Action::Settings(SettingsAction::ThemePickerCancel))
    );

    // The picker's hint chips route to the same actions.
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::ThemePickerApply),
        Some(Action::Settings(SettingsAction::ThemePickerConfirm))
    );
    assert_eq!(
        mouse::hint_id_to_action(&app, render::HintId::ThemePickerRevert),
        Some(Action::Settings(SettingsAction::ThemePickerCancel))
    );
}

// ── Hover ─────────────────────────────────────────────────────────────────────

#[test]
fn hover_tracks_journal_rows_without_moving_selection() {
    let mut app = app_with_journals(&["work", "zeta"]);
    let area = Rect::new(0, 0, 120, 20);
    let journals = render::tui_layout(area, &app)
        .journals
        .expect("journals panel");
    let list = render::journal_list_rect(journals.content);
    let selected_before = app.nav.journal_list.selected();

    // The middle line of the second journal's row.
    let row = list.y + render::journal_row_height(&app.appearance.theme) + 1;
    assert!(apply_hover(&mut app, list.x + 2, row, area));
    assert_eq!(app.hover, HoverTarget::Journal(1));
    assert_eq!(
        app.nav.journal_list.selected(),
        selected_before,
        "hover must never move the journal selection"
    );

    // Motion within the same row doesn't ask for a repaint.
    assert!(!apply_hover(&mut app, list.x + 3, row, area));

    // The run loop dispatches SetHover(None) ahead of every key event — pin
    // the handler half of that "any key clears the glow" contract.
    let backend = ratatui::backend::TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    dispatch_action(&mut terminal, &mut app, Action::SetHover(HoverTarget::None)).unwrap();
    assert_eq!(app.hover, HoverTarget::None);
}

#[test]
fn hover_translation_does_not_mutate_the_model() {
    let mut app = app_with_journals(&["work", "zeta"]);
    let area = Rect::new(0, 0, 120, 20);
    let (_, view) = render_view(&mut app, area.width, area.height);
    let journals = render::tui_layout(area, &app)
        .journals
        .expect("journals panel");
    let list = render::journal_list_rect(journals.content);
    let row = list.y + render::journal_row_height(&app.appearance.theme) + 1;
    let selected_before = app.nav.journal_list.selected();
    let hover_before = app.hover;

    assert_eq!(
        mouse::hover_action_at(&app, list.x + 2, row, area, &view),
        Action::SetHover(HoverTarget::Journal(1))
    );
    assert_eq!(app.nav.journal_list.selected(), selected_before);
    assert_eq!(app.hover, hover_before);
}

#[test]
fn click_translation_does_not_mutate_the_model() {
    let mut app = app_with_journals(&["work", "zeta"]);
    let area = Rect::new(0, 0, 120, 20);
    let backend = ratatui::backend::TestBackend::new(area.width, area.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let mut view = crate::tui::ui::ViewState::default();
    let theme = app.appearance.theme.clone();
    terminal
        .draw(|frame| {
            let mut context = crate::tui::ui::RenderContext::new(&theme, &mut view);
            crate::tui::render::draw(frame, &mut app, &mut context);
        })
        .unwrap();
    let journals = view.layout.unwrap().journals.unwrap();
    let list = render::journal_list_rect(journals.content);
    let row = list.y + render::journal_row_height(&app.appearance.theme) + 1;
    let before = (
        app.nav.focus,
        app.nav.journal_list.selected(),
        app.nav.entry_list.selected(),
        app.nav.scroll.reader,
        app.toasts.items().len(),
    );

    let action = mouse::mouse_to_action(&app, mouse(down(), list.x + 2, row), area, &view, false);

    assert_eq!(
        action,
        Some(Action::Mouse(action::MouseAction::JournalClick {
            index: Some(1),
            compact: false,
        }))
    );
    assert_eq!(
        before,
        (
            app.nav.focus,
            app.nav.journal_list.selected(),
            app.nav.entry_list.selected(),
            app.nav.scroll.reader,
            app.toasts.items().len(),
        )
    );
}

#[test]
fn hover_finds_footer_hints() {
    let mut app = app_with_journals(&["work"]);
    let area = Rect::new(0, 0, 120, 20);
    let footer = render::tui_layout(area, &app).footer;
    let hovered = (footer.x..footer.x + footer.width).any(|col| {
        apply_hover(&mut app, col, footer.y, area);
        matches!(app.hover, HoverTarget::FooterHint(_))
    });
    assert!(hovered, "no footer hint hoverable on the browse footer");
}

#[test]
fn hover_tracks_insights_tabs_without_switching_tabs() {
    let mut app = app_with_entries(1);
    app.nav.selected_entry_index = None;
    app.nav.insights_tab = InsightsTab::Overview;
    let area = Rect::new(0, 0, 140, 20);
    let insights = render::tui_layout(area, &app)
        .insights
        .expect("insights panel");
    let col = (insights.area.x..insights.area.x + insights.area.width)
        .find(|col| {
            render::insights_tab_at(&app.appearance.theme, insights.area, *col, insights.area.y)
                == Some(InsightsTab::Writing)
        })
        .expect("writing tab");

    assert!(apply_hover(&mut app, col, insights.area.y, area));
    assert_eq!(app.hover, HoverTarget::InsightsTab(InsightsTab::Writing));
    assert_eq!(app.nav.insights_tab, InsightsTab::Overview);
}

#[test]
fn theme_picker_hover_targets_rows_without_selecting() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    let area = Rect::new(0, 0, 90, 30);
    let state = app.theme_picker_state().expect("picker open");
    let len = state.entries.len();
    assert!(len > 1, "picker should list the bundled themes");
    let initial = state.selected_index();
    let offset = state.offset();
    let target = if initial == Some(offset) {
        offset + 1
    } else {
        offset
    };
    let layout = render::theme_picker_layout(&app.appearance.theme, area, len, state.hint_state());

    let row = layout.list.y + (target - offset) as u16;
    assert!(apply_hover(&mut app, layout.list.x + 1, row, area));
    // Like every other dialog, hover only highlights the row — it neither
    // moves the selection nor previews the theme (that's click's job).
    assert_eq!(app.hover, HoverTarget::DialogRow(target));
    assert_eq!(app.theme_picker_state().unwrap().selected_index(), initial);
}

#[test]
fn settings_menu_hover_targets_its_rows() {
    use crate::tui::ui::{DialogId, InteractionKind};

    let mut app = app_with_journals(&["work"]);
    app.open_settings_menu();
    let area = Rect::new(0, 0, 64, 20);

    // Find the theme row through the regions render registered — the same
    // ones the click path resolves against.
    let (_, view) = render_view(&mut app, area.width, area.height);
    let point = find_interaction(&view, area.width, area.height, |kind| {
        matches!(
            kind,
            InteractionKind::DialogRow {
                dialog: DialogId::Settings,
                index: 0,
            }
        )
    })
    .expect("settings menu has a hoverable row");
    assert!(apply_hover(&mut app, point.0, point.1, area));
    assert_eq!(app.hover, HoverTarget::DialogRow(0));
}

#[test]
fn editor_discard_prompt_hover_targets_the_buttons() {
    let mut app = app_with_entries(1);
    app.select_entry_index(0);
    app.open_editor_for_selected().unwrap();
    app.editor.as_mut().unwrap().prompt = crate::tui::editor_state::EditorPrompt::ConfirmDiscard {
        discard_selected: false,
    };
    let area = Rect::new(0, 0, 120, 20);

    // Probe every cell until both buttons are found through the real regions.
    let mut saw = (false, false);
    for row in 0..area.height {
        for col in 0..area.width {
            apply_hover(&mut app, col, row, area);
            match app.hover {
                HoverTarget::ConfirmButton(true) => saw.0 = true,
                HoverTarget::ConfirmButton(false) => saw.1 = true,
                _ => {}
            }
        }
    }
    assert!(saw.0 && saw.1, "both discard buttons hoverable: {saw:?}");
}

// ── Menu clicks through the interaction map ───────────────────────────────────

/// The action a click at the given dialog row / close region translates to,
/// resolved through the regions render registered.
fn menu_click_action(
    app: &mut AppModel,
    area: Rect,
    predicate: impl Fn(&crate::tui::ui::InteractionKind) -> bool,
) -> Option<Action> {
    let (_, view) = render_view(app, area.width, area.height);
    let (col, row) =
        find_interaction(&view, area.width, area.height, predicate).expect("region registered");
    mouse::mouse_to_action(app, mouse(down(), col, row), area, &view, false)
}

#[test]
fn settings_menu_click_maps_rows_and_close_through_the_regions() {
    use crate::tui::ui::{DialogId, InteractionKind};

    let mut app = app_with_journals(&["work"]);
    app.open_settings_menu();
    let area = Rect::new(0, 0, 64, 20);

    let row_action = menu_click_action(&mut app, area, |kind| {
        matches!(
            kind,
            InteractionKind::DialogRow {
                dialog: DialogId::Settings,
                index: 0,
            }
        )
    });
    assert_eq!(
        row_action,
        Some(Action::Settings(SettingsAction::OpenThemePicker))
    );

    let close_action = menu_click_action(&mut app, area, |kind| {
        matches!(kind, InteractionKind::DialogClose(DialogId::Settings))
    });
    assert_eq!(close_action, Some(Action::Overlay(OverlayAction::Cancel)));

    // Dispatching the row click actually lands in the theme picker.
    let (_, view) = render_view(&mut app, area.width, area.height);
    let (col, row) = find_interaction(&view, area.width, area.height, |kind| {
        matches!(
            kind,
            InteractionKind::DialogRow {
                dialog: DialogId::Settings,
                index: 0,
            }
        )
    })
    .unwrap();
    mouse_in_area(&mut app, mouse(down(), col, row), area.width, area.height);
    assert!(app.theme_picker_state().is_some());
}

#[test]
fn metadata_menu_click_maps_every_row_to_its_action() {
    use crate::tui::ui::{DialogId, InteractionKind};

    let mut app = app_with_entries(1);
    app.select_entry_index(0);
    app.open_metadata_menu();
    assert!(matches!(app.overlay, Overlay::MetadataMenu));
    let area = Rect::new(0, 0, 80, 24);

    let expected: [Action; 6] = [
        Action::Metadata(MetadataAction::BeginEdit(MetadataKind::Tags)),
        Action::Metadata(MetadataAction::BeginEdit(MetadataKind::People)),
        Action::Metadata(MetadataAction::BeginEdit(MetadataKind::Activities)),
        Action::Metadata(MetadataAction::BeginFeelings),
        Action::Metadata(MetadataAction::BeginMood),
        Action::Location(LocationAction::BeginEdit),
    ];
    for (index, expected) in expected.into_iter().enumerate() {
        let action = menu_click_action(&mut app, area, |kind| {
            matches!(
                kind,
                InteractionKind::DialogRow {
                    dialog: DialogId::MetadataMenu,
                    index: i,
                } if *i == index
            )
        });
        assert_eq!(action, Some(expected), "row {index}");
    }

    let close_action = menu_click_action(&mut app, area, |kind| {
        matches!(kind, InteractionKind::DialogClose(DialogId::MetadataMenu))
    });
    assert_eq!(close_action, Some(Action::Overlay(OverlayAction::Cancel)));
}

#[test]
fn editor_double_click_maps_to_select_word() {
    let mut app = app_with_entries(1);
    app.select_entry_index(0);
    app.open_editor_for_selected().unwrap();
    let area = Rect::new(0, 0, 80, 24);
    let (_, view) = render_view(&mut app, area.width, area.height);

    // A central cell in the text body, away from the border and footer.
    let (col, row) = (10, 6);
    let single = mouse::mouse_to_action(&app, mouse(down(), col, row), area, &view, false);
    assert_eq!(
        single,
        Some(Action::Editor(EditorAction::StartSelection { col, row }))
    );
    let double = mouse::mouse_to_action(&app, mouse(down(), col, row), area, &view, true);
    assert_eq!(
        double,
        Some(Action::Editor(EditorAction::SelectWord { col, row }))
    );
}

#[test]
fn editor_metadata_menu_click_maps_rows_and_close() {
    use crate::tui::ui::{DialogId, InteractionKind};

    let mut app = app_with_entries(1);
    app.select_entry_index(0);
    app.open_editor_for_selected().unwrap();
    app.editor.as_mut().unwrap().prompt = crate::tui::editor_state::EditorPrompt::MetadataMenu;
    let area = Rect::new(0, 0, 80, 24);

    let row_action = menu_click_action(&mut app, area, |kind| {
        matches!(
            kind,
            InteractionKind::DialogRow {
                dialog: DialogId::EditorMetadataMenu,
                index: 0,
            }
        )
    });
    assert_eq!(
        row_action,
        Some(Action::Metadata(MetadataAction::BeginEdit(
            MetadataKind::Tags
        )))
    );

    // The editor's menu closes back to the editor, not the overlay layer.
    let close_action = menu_click_action(&mut app, area, |kind| {
        matches!(
            kind,
            InteractionKind::DialogClose(DialogId::EditorMetadataMenu)
        )
    });
    assert_eq!(
        close_action,
        Some(Action::Editor(EditorAction::ClosePrompt))
    );
}

// ── Toast interaction ─────────────────────────────────────────────────────────

#[test]
fn clicking_a_toast_dismisses_it() {
    let mut app = app_with_journals(&["work"]);
    app.toast(crate::tui::state::ToastVariant::Info, "First");
    app.toast(crate::tui::state::ToastVariant::Error, "Second");
    let area = Rect::new(0, 0, 120, 30);
    let rects = render::toast_rects(&app, area);
    assert_eq!(rects.len(), 2);

    // Click the second toast: only it disappears.
    let target = rects[1];
    mouse_in_area(&mut app, mouse(down(), target.x + 1, target.y + 1), 120, 30);
    let remaining: Vec<_> = app
        .toasts
        .items()
        .iter()
        .map(|toast| toast.message.clone())
        .collect();
    assert_eq!(remaining, vec!["First".to_string()]);

    // A click outside any toast is not swallowed by the dismiss probe.
    mouse_in_area(&mut app, mouse(down(), 0, area.height - 1), 120, 30);
    assert_eq!(app.toasts.items().len(), 1);
}

#[test]
fn hovering_a_toast_targets_it_over_everything() {
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    app.toast(crate::tui::state::ToastVariant::Info, "Saved");
    let area = Rect::new(0, 0, 120, 30);
    let rect = render::toast_rects(&app, area)[0];

    assert!(apply_hover(&mut app, rect.x + 1, rect.y + 1, area));
    // Even with the picker open, the topmost toast wins the probe.
    assert_eq!(app.hover, HoverTarget::Toast(0));
}

#[test]
fn dialog_list_hover_targets_rows_without_selecting() {
    let mut app = app_with_entries(1);
    app.begin_edit_tags();
    set_tag_dialog_items(&mut app, 5);
    let area = Rect::new(0, 0, 120, 20);
    let layout = render::metadata_dialog_layout(&app.appearance.theme, area, 5);

    // The third row: hover targets it, but selection and toggles stay put.
    assert!(apply_hover(
        &mut app,
        layout.list.x,
        layout.list.y + 2,
        area
    ));
    assert_eq!(app.hover, HoverTarget::DialogRow(2));
    let state = app.edit_metadata_state().unwrap();
    assert_eq!(state.selected_index(), Some(0));
    assert!(state.selected.is_empty());
}

#[test]
fn confirm_delete_hover_targets_the_buttons() {
    let mut app = app_with_entries(1);
    let ctx = crate::tui::state::DeleteContext::Entry { has_body: true };
    app.overlay = crate::tui::state::Overlay::ConfirmDelete(
        crate::tui::state::DeleteContext::Entry { has_body: true },
        false,
    );
    let area = Rect::new(0, 0, 120, 20);
    let inner = render::confirm_delete_inner(&app.appearance.theme, area, &ctx);

    // Probe every cell of the buttons row until each button is found.
    let mut saw = (false, false);
    for col in inner.x..inner.x + inner.width {
        for row in inner.y..inner.y + inner.height {
            apply_hover(&mut app, col, row, area);
            match app.hover {
                HoverTarget::ConfirmButton(true) => saw.0 = true,
                HoverTarget::ConfirmButton(false) => saw.1 = true,
                _ => {}
            }
        }
    }
    assert!(saw.0 && saw.1, "both confirm buttons hoverable: {saw:?}");
}

#[test]
fn confirm_delete_enter_commits_the_selected_button() {
    let mut app = app_with_entries(1);
    app.begin_confirm_delete();

    // Safe default: Cancel is selected, so a bare Enter cancels rather than deletes.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::Overlay(OverlayAction::Cancel))
    );
    // The y/n shortcuts still fire directly, whatever the selection.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('y')), true),
        Some(Action::Browser(BrowserAction::ConfirmDelete))
    );
    // Left picks the destructive button, Right the safe one.
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Left), true),
        Some(Action::Overlay(OverlayAction::ConfirmSelect(true)))
    );

    // With Delete selected, Enter commits the delete.
    app.overlay = crate::tui::state::Overlay::ConfirmDelete(
        crate::tui::state::DeleteContext::Entry { has_body: true },
        true,
    );
    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Enter), true),
        Some(Action::Browser(BrowserAction::ConfirmDelete))
    );
}

#[test]
fn theme_picker_cycles_chrome_and_cancel_restores_it() {
    use crate::tui::theme::ChromeStyle;
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    assert_eq!(app.appearance.chrome_override, None);

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('b')), true),
        Some(Action::Settings(SettingsAction::ThemePickerCycleChrome))
    );

    // auto → flat → bordered → auto, previewing live.
    app.theme_picker_cycle_chrome();
    assert_eq!(app.appearance.chrome_override, Some(ChromeStyle::Flat));
    app.theme_picker_cycle_chrome();
    assert_eq!(app.appearance.chrome_override, Some(ChromeStyle::Bordered));

    // Cancel restores the override from open time along with the theme.
    app.theme_picker_cancel();
    assert_eq!(app.appearance.chrome_override, None);
}

#[test]
fn theme_picker_confirm_persists_the_chrome_override() {
    use crate::tui::theme::ChromeStyle;
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    app.theme_picker_cycle_chrome();
    app.theme_picker_confirm();
    assert_eq!(
        app.services.config.ui.chrome,
        crate::config::ChromeMode::Flat
    );
    assert_eq!(app.appearance.chrome_override, Some(ChromeStyle::Flat));
    // The saved config round-trips the setting.
    let loaded = crate::config::load_config(&app.services.config_path).unwrap();
    assert_eq!(loaded.ui.chrome, crate::config::ChromeMode::Flat);
}

#[test]
fn theme_picker_cycles_color_mode_and_cancel_restores_it() {
    use crate::config::ColorMode;
    use crate::tui::theme::Mode;
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    assert_eq!(app.appearance.color_mode, ColorMode::Auto);

    assert_eq!(
        keyboard::key_to_action(&app, key(KeyCode::Char('m')), true),
        Some(Action::Settings(SettingsAction::ThemePickerCycleMode))
    );

    // auto → dark → light → auto, previewing live; the resolved mode follows
    // (auto falls back to dark with no detected terminal background).
    app.theme_picker_cycle_mode();
    assert_eq!(app.appearance.color_mode, ColorMode::Dark);
    app.theme_picker_cycle_mode();
    assert_eq!(app.appearance.color_mode, ColorMode::Light);
    assert_eq!(app.appearance.mode(), Mode::Light);

    // A mode change re-resolves the picker rows against the new variant.
    let journal_light = app
        .theme_picker_state()
        .and_then(|state| state.entries.iter().find(|entry| entry.name == "journal"))
        .and_then(|entry| entry.theme.clone())
        .expect("bundled journal theme resolves");
    assert_eq!(
        journal_light.base_bg(),
        ratatui::style::Color::Rgb(0xfc, 0xfc, 0xfc),
        "journal rows must re-resolve to the light variant"
    );

    // Cancel restores the mode from open time along with the theme.
    app.theme_picker_cancel();
    assert_eq!(app.appearance.color_mode, ColorMode::Auto);
}

#[test]
fn theme_picker_confirm_persists_the_color_mode() {
    use crate::config::ColorMode;
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();
    app.theme_picker_cycle_mode();
    app.theme_picker_confirm();
    assert_eq!(app.services.config.ui.color_mode, ColorMode::Dark);
    // The saved config round-trips the setting.
    let loaded = crate::config::load_config(&app.services.config_path).unwrap();
    assert_eq!(loaded.ui.color_mode, ColorMode::Dark);
}

#[test]
fn theme_picker_hides_the_mode_switch_on_mode_agnostic_themes() {
    use crate::config::ColorMode;
    let mut app = app_with_journals(&["work"]);
    app.open_theme_picker();

    // classic resolves identically in both modes, so the switch would be a
    // no-op there; blossom has real variants.
    let position = |app: &crate::tui::app::AppModel, name: &str| {
        app.theme_picker_state()
            .unwrap()
            .entries
            .iter()
            .position(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("bundled theme '{name}' listed"))
    };
    let classic = position(&app, "classic");
    app.theme_picker_select(classic);
    let state = app.theme_picker_state().unwrap();
    assert!(!state.mode_switchable());
    assert!(
        render::theme_picker_hints(
            state.hint_state(),
            app.appearance.chrome_override,
            app.appearance.color_mode,
        )
        .iter()
        .all(|hint| hint.id != render::HintId::ThemePickerMode),
        "mode hint should be hidden on classic"
    );
    // The key is a no-op while the hint is hidden.
    app.theme_picker_cycle_mode();
    assert_eq!(app.appearance.color_mode, ColorMode::Auto);

    // A variant theme shows the switch again.
    let blossom = position(&app, "blossom");
    app.theme_picker_select(blossom);
    assert!(app.theme_picker_state().unwrap().mode_switchable());

    app.theme_picker_cancel();
}
