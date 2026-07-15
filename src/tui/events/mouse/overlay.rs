use super::*;

pub(super) fn text_field_hover_at(col: u16, row: u16, view: &ViewState) -> Option<HoverTarget> {
    match view.interactions.hit(col, row) {
        Some(InteractionKind::TextField(id)) => view
            .interactions
            .area_for_text_field(*id)
            .map(HoverTarget::TextField),
        _ => None,
    }
}

/// The hover target inside an overlay or editor prompt, read from the
/// interaction regions registered during render.
pub(super) fn mapped_hover_target(col: u16, row: u16, view: &ViewState) -> HoverTarget {
    match view.interactions.hit(col, row) {
        Some(InteractionKind::TextField(_)) => {
            text_field_hover_at(col, row, view).unwrap_or_default()
        }
        Some(InteractionKind::Hint(id)) => HoverTarget::FooterHint(*id),
        Some(InteractionKind::DialogRow { index, .. }) => HoverTarget::DialogRow(*index),
        Some(InteractionKind::ConfirmButton { destructive, .. }) => {
            HoverTarget::ConfirmButton(*destructive)
        }
        _ => HoverTarget::None,
    }
}

// ── Footer click ──────────────────────────────────────────────────────────────

/// The footer hint under `(col, row)`.
pub(super) fn footer_hint_at(
    app: &AppModel,
    footer: Rect,
    col: u16,
    row: u16,
) -> Option<render::HintId> {
    render::footer_hint_id_at_point(app, footer.x, footer.y, footer.width, col, row)
}

pub(super) fn footer_click_to_action(
    app: &AppModel,
    mouse: MouseEvent,
    footer: Rect,
) -> Option<Action> {
    footer_hint_at(app, footer, mouse.column, mouse.row).and_then(|id| hint_id_to_action(app, id))
}

pub(super) fn footer_area(app: &AppModel, area: Rect) -> Rect {
    if app.reader_is_fullscreen(area.width) {
        let height = render::footer_height(app, area.width).min(area.height);
        return Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(height),
            width: area.width,
            height,
        };
    }

    render::tui_layout(area, app).footer
}

// ── Dialog mouse routing ──────────────────────────────────────────────────────

pub(super) fn overlay_mouse_action(
    app: &AppModel,
    mouse: MouseEvent,
    _area: Rect,
    view: &ViewState,
    double_click: bool,
) -> Option<Action> {
    if let Some(action) = text_field_mouse_action(app, mouse, view, double_click) {
        return Some(Action::Mouse(action));
    }
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            mapped_overlay_click(app, mouse.column, mouse.row, view)
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            mapped_mood_action(mouse.column, mouse.row, view)
        }
        MouseEventKind::ScrollUp => mapped_overlay_wheel(app, mouse.column, mouse.row, -1, view),
        MouseEventKind::ScrollDown => mapped_overlay_wheel(app, mouse.column, mouse.row, 1, view),
        _ => None,
    }
}

pub(super) fn prompt_mouse_action(
    app: &AppModel,
    mouse: MouseEvent,
    view: &ViewState,
) -> Option<Action> {
    let prompt = app.editor.as_ref().map(|editor| &editor.prompt)?;
    match prompt {
        EditorPrompt::None => None,
        EditorPrompt::Help { .. } => match mouse.kind {
            MouseEventKind::ScrollDown => Some(Action::Editor(EditorAction::ScrollHelp(1))),
            MouseEventKind::ScrollUp => Some(Action::Editor(EditorAction::ScrollHelp(-1))),
            MouseEventKind::Down(MouseButton::Left) => {
                Some(Action::Editor(EditorAction::ClosePrompt))
            }
            _ => None,
        },
        _ if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
            mapped_overlay_click(app, mouse.column, mouse.row, view)
        }
        _ => None,
    }
}

