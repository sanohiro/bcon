//! Tab and TabManager: manage multiple tabs, each containing a pane tree

use std::collections::HashMap;

use super::layout;
use super::split_tree::SplitTree;
use super::{Direction, NavDirection, Pane, PaneId, PaneRect};
use crate::terminal::Terminal;

/// A single tab containing a split tree and its panes
pub struct Tab {
    pub tree: SplitTree,
    pub panes: HashMap<PaneId, Pane>,
    pub active_pane: PaneId,
    pub zoomed_pane: Option<PaneId>,
    pub title: String,
}

impl Tab {
    /// Create a new tab with a single pane
    pub fn new(pane_id: PaneId, terminal: Terminal, rect: PaneRect) -> Self {
        let tree = SplitTree::new(pane_id);
        let mut panes = HashMap::new();
        panes.insert(pane_id, Pane::new(pane_id, terminal, rect));
        Self {
            tree,
            panes,
            active_pane: pane_id,
            zoomed_pane: None,
            title: String::new(),
        }
    }

    /// Get a reference to the active pane
    pub fn active_pane(&self) -> &Pane {
        self.panes.get(&self.active_pane).expect("active pane must exist")
    }

    /// Get a mutable reference to the active pane
    pub fn active_pane_mut(&mut self) -> &mut Pane {
        self.panes
            .get_mut(&self.active_pane)
            .expect("active pane must exist")
    }

    /// Recalculate layout for all panes in this tab
    pub fn relayout(&mut self, available: PaneRect) {
        let rects = layout::calculate_layout(&self.tree, available);
        for (pid, rect) in &rects {
            if let Some(pane) = self.panes.get_mut(pid) {
                pane.rect = *rect;
            }
        }
    }

    /// Get layout rects (for navigation)
    pub fn layout_rects(&self) -> HashMap<PaneId, PaneRect> {
        self.panes.iter().map(|(id, p)| (*id, p.rect)).collect()
    }

    /// Split the active pane
    pub fn split(
        &mut self,
        new_pane_id: PaneId,
        terminal: Terminal,
        direction: Direction,
        available: PaneRect,
    ) {
        self.tree
            .split(self.active_pane, new_pane_id, direction, 0.5);

        // Add new pane with a temporary rect (will be set by relayout)
        let pane = Pane::new(new_pane_id, terminal, PaneRect::new(0.0, 0.0, 0.0, 0.0));
        self.panes.insert(new_pane_id, pane);

        // Focus the new pane
        self.active_pane = new_pane_id;

        // Recalculate all rects
        self.relayout(available);
    }

    /// Close the active pane. Returns None if it was the last pane.
    pub fn close_active_pane(&mut self, available: PaneRect) -> Option<PaneId> {
        let closing = self.active_pane;
        let new_focus = self.tree.remove(closing)?;

        self.panes.remove(&closing);
        self.active_pane = new_focus;

        // Clear zoom if the zoomed pane was closed
        if self.zoomed_pane == Some(closing) {
            self.zoomed_pane = None;
        }

        self.relayout(available);
        Some(new_focus)
    }

    /// Navigate to an adjacent pane
    pub fn navigate(&mut self, direction: NavDirection) -> bool {
        let rects = self.layout_rects();
        if let Some(target) = self.tree.navigate(self.active_pane, direction, &rects) {
            self.active_pane = target;
            true
        } else {
            false
        }
    }

    /// Resize the active pane's split ratio
    pub fn resize_active(&mut self, delta: f32, available: PaneRect) -> bool {
        if self.tree.resize_ratio(self.active_pane, delta) {
            self.relayout(available);
            true
        } else {
            false
        }
    }

    /// Toggle zoom for the active pane
    pub fn toggle_zoom(&mut self) {
        if self.zoomed_pane.is_some() {
            self.zoomed_pane = None;
        } else if self.panes.len() > 1 {
            self.zoomed_pane = Some(self.active_pane);
        }
    }

    /// Find which pane contains the given pixel coordinate
    #[allow(dead_code)]
    pub fn pane_at(&self, px: f32, py: f32) -> Option<PaneId> {
        for (id, pane) in &self.panes {
            if pane.rect.contains(px, py) {
                return Some(*id);
            }
        }
        None
    }

    /// Process PTY output for all panes. Returns true if any pane produced output.
    pub fn process_all_pty(&mut self) -> bool {
        let mut any_output = false;
        let pane_ids: Vec<PaneId> = self.panes.keys().copied().collect();
        for pid in pane_ids {
            if let Some(pane) = self.panes.get_mut(&pid) {
                loop {
                    match pane.terminal.process_pty_output() {
                        Ok(0) => break,
                        Ok(_) => any_output = true,
                        Err(e) => {
                            log::warn!("PTY read error for pane {:?}: {}", pid, e);
                            break;
                        }
                    }
                }
            }
        }
        any_output
    }

