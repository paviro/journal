use super::*;

/// Adjust a list's pixel scroll offset so the row for `selected` is in view, given
/// the rows' `meta`. Shared by the entry list and journal column.
fn ensure_pixel_row_visible(
    list: &mut ratatui::widgets::ListState,
    meta: &[RowMeta],
    selected: Option<usize>,
    viewport_height: u16,
) {
    let mut scroll = list.offset();
    crate::tui::entry_rows::ensure_row_visible(&mut scroll, meta, selected, viewport_height);
    *list.offset_mut() = scroll;
}

impl App {
    pub(crate) fn selected_journal_index(&self) -> usize {
        self.nav.journal_list.selected().unwrap_or(0)
    }

    pub(crate) fn selected_journal(&self) -> Option<&Journal> {
        self.library.journals.get(self.selected_journal_index())
    }

    /// Number of active (non-archived) journals. Because `library.journals` is
    /// ordered active-first, this is also the index of the first archived journal
    /// — the split point where the "Archived" divider sits.
    pub(crate) fn active_journal_count(&self) -> usize {
        self.library
            .journals
            .iter()
            .filter(|journal| !journal.archived)
            .count()
    }

    pub(crate) fn has_archived_journals(&self) -> bool {
        self.active_journal_count() < self.library.journals.len()
    }

    /// Pixel offset that puts journal `index`'s box at the top of the list. The
    /// journal column's rows are a uniform [`JOURNAL_BOX_HEIGHT`] tall — boxes and
    /// the "Archived" divider alike — so this is a plain multiply without building
    /// the rows.
    pub(super) fn journal_row_top(&self, index: usize) -> usize {
        let divider_before =
            usize::from(self.has_archived_journals() && index >= self.active_journal_count());
        (index + divider_before) * crate::tui::render::JOURNAL_BOX_HEIGHT as usize
    }

    /// Whether an entry is the active preview: one is selected *and* focus sits on
    /// a column that owns it (the entry list or the entry viewer). Browsing the
    /// journal list or the insights panel — even with a selection lingering from an
    /// earlier preview — is not previewing an entry; both those columns show insights
    /// in the shared right-hand pane instead. Both the entry-list highlight and the
    /// preview-vs-insights choice key off this single predicate so they can never
    /// disagree.
    fn entry_is_previewed(&self) -> bool {
        matches!(self.nav.focus, Focus::Entries | Focus::EntryView)
            && self.nav.selected_entry_index.is_some()
    }

    /// The preview pane shows journal insights (instead of an entry) when browsing and
    /// no entry is being previewed.
    pub(crate) fn show_journal_insights_preview(&self) -> bool {
        self.nav.mode == Mode::Browse && !self.entry_is_previewed()
    }

    /// Whether the insights panel is the focused pane — the context in which its
    /// tab (Left/Right) and scope (`g`) keys apply, and where its tabs render in
    /// the inverted focused style.
    pub(crate) fn insights_panel_focused(&self) -> bool {
        self.nav.focus == Focus::Insights
    }

    /// Whether the entries list should draw a highlighted selection row.
    pub(crate) fn entries_highlighted(&self) -> bool {
        self.entry_is_previewed()
    }

    /// Clamp only the journal list's *selection* index into `[0, len)`. The offset
    /// is tracked in pixels and clamped separately at render (via `clamp_scroll`),
    /// so — unlike [`normalize_list_state`](crate::tui::state::normalize_list_state)
    /// — this must not touch it.
    pub(crate) fn normalize_journal_selection(&mut self) {
        let len = self.library.journals.len();
        let state = &mut self.nav.journal_list;
        if len == 0 {
            state.select(None);
        } else {
            state.select(Some(state.selected().unwrap_or(0).min(len - 1)));
        }
    }

    /// The journal column's rows and their per-row scroll metadata, paired with the
    /// content-relative list rect they lay out in. Shared by the render, click,
    /// wheel, and scrollbar paths so they all agree with what `draw_journals` drew.
    pub(crate) fn journal_rows(
        &self,
        content: ratatui::layout::Rect,
    ) -> (
        Vec<crate::tui::entry_rows::BoxRow>,
        Vec<RowMeta>,
        ratatui::layout::Rect,
    ) {
        let list_area = crate::tui::render::journal_list_rect(content);
        let inner_width = list_area.width.saturating_sub(4) as usize;
        let rows = crate::tui::entry_rows::journal_list_rows(self, inner_width);
        let meta = crate::tui::entry_rows::rows_meta(&rows);
        (rows, meta, list_area)
    }

    /// Scroll the journal list so the selected journal's box is in view. The
    /// journal column uses the same pixel-row model as the entry list, so `meta`
    /// carries per-row heights (including the "Archived" divider row).
    pub(crate) fn journal_list_ensure_visible(&mut self, meta: &[RowMeta], viewport_height: u16) {
        let selected = self.nav.journal_list.selected();
        ensure_pixel_row_visible(&mut self.nav.journal_list, meta, selected, viewport_height);
    }

