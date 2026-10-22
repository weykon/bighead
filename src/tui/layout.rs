pub(super) const MIN_WIDTH: u16 = 48;
pub(super) const MIN_HEIGHT: u16 = 14;

const HEADER_HEIGHT: u16 = 3;
const COMPACT_INPUT_HEIGHT: u16 = 4;
const REGULAR_INPUT_HEIGHT: u16 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rect {
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) width: u16,
    pub(super) height: u16,
}

impl Rect {
    pub(super) fn contains_row(self, row: u16) -> bool {
        row >= self.y && row < self.y.saturating_add(self.height)
    }

    pub(super) fn inset(self, horizontal: u16, vertical: u16) -> Self {
        Self {
            x: self.x.saturating_add(horizontal),
            y: self.y.saturating_add(vertical),
            width: self.width.saturating_sub(horizontal.saturating_mul(2)),
            height: self.height.saturating_sub(vertical.saturating_mul(2)),
        }
    }

    pub(super) fn split_columns(self, left_percent: u16, gap: u16) -> Option<(Self, Self)> {
        if self.width < gap.saturating_add(2) {
            return None;
        }
        let usable = self.width - gap;
        let left_width = usable.saturating_mul(left_percent.min(100)) / 100;
        let right_width = usable.saturating_sub(left_width);
        Some((
            Self {
                width: left_width,
                ..self
            },
            Self {
                x: self.x.saturating_add(left_width).saturating_add(gap),
                width: right_width,
                ..self
            },
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TuiLayout {
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) header: Rect,
    pub(super) body: Rect,
    pub(super) input: Rect,
    pub(super) wide: bool,
}

impl TuiLayout {
    pub(super) fn new(width: u16, height: u16) -> Option<Self> {
        if width < MIN_WIDTH || height < MIN_HEIGHT {
            return None;
        }

        let input_height = if height >= 24 {
            REGULAR_INPUT_HEIGHT
        } else {
            COMPACT_INPUT_HEIGHT
        };
        let body_height = height.saturating_sub(HEADER_HEIGHT + input_height);

        Some(Self {
            width,
            height,
            header: Rect {
                x: 0,
                y: 0,
                width,
                height: HEADER_HEIGHT,
            },
            body: Rect {
                x: 0,
                y: HEADER_HEIGHT,
                width,
                height: body_height,
            },
            input: Rect {
                x: 0,
                y: HEADER_HEIGHT + body_height,
                width,
                height: input_height,
            },
            wide: width >= 96 && body_height >= 12,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_layout_gives_the_workspace_remaining_height() {
        let layout = TuiLayout::new(120, 40).expect("layout");
        assert_eq!(layout.header.height, 3);
        assert_eq!(layout.body.y, 3);
        assert_eq!(layout.body.height, 31);
        assert_eq!(layout.input.y, 34);
        assert_eq!(layout.input.height, 6);
        assert!(layout.wide);
        assert!(layout.body.contains_row(20));
        assert!(!layout.body.contains_row(35));
    }

    #[test]
    fn compact_layout_reclaims_rows_from_the_input() {
        let layout = TuiLayout::new(72, 20).expect("layout");
        assert_eq!(layout.body.height, 13);
        assert_eq!(layout.input.height, 4);
        assert!(!layout.wide);
    }

    #[test]
    fn rect_split_accounts_for_the_gap() {
        let rect = Rect {
            x: 2,
            y: 4,
            width: 100,
            height: 8,
        };
        let (left, right) = rect.split_columns(40, 2).expect("columns");
        assert_eq!(left.width + right.width + 2, rect.width);
        assert_eq!(right.x, left.x + left.width + 2);
    }

    #[test]
    fn layout_rejects_tiny_terminals() {
        assert!(TuiLayout::new(MIN_WIDTH - 1, MIN_HEIGHT).is_none());
        assert!(TuiLayout::new(MIN_WIDTH, MIN_HEIGHT - 1).is_none());
    }
}
