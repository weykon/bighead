use super::render::RenderLine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspaceTab {
    Conversation,
    Activity,
    TickActions,
    Processes,
    Questions,
}

impl WorkspaceTab {
    pub(super) const ALL: [Self; 5] = [
        Self::Conversation,
        Self::Activity,
        Self::TickActions,
        Self::Processes,
        Self::Questions,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::Activity => "Activity",
            Self::TickActions => "Tick Actions",
            Self::Processes => "Processes",
            Self::Questions => "Questions",
        }
    }

    pub(super) fn compact_title(self) -> &'static str {
        match self {
            Self::Conversation => "Chat",
            Self::Activity => "Run",
            Self::TickActions => "Ticks",
            Self::Processes => "Procs",
            Self::Questions => "Ask",
        }
    }

    pub(super) fn shortcut(self) -> &'static str {
        match self {
            Self::Conversation => "Ctrl+[",
            Self::Activity => "Ctrl+]",
            Self::TickActions => "Ctrl+\\",
            Self::Processes => "Ctrl+P",
            Self::Questions => "Ctrl+?",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Conversation => 0,
            Self::Activity => 1,
            Self::TickActions => 2,
            Self::Processes => 3,
            Self::Questions => 4,
        }
    }

    pub(super) fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub(super) fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone)]
pub(super) struct LineBuffer {
    lines: Vec<RenderLine>,
    scroll_offset: usize,
    capacity: usize,
}

impl LineBuffer {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            lines: Vec::new(),
            scroll_offset: 0,
            capacity,
        }
    }

    pub(super) fn lines(&self) -> &[RenderLine] {
        &self.lines
    }

    pub(super) fn len(&self) -> usize {
        self.lines.len()
    }

    pub(super) fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub(super) fn is_following(&self) -> bool {
        self.scroll_offset == 0
    }

    pub(super) fn push(&mut self, incoming: impl IntoIterator<Item = RenderLine>) -> usize {
        let start = self.lines.len();
        self.lines.extend(incoming);
        let added = self.lines.len().saturating_sub(start);
        if self.scroll_offset > 0 {
            self.scroll_offset = self.scroll_offset.saturating_add(added);
        }
        self.trim();
        added
    }

    fn trim(&mut self) {
        if self.lines.len() <= self.capacity {
            return;
        }
        let drop_count = self.lines.len() - self.capacity;
        self.lines.drain(0..drop_count);
        self.scroll_offset = self.scroll_offset.saturating_sub(drop_count);
    }

    fn scroll_up(&mut self, lines: usize) {
        // Wrapped terminal rows can outnumber logical lines, so retain a generous bounded range.
        let upper_bound = self.capacity.saturating_mul(8);
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(upper_bound);
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    fn follow(&mut self) {
        self.scroll_offset = 0;
    }
}

#[derive(Debug, Clone)]
pub(super) struct WorkspaceState {
    active: WorkspaceTab,
    feeds: [LineBuffer; 5],
    unread: [usize; 5],
}

impl WorkspaceState {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            active: WorkspaceTab::Conversation,
            feeds: std::array::from_fn(|_| LineBuffer::new(capacity)),
            unread: [0; 5],
        }
    }

    pub(super) fn active(&self) -> WorkspaceTab {
        self.active
    }

    pub(super) fn switch_to(&mut self, tab: WorkspaceTab) -> bool {
        if self.active == tab {
            return false;
        }
        self.active = tab;
        self.unread[tab.index()] = 0;
        true
    }

    pub(super) fn cycle_forward(&mut self) {
        self.switch_to(self.active.next());
    }

    pub(super) fn cycle_backward(&mut self) {
        self.switch_to(self.active.previous());
    }

    pub(super) fn push(&mut self, tab: WorkspaceTab, lines: impl IntoIterator<Item = RenderLine>) {
        let added = self.feeds[tab.index()].push(lines);
        if tab != self.active {
            self.unread[tab.index()] = self.unread[tab.index()].saturating_add(added);
        }
    }

    pub(super) fn feed(&self, tab: WorkspaceTab) -> &LineBuffer {
        &self.feeds[tab.index()]
    }

    pub(super) fn active_feed(&self) -> &LineBuffer {
        self.feed(self.active)
    }

    fn active_feed_mut(&mut self) -> &mut LineBuffer {
        &mut self.feeds[self.active.index()]
    }

    pub(super) fn unread(&self, tab: WorkspaceTab) -> usize {
        self.unread[tab.index()]
    }

    pub(super) fn scroll_up(&mut self, lines: usize) {
        self.active_feed_mut().scroll_up(lines);
    }

    pub(super) fn scroll_down(&mut self, lines: usize) {
        self.active_feed_mut().scroll_down(lines);
    }

    pub(super) fn follow(&mut self) {
        self.active_feed_mut().follow();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::LineStyle;

    fn line(text: &str) -> RenderLine {
        RenderLine {
            text: text.into(),
            style: LineStyle::Normal,
        }
    }

    #[test]
    fn tabs_cycle_and_clear_unread_when_opened() {
        let mut workspace = WorkspaceState::new(10);
        workspace.push(WorkspaceTab::Activity, [line("tool call")]);
        assert_eq!(workspace.unread(WorkspaceTab::Activity), 1);

        workspace.cycle_forward();
        assert_eq!(workspace.active(), WorkspaceTab::Activity);
        assert_eq!(workspace.unread(WorkspaceTab::Activity), 0);

        workspace.cycle_backward();
        assert_eq!(workspace.active(), WorkspaceTab::Conversation);
        assert_eq!(WorkspaceTab::Conversation.compact_title(), "Chat");
        assert_eq!(WorkspaceTab::TickActions.compact_title(), "Ticks");
        assert_eq!(WorkspaceTab::Processes.compact_title(), "Procs");
        assert_eq!(WorkspaceTab::Activity.shortcut(), "Ctrl+]");
    }

    #[test]
    fn line_buffers_are_bounded_and_keep_scroll_state_per_tab() {
        let mut workspace = WorkspaceState::new(3);
        workspace.push(
            WorkspaceTab::Conversation,
            [line("one"), line("two"), line("three"), line("four")],
        );
        workspace.scroll_up(2);
        workspace.push(WorkspaceTab::Conversation, [line("five")]);

        let conversation = workspace.feed(WorkspaceTab::Conversation);
        assert_eq!(conversation.len(), 3);
        assert_eq!(conversation.lines()[0].text, "three");
        assert!(!conversation.is_following());

        workspace.switch_to(WorkspaceTab::Activity);
        assert!(workspace.active_feed().is_following());
    }
}
