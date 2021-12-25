pub mod table_column;
pub use self::table_column::{TextColumn, TextColumnConstraint};

mod table_scroll_state;
use self::table_scroll_state::ScrollState as TextTableState;

pub mod data_row;
pub use data_row::DataRow;

pub mod data_cell;
pub use data_cell::DataCell;

use std::{borrow::Cow, cmp::min, panic::Location};

use tui::{
    backend::Backend,
    layout::{Constraint, Rect},
    style::Style,
    widgets::{Row, Table},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    constants::TABLE_GAP_HEIGHT_LIMIT,
    tuine::{DrawContext, Event, Key, StateContext, Status, TmpComponent, ViewContext},
};

#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    text: Style,
    selected_text: Style,
    table_header: Style,
}

enum SortStatus {
    Unsortable,
    Sortable { column: usize },
}

/// A sortable, scrollable table for text data.
pub struct TextTable<Message> {
    key: Key,
    column_widths: Vec<u16>,
    columns: Vec<TextColumn>,
    show_gap: bool,
    show_selected_entry: bool,
    rows: Vec<DataRow>,
    style_sheet: StyleSheet,
    sortable: SortStatus,
    table_gap: u16,
    on_select: Option<Box<dyn Fn(usize) -> Message>>,
    on_selected_click: Option<Box<dyn Fn(usize) -> Message>>,
}

impl<Message> TextTable<Message> {
    #[track_caller]
    pub fn new<S: Into<Cow<'static, str>>>(ctx: &mut ViewContext<'_>, columns: Vec<S>) -> Self {
        Self {
            key: ctx.register_component(Location::caller()),
            column_widths: vec![0; columns.len()],
            columns: columns
                .into_iter()
                .map(|name| TextColumn::new(name))
                .collect(),
            show_gap: true,
            show_selected_entry: true,
            rows: Vec::default(),
            style_sheet: StyleSheet::default(),
            sortable: SortStatus::Unsortable,
            table_gap: 0,
            on_select: None,
            on_selected_click: None,
        }
    }

    /// Sets the row to display in the table.
    ///
    /// Defaults to displaying no data if not set.
    pub fn rows(mut self, rows: Vec<DataRow>) -> Self {
        self.rows = rows;
        self.try_sort_data();

        self
    }

    /// Whether to try to show a gap between the table headers and data.
    /// Note that if there isn't enough room, the gap will still be hidden.
    ///
    /// Defaults to `true` if not set.
    pub fn show_gap(mut self, show_gap: bool) -> Self {
        self.show_gap = show_gap;
        self
    }

    /// Whether to highlight the selected entry.
    ///
    /// Defaults to `true` if not set.
    pub fn show_selected_entry(mut self, show_selected_entry: bool) -> Self {
        self.show_selected_entry = show_selected_entry;
        self
    }

    /// Whether the table should display as sortable.
    ///
    /// Defaults to unsortable if not set.
    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = if sortable {
            SortStatus::Sortable { column: 0 }
        } else {
            SortStatus::Unsortable
        };

        self.try_sort_data();

        self
    }

    /// Calling this enables sorting, and sets the sort column to `column`.
    pub fn sort_column(mut self, column: usize) -> Self {
        self.sortable = SortStatus::Sortable { column };

        self.try_sort_data();

        self
    }

    /// Returns whether the table is currently sortable.
    pub fn is_sortable(&self) -> bool {
        matches!(self.sortable, SortStatus::Sortable { .. })
    }

    /// What to do when selecting an entry. Expects a boxed function that takes in
    /// the currently selected index and returns a [`Message`].
    ///
    /// Defaults to `None` if not set.
    pub fn on_select(mut self, on_select: Option<Box<dyn Fn(usize) -> Message>>) -> Self {
        self.on_select = on_select;
        self
    }

    /// What to do when clicking on an entry that is already selected.
    ///
    /// Defaults to `None` if not set.
    pub fn on_selected_click(
        mut self, on_selected_click: Option<Box<dyn Fn(usize) -> Message>>,
    ) -> Self {
        self.on_selected_click = on_selected_click;
        self
    }

    fn try_sort_data(&mut self) {
        use std::cmp::Ordering;

        if let SortStatus::Sortable { column } = self.sortable {
            // TODO: We can avoid some annoying checks vy using const generics - this is waiting on
            // the const_generics_defaults feature, landing in 1.59, however!

            self.rows
                .sort_by(|a, b| match (a.get(column), b.get(column)) {
                    (Some(a), Some(b)) => a.cmp(b),
                    (Some(_a), None) => Ordering::Greater,
                    (None, Some(_b)) => Ordering::Less,
                    (None, None) => Ordering::Equal,
                });
        }
    }

    fn update_column_widths(&mut self, bounds: Rect) {
        let total_width = bounds.width;
        let mut width_remaining = bounds.width;

        let mut column_widths: Vec<u16> = self
            .columns
            .iter()
            .map(|column| {
                let width = match column.width_constraint {
                    TextColumnConstraint::Fill => {
                        let desired = column.name.graphemes(true).count().saturating_add(1) as u16;
                        min(desired, width_remaining)
                    }
                    TextColumnConstraint::Length(length) => min(length, width_remaining),
                    TextColumnConstraint::Percentage(percentage) => {
                        let length = total_width * percentage / 100;
                        min(length, width_remaining)
                    }
                    TextColumnConstraint::MaxLength(length) => {
                        let desired = column.name.graphemes(true).count().saturating_add(1) as u16;
                        min(min(length, desired), width_remaining)
                    }
                    TextColumnConstraint::MaxPercentage(percentage) => {
                        let desired = column.name.graphemes(true).count().saturating_add(1) as u16;
                        let length = total_width * percentage / 100;
                        min(min(desired, length), width_remaining)
                    }
                };
                width_remaining -= width;
                width
            })
            .collect();

        if !column_widths.is_empty() {
            let amount_per_slot = width_remaining / column_widths.len() as u16;
            width_remaining %= column_widths.len() as u16;
            for (index, width) in column_widths.iter_mut().enumerate() {
                if (index as u16) < width_remaining {
                    *width += amount_per_slot + 1;
                } else {
                    *width += amount_per_slot;
                }
            }
        }

        self.column_widths = column_widths;
    }
}