    /// Check if any pane's terminal has died
    #[allow(dead_code)]
    pub fn dead_panes(&self) -> Vec<PaneId> {
        self.panes
            .iter()
            .filter(|(_, p)| !p.terminal.is_alive())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Number of panes
    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }
}

/// Manages multiple tabs
pub struct TabManager {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    next_pane_id: u16,
}

impl TabManager {
    /// Create a TabManager with a single tab containing one pane
    pub fn new(terminal: Terminal, rect: PaneRect) -> Self {
        let pane_id = PaneId(0);
        let tab = Tab::new(pane_id, terminal, rect);
        Self {
            tabs: vec![tab],
            active_tab: 0,
            next_pane_id: 1,
        }
    }

    /// Allocate a new unique PaneId
    pub fn next_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        id
    }

    /// Get the active tab
    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    /// Get the active tab mutably
    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
    }

    /// Get a mutable reference to the active pane's terminal
    pub fn active_terminal_mut(&mut self) -> &mut Terminal {
        &mut self.active_tab_mut().active_pane_mut().terminal
    }

    /// Get a reference to the active pane's terminal
    pub fn active_terminal(&self) -> &Terminal {
        &self.active_tab().active_pane().terminal
    }

    /// Split the active pane in the active tab
    pub fn split(
        &mut self,
        direction: Direction,
        terminal: Terminal,
        available: PaneRect,
    ) -> PaneId {
        let new_id = self.next_pane_id();
        self.active_tab_mut().split(new_id, terminal, direction, available);
        new_id
    }

    /// Close the active pane. Returns false if it was the last pane in the last tab.
    pub fn close_active_pane(&mut self, available: PaneRect) -> bool {
        let tab = &mut self.tabs[self.active_tab];
        if tab.pane_count() > 1 {
            tab.close_active_pane(available);
            true
        } else if self.tabs.len() > 1 {
            // Close the whole tab
            self.tabs.remove(self.active_tab);
            if self.active_tab >= self.tabs.len() {
                self.active_tab = self.tabs.len() - 1;
            }
            true
        } else {
            false // Last pane in last tab
        }
    }

    /// Add a new tab with a single pane
    pub fn new_tab(&mut self, terminal: Terminal, rect: PaneRect) -> PaneId {
        let pane_id = self.next_pane_id();
        let tab = Tab::new(pane_id, terminal, rect);
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        pane_id
    }

    /// Close the active tab. Returns false if it's the last tab.
    pub fn close_active_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        true
    }

    /// Switch to next tab
    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
        }
    }

    /// Switch to previous tab
    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = if self.active_tab == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab - 1
            };
        }
    }

    /// Process PTY output for ALL tabs (prevents buffer overflow in background tabs)
    pub fn process_all_pty(&mut self) -> bool {
        let mut any_output = false;
        for tab in &mut self.tabs {
            if tab.process_all_pty() {
                any_output = true;
            }
        }
        any_output
    }

    /// Number of tabs
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Relayout all panes in the active tab
    #[allow(dead_code)]
    pub fn relayout_active(&mut self, available: PaneRect) {
        self.active_tab_mut().relayout(available);
    }

    /// Relayout all tabs (for screen resize)
    pub fn relayout_all(&mut self, available: PaneRect) {
        for tab in &mut self.tabs {
            tab.relayout(available);
        }
    }

    /// Get the active pane ID
    pub fn active_pane_id(&self) -> PaneId {
        self.active_tab().active_pane
    }

    /// Check if zoom is active
    pub fn is_zoomed(&self) -> bool {
        self.active_tab().zoomed_pane.is_some()
    }

    /// Resize all terminals in the active tab to match their pane rects
    pub fn resize_terminals_to_rects(&mut self, cell_w: f32, cell_h: f32) {
        Self::resize_tab_terminals(&mut self.tabs[self.active_tab], cell_w, cell_h);
    }

    /// Resize all terminals in ALL tabs to match their pane rects
    pub fn resize_all_terminals_to_rects(&mut self, cell_w: f32, cell_h: f32) {
        for tab in &mut self.tabs {
            Self::resize_tab_terminals(tab, cell_w, cell_h);
        }
    }

    fn resize_tab_terminals(tab: &mut Tab, cell_w: f32, cell_h: f32) {
        let pane_ids: Vec<PaneId> = tab.panes.keys().copied().collect();
        for pid in pane_ids {
            if let Some(pane) = tab.panes.get_mut(&pid) {
                let cols = (pane.rect.width / cell_w).floor() as usize;
                let rows = (pane.rect.height / cell_h).floor() as usize;
                let cols = cols.max(1);
                let rows = rows.max(1);
                if cols != pane.terminal.grid.cols() || rows != pane.terminal.grid.rows() {
                    pane.terminal.resize(cols, rows);
                }
            }
        }
    }
}
