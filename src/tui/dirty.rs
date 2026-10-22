use std::time::{Duration, Instant};

const MIN_RENDER_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DirtyAreas {
    pub(super) chrome: bool,
    pub(super) body: bool,
    pub(super) input: bool,
}

impl DirtyAreas {
    pub(super) fn all() -> Self {
        Self {
            chrome: true,
            body: true,
            input: true,
        }
    }

    pub(super) fn chrome() -> Self {
        Self {
            chrome: true,
            ..Self::default()
        }
    }

    pub(super) fn input() -> Self {
        Self {
            input: true,
            ..Self::default()
        }
    }

    pub(super) fn runtime() -> Self {
        Self {
            chrome: true,
            body: true,
            ..Self::default()
        }
    }

    pub(super) fn any(self) -> bool {
        self.chrome || self.body || self.input
    }

    fn immediate(self) -> bool {
        // Keystrokes should feel immediate. Runtime status/activity can arrive in
        // bursts, so chrome updates share the normal frame-rate limit.
        self.input
    }

    pub(super) fn include(&mut self, other: Self) {
        self.chrome |= other.chrome;
        self.body |= other.body;
        self.input |= other.input;
    }
}

pub(super) fn dirty_if(changed: bool, dirty: DirtyAreas) -> DirtyAreas {
    if changed {
        dirty
    } else {
        DirtyAreas::default()
    }
}

pub(super) fn should_render(dirty: DirtyAreas, last_draw: Option<Instant>) -> bool {
    if !dirty.any() {
        return false;
    }
    if dirty.immediate() {
        return true;
    }
    last_draw.is_none_or(|drawn_at| drawn_at.elapsed() >= MIN_RENDER_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_updates_are_throttled_but_input_is_immediate() {
        let just_drawn = Some(Instant::now());

        assert!(!should_render(DirtyAreas::runtime(), just_drawn));
        assert!(should_render(DirtyAreas::input(), just_drawn));
    }

    #[test]
    fn clean_frames_never_render() {
        assert!(!should_render(DirtyAreas::default(), None));
    }
}