impl<Message> TmpComponent<Message> for TextTable<Message> {
    fn draw<B>(
        &mut self, state_ctx: &mut StateContext<'_>, draw_ctx: &DrawContext<'_>,
        frame: &mut Frame<'_, B>,
    ) where
        B: Backend,
    {
        let rect = draw_ctx.rect();
        let state = state_ctx.mut_state::<TextTableState>(self.key);
        state.set_num_items(self.rows.len()); // FIXME: Not a fan of this system like this - should be easier to do.

        self.table_gap = if !self.show_gap
            || (self.rows.len() + 2 > rect.height.into() && rect.height < TABLE_GAP_HEIGHT_LIMIT)
        {
            0
        } else {
            1
        };

        let table_extras = 1 + self.table_gap;
        let scrollable_height = rect.height.saturating_sub(table_extras);
        self.update_column_widths(rect);

        // Calculate widths first, since we need them later.
        let widths = self
            .column_widths
            .iter()
            .map(|column| Constraint::Length(*column))
            .collect::<Vec<_>>();

        // Then calculate rows. We truncate the amount of data read based on height,
        // as well as truncating some entries based on available width.
        let data_slice = {
            // Note: `get_list_start` already ensures `start` is within the bounds of the number of items, so no need to check!
            let start = state.display_start_index(rect, scrollable_height as usize);
            let end = min(state.num_items(), start + scrollable_height as usize);

            debug!("Start: {}, end: {}", start, end);
            self.rows.drain(start..end).into_iter().map(|row| {
                let r: Row<'_> = row.into();
                r
            })
        };

        // Now build up our headers...
        let header = Row::new(self.columns.iter().map(|column| column.name.clone()))
            .style(self.style_sheet.table_header)
            .bottom_margin(self.table_gap);

        let mut table = Table::new(data_slice)
            .header(header)
            .style(self.style_sheet.text);

        if self.show_selected_entry {
            table = table.highlight_style(self.style_sheet.selected_text);
        }

        frame.render_stateful_widget(table.widths(&widths), rect, state.tui_state());
    }

    fn on_event(
        &mut self, state_ctx: &mut StateContext<'_>, draw_ctx: &DrawContext<'_>, event: Event,
        messages: &mut Vec<Message>,
    ) -> Status {
        use crate::tuine::MouseBoundIntersect;
        use crossterm::event::{MouseButton, MouseEventKind};

        let rect = draw_ctx.rect();
        let state = state_ctx.mut_state::<TextTableState>(self.key);
        state.set_num_items(self.rows.len());

        match event {
            Event::Keyboard(key_event) => {
                if key_event.modifiers.is_empty() {
                    match key_event.code {
                        _ => Status::Ignored,
                    }
                } else {
                    Status::Ignored
                }
            }
            Event::Mouse(mouse_event) => {
                if mouse_event.does_mouse_intersect_bounds(rect) {
                    match mouse_event.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let y = mouse_event.row - rect.top();

                            if y == 0 {
                                if let SortStatus::Sortable { column } = self.sortable {
                                    todo!() // Sort by the clicked column!
                                            // self.sort_data();
                                } else {
                                    Status::Ignored
                                }
                            } else if y > self.table_gap {
                                let visual_index = usize::from(y - self.table_gap);
                                state.set_visual_index(visual_index)
                            } else {
                                Status::Ignored
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            let status = state.move_down(1);
                            if let Some(on_select) = &self.on_select {
                                messages.push(on_select(state.current_index()));
                            }
                            status
                        }
                        MouseEventKind::ScrollUp => {
                            let status = state.move_up(1);
                            if let Some(on_select) = &self.on_select {
                                messages.push(on_select(state.current_index()));
                            }
                            status
                        }
                        _ => Status::Ignored,
                    }
                } else {
                    Status::Ignored
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {}