    pub(crate) fn scroll_journal_list(
        &mut self,
        delta: i16,
        total_height: usize,
        viewport_height: u16,
    ) {
        *self.nav.journal_list.offset_mut() = crate::tui::render::scroll_pixels(
            self.nav.journal_list.offset(),
            delta,
            total_height,
            viewport_height,
        );
    }

    pub(super) fn reset_entry_scroll(&mut self) {
        *self.nav.entry_list.offset_mut() = 0;
        self.nav.scroll.reset_entry_view();
    }

    pub(crate) fn scroll_entry_list(
        &mut self,
        delta: i16,
        total_height: usize,
        viewport_height: u16,
    ) {
        *self.nav.entry_list.offset_mut() = crate::tui::render::scroll_pixels(
            self.nav.entry_list.offset(),
            delta,
            total_height,
            viewport_height,
        );
    }

    pub(crate) fn entry_list_ensure_visible(&mut self, rows: &[RowMeta], viewport_height: u16) {
        let selected = self.nav.selected_entry_index;
        ensure_pixel_row_visible(&mut self.nav.entry_list, rows, selected, viewport_height);
    }

    /// Contiguous index range into `entries` for the selected journal, or `None`
    /// when no journal is selected or it has no entries.
    fn selected_entry_range(&self) -> Option<Range<usize>> {
        let journal = self.selected_journal()?;
        self.library.range(&journal.name)
    }

    pub(crate) fn selected_entries(&self) -> Vec<&Entry> {
        match self.selected_entry_range() {
            Some(range) => self.library.entries[range].iter().collect(),
            None => Vec::new(),
        }
    }

    pub(crate) fn current_entry_list_len(&self) -> usize {
        match self.nav.mode {
            Mode::Search => self.search.hits.len(),
            Mode::Browse => self.selected_entry_range().map_or(0, |range| range.len()),
        }
    }

    /// The entry backing the current selection, resolving a search hit through
    /// the id index. Unifies the Search/Browse branches the preview getters share.
    pub(super) fn resolved_selected_entry(&self) -> Option<&Entry> {
        match self.nav.mode {
            Mode::Search => self.library.entry_by_id(&self.selected_search_hit()?.id),
            Mode::Browse => self.selected_entry(),
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let len = match self.nav.focus {
            // The insights panel's tabs are non-scrolling top-N views; Up/Down do
            // nothing there (tabs switch with Left/Right).
            Focus::Insights => return,
            Focus::Journals if self.nav.mode == Mode::Browse => self.library.journals.len(),
            Focus::Entries | Focus::EntryView | Focus::Journals => self.current_entry_list_len(),
        };
        if len == 0 {
            return;
        }

        let previous_entry_index = self.nav.selected_entry_index;
        if self.nav.focus == Focus::Journals && self.nav.mode == Mode::Browse {
            move_list_selection(&mut self.nav.journal_list, len, delta);
            // Browsing journals previews the journal's insights, not an entry: leave the
            // entry selection empty until the user moves into the entries column.
            self.nav.selected_entry_index = None;
            *self.nav.entry_list.offset_mut() = 0;
            // A different journal means different insights, so its lists start fresh.
            self.nav.scroll.reset_insights();
        } else {
            match self.nav.selected_entry_index {
                // Deselected (Browse shows journal insights): a downward move selects
                // the first entry; an upward move stays on the insights view.
                None if self.nav.mode == Mode::Browse => {
                    if delta > 0 {
                        self.nav.selected_entry_index = Some(0);
                    }
                }
                // Scrolling up past the first entry deselects, revealing journal insights.
                Some(0) if self.nav.mode == Mode::Browse && delta < 0 => {
                    self.nav.selected_entry_index = None;
                }
                current => {
                    let base = current.unwrap_or(0) as isize;
                    let next = (base + delta).clamp(0, len as isize - 1) as usize;
                    self.nav.selected_entry_index = Some(next);
                }
            }
        }
        if self.nav.selected_entry_index != previous_entry_index {
            self.nav.scroll.entry_view = 0;
        }
    }

    pub(crate) fn select_journal(&mut self, index: usize) {
        if index >= self.library.journals.len() {
            return;
        }

        if self.selected_journal_index() != index {
            self.nav.journal_list.select(Some(index));
            // Selecting a journal previews its insights, not an entry: clear any entry
            // selection so the insights column shows and no preview is rendered.
            self.nav.selected_entry_index = None;
            self.reset_entry_scroll();
        }
    }

    pub(crate) fn select_entry_index(&mut self, index: usize) {
        if index >= self.current_entry_list_len() {
            return;
        }

        if self.nav.selected_entry_index != Some(index) {
            self.nav.selected_entry_index = Some(index);
            self.nav.scroll.entry_view = 0;
        }
    }

    pub(crate) fn select_entry_by_id(&mut self, id: &str, reset_entry_scroll: bool) -> bool {
        let index = match self.nav.mode {
            Mode::Search => self.search.hits.iter().position(|hit| hit.id == id),
            Mode::Browse => self.journal_name_for_entry_id(id).and_then(|journal_name| {
                self.library
                    .entries
                    .iter()
                    .filter(|entry| entry.journal == journal_name)
                    .position(|entry| entry.id == id)
            }),
        };
        let Some(index) = index else { return false };

        if self.nav.selected_entry_index != Some(index) {
            self.nav.selected_entry_index = Some(index);
        }
        if reset_entry_scroll {
            self.nav.scroll.entry_view = 0;
        }
        true
    }

    fn journal_name_for_entry_id(&mut self, id: &str) -> Option<String> {
        let journal_name = self
            .library
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.journal.clone())?;
        let journal_index = self
            .library
            .journals
            .iter()
            .position(|journal| journal.name == journal_name)?;
        if self.selected_journal_index() != journal_index {
            self.nav.journal_list.select(Some(journal_index));
            *self.nav.entry_list.offset_mut() = 0;
        }
        Some(journal_name)
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        let index = self.nav.selected_entry_index?;
        let range = self.selected_entry_range()?;
        (index < range.len()).then(|| &self.library.entries[range.start + index])
    }