fn mapped_overlay_click(app: &AppModel, col: u16, row: u16, view: &ViewState) -> Option<Action> {
    match view.interactions.hit(col, row)? {
        InteractionKind::Hint(id) => hint_id_to_action(app, *id),
        InteractionKind::DialogClose(DialogId::EditorMetadataMenu) => {
            Some(Action::Editor(EditorAction::ClosePrompt))
        }
        InteractionKind::DialogClose(_) => Some(Action::Overlay(OverlayAction::Cancel)),
        InteractionKind::DialogRow { dialog, index } => dialog_row_action(*dialog, *index),
        InteractionKind::DialogList { dialog, .. } => dialog_list_focus_action(*dialog),
        InteractionKind::DialogInput(input) => dialog_input_focus_action(*input),
        InteractionKind::ConfirmButton {
            confirm: ConfirmId::Delete,
            destructive,
        } => Some(if *destructive {
            Action::Browser(BrowserAction::ConfirmDelete)
        } else {
            Action::Overlay(OverlayAction::Cancel)
        }),
        InteractionKind::ConfirmButton {
            confirm: ConfirmId::EditorDiscard,
            destructive,
        } => Some(if *destructive {
            Action::Editor(EditorAction::Discard)
        } else {
            Action::Editor(EditorAction::ClosePrompt)
        }),
        InteractionKind::MoodBar(bar) => Some(Action::Mouse(MouseAction::SetMood(mood_score_at(
            *bar, col,
        )))),
        InteractionKind::Overlay if matches!(app.overlay, Overlay::Help { .. }) => {
            Some(Action::Overlay(OverlayAction::Cancel))
        }
        _ => None,
    }
}

fn dialog_row_action(dialog: DialogId, index: usize) -> Option<Action> {
    match dialog {
        DialogId::Settings => {
            (index == 0).then_some(Action::Settings(SettingsAction::OpenThemePicker))
        }
        DialogId::MetadataMenu | DialogId::EditorMetadataMenu => match index {
            0 => Some(Action::Metadata(MetadataAction::BeginEdit(
                MetadataKind::Tags,
            ))),
            1 => Some(Action::Metadata(MetadataAction::BeginEdit(
                MetadataKind::People,
            ))),
            2 => Some(Action::Metadata(MetadataAction::BeginEdit(
                MetadataKind::Activities,
            ))),
            3 => Some(Action::Metadata(MetadataAction::BeginFeelings)),
            4 => Some(Action::Metadata(MetadataAction::BeginMood)),
            5 => Some(Action::Location(LocationAction::BeginEdit)),
            _ => None,
        },
        DialogId::ThemePicker => Some(Action::Settings(SettingsAction::ThemePickerSelect(index))),
        DialogId::Metadata => Some(Action::Mouse(MouseAction::DialogRow {
            target: DialogListTarget::Metadata,
            index,
        })),
        DialogId::Feelings => Some(Action::Mouse(MouseAction::DialogRow {
            target: DialogListTarget::Feelings,
            index,
        })),
        DialogId::Location => Some(Action::Mouse(MouseAction::DialogRow {
            target: DialogListTarget::Location,
            index,
        })),
    }
}

fn dialog_list_focus_action(dialog: DialogId) -> Option<Action> {
    match dialog {
        DialogId::Metadata | DialogId::Feelings => Some(Action::Mouse(
            MouseAction::DialogFocusMetadata(EditMetadataFocusTarget::List),
        )),
        DialogId::Location => Some(Action::Mouse(MouseAction::DialogFocusLocation(
            EditLocationFocus::List,
        ))),
        _ => None,
    }
}

fn dialog_input_focus_action(input: DialogInputId) -> Option<Action> {
    match input {
        DialogInputId::Metadata | DialogInputId::Feelings => Some(Action::Mouse(
            MouseAction::DialogFocusMetadata(EditMetadataFocusTarget::Input),
        )),
        DialogInputId::LocationQuery => Some(Action::Mouse(MouseAction::DialogFocusLocation(
            EditLocationFocus::Query,
        ))),
        DialogInputId::LocationName => Some(Action::Mouse(MouseAction::DialogFocusLocation(
            EditLocationFocus::Name,
        ))),
    }
}