    pub(crate) fn selected_search_hit(&self) -> Option<&SearchHit> {
        self.search.hits.get(self.nav.selected_entry_index?)
    }

    pub(crate) fn selected_entry_target(&self) -> Option<EntryTarget> {
        // In Search mode the title comes from the hit (journal-prefixed label),
        // otherwise from the entry itself; the rest is shared.
        let title = match self.nav.mode {
            Mode::Search => self.search_hit_label(self.selected_search_hit()?),
            Mode::Browse => self.selected_entry()?.display_label(),
        };
        let entry = self.resolved_selected_entry()?;
        Some(EntryTarget {
            id: entry.id.clone(),
            path: entry.path.clone(),
            title,
            locked: entry.encryption_state == EntryEncryptionState::EncryptedLocked,
        })
    }

    pub(crate) fn selected_entry_tags(&self) -> Vec<String> {
        self.selected_entry_metadata(MetadataKind::Tags)
    }

    pub(crate) fn selected_entry_people(&self) -> Vec<String> {
        self.selected_entry_metadata(MetadataKind::People)
    }

    pub(crate) fn selected_entry_activities(&self) -> Vec<String> {
        self.selected_entry_metadata(MetadataKind::Activities)
    }

    pub(super) fn selected_entry_metadata(&self, kind: MetadataKind) -> Vec<String> {
        self.resolved_selected_entry()
            .map(|entry| metadata_values(entry, kind).to_vec())
            .unwrap_or_default()
    }

    pub(crate) fn selected_entry_feelings(&self) -> Vec<String> {
        self.resolved_selected_entry()
            .map(|entry| entry.metadata.feelings.clone())
            .unwrap_or_default()
    }

    pub(crate) fn has_selected_entry_target(&self) -> bool {
        self.selected_entry_target().is_some()
    }

    pub(crate) fn can_act_on_selected_entry(&self) -> bool {
        matches!(self.nav.focus, Focus::Entries | Focus::EntryView)
            && self.has_selected_entry_target()
    }

    /// Whether the entry viewer currently occupies the whole screen: either the
    /// terminal is single-column (no room for other panes) or the viewer has been
    /// expanded to full screen in a multi-column layout.
    pub(crate) fn entry_view_is_fullscreen(&self, width: u16) -> bool {
        self.nav.focus == Focus::EntryView
            && (single_panel_is_active(width) || self.nav.entry_view_fullscreen)
    }

    /// Whether the insights panel currently occupies the whole screen: either the
    /// terminal is single-column (no room for other panes) or the panel has been
    /// expanded to full screen in a multi-column layout.
    pub(crate) fn insights_is_fullscreen(&self, width: u16) -> bool {
        self.nav.focus == Focus::Insights
            && (single_panel_is_active(width) || self.nav.insights_fullscreen)
    }

    pub(crate) fn selected_entry_view(&self) -> Option<(String, String)> {
        let entry = self.resolved_selected_entry()?;
        if entry.encryption_state == EntryEncryptionState::EncryptedLocked {
            return Some((
                entry_timestamp_label(entry),
                "Encryption identity not available".to_string(),
            ));
        }
        Some((entry_timestamp_label(entry), entry.content.clone()))
    }

    pub(crate) fn select_journal_by_name(&mut self, name: &str) {
        if let Some(index) = self
            .library
            .journals
            .iter()
            .position(|journal| journal.name == name)
        {
            self.nav.journal_list.select(Some(index));
            *self.nav.journal_list.offset_mut() = self.journal_row_top(index);
            self.nav.selected_entry_index = Some(0);
            self.reset_entry_scroll();
            self.nav.focus = Focus::Entries;
        }
    }
}