fn mapped_mood_action(col: u16, row: u16, view: &ViewState) -> Option<Action> {
    let InteractionKind::MoodBar(bar) = view.interactions.hit(col, row)? else {
        return None;
    };
    Some(Action::Mouse(MouseAction::SetMood(mood_score_at(
        *bar, col,
    ))))
}

fn mapped_overlay_wheel(
    app: &AppModel,
    col: u16,
    row: u16,
    delta: i16,
    view: &ViewState,
) -> Option<Action> {
    if matches!(app.overlay, Overlay::Help { .. }) {
        return Some(Action::Overlay(OverlayAction::HelpScroll(delta)));
    }
    let (dialog, viewport) = match view.interactions.hit(col, row)? {
        InteractionKind::DialogList { dialog, viewport } => (*dialog, *viewport),
        InteractionKind::DialogRow { dialog, .. } => {
            (*dialog, view.interactions.dialog_list_viewport(*dialog)?)
        }
        _ => return None,
    };
    let target = match dialog {
        DialogId::Metadata => DialogListTarget::Metadata,
        DialogId::Feelings => DialogListTarget::Feelings,
        DialogId::Location => DialogListTarget::Location,
        DialogId::ThemePicker => DialogListTarget::ThemePicker,
        _ => return None,
    };
    Some(Action::Mouse(MouseAction::DialogScroll {
        target,
        delta,
        viewport,
    }))
}

/// Mouse editing for the single-line text fields (the search box and dialog
/// inputs): a press in a field focuses it, places the caret, and arms a
/// selection; a drag extends it; release finishes it. Returns whether the
/// event was consumed by a field, mirroring the editor's selection flow.
pub(super) fn text_field_mouse_action(
    app: &AppModel,
    mouse: MouseEvent,
    view: &ViewState,
    double_click: bool,
) -> Option<MouseAction> {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let (target, column) = text_field_at(mouse.column, mouse.row, view)?;
            Some(if double_click {
                MouseAction::TextFieldSelectWord { target, column }
            } else {
                MouseAction::TextFieldPress { target, column }
            })
        }
        MouseEventKind::Drag(MouseButton::Left) if app.nav.input_selecting => {
            let target = active_text_field_target(app)?;
            let rect = view.interactions.area_for_text_field(target.into())?;
            Some(MouseAction::TextFieldDrag {
                column: mouse
                    .column
                    .clamp(rect.x, rect.x + rect.width.saturating_sub(1))
                    .saturating_sub(rect.x),
            })
        }
        MouseEventKind::Up(MouseButton::Left) if app.nav.input_selecting => {
            Some(MouseAction::TextFieldRelease)
        }
        _ => None,
    }
}

/// The text field under `(col, row)`, if any: focuses it and returns the click
/// column within the field.
fn text_field_at(col: u16, row: u16, view: &ViewState) -> Option<(TextFieldTarget, u16)> {
    let InteractionKind::TextField(id) = view.interactions.hit(col, row)? else {
        return None;
    };
    let area = view.interactions.area_for_text_field(*id)?;
    Some(((*id).into(), col - area.x))
}

fn active_text_field_target(app: &AppModel) -> Option<TextFieldTarget> {
    match &app.overlay {
        Overlay::NewJournal(_) => Some(TextFieldTarget::NewJournal),
        Overlay::EditMetadata(_) => Some(TextFieldTarget::Metadata),
        Overlay::EditFeelings(_) => Some(TextFieldTarget::Feelings),
        Overlay::EditLocation(state) => match state.focus {
            EditLocationFocus::Query => Some(TextFieldTarget::LocationQuery),
            EditLocationFocus::Name => Some(TextFieldTarget::LocationName),
            EditLocationFocus::List => None,
        },
        Overlay::None if app.nav.mode == Mode::Search => Some(TextFieldTarget::Search),
        _ => None,
    }
}

fn mood_score_at(bar: Rect, column: u16) -> i8 {
    if bar.width <= 1 {
        return 0;
    }

    let relative = column.saturating_sub(bar.x).min(bar.width - 1);
    let scaled = (relative as f32 / (bar.width - 1) as f32 * 10.0).round() as i8;
    (scaled - 5).clamp(-5, 5)
}
