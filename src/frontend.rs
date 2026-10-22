#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendMode {
    Tui,
    Headless,
    Cli,
}

impl FrontendMode {
    pub(crate) fn from_args(args: &[String]) -> Self {
        if args.iter().any(|a| a == "--tui") {
            Self::Tui
        } else if args
            .iter()
            .any(|a| a == "--headless" || a == "--desktop-server")
        {
            Self::Headless
        } else {
            Self::Cli
        }
    }

    pub(crate) fn source_label(self) -> &'static str {
        match self {
            Self::Tui => "Big Head TUI",
            Self::Headless => "headless stdin",
            Self::Cli => "terminal CLI",
        }
    }

    pub(crate) fn is_tui(self) -> bool {
        matches!(self, Self::Tui)
    }

    pub(crate) fn is_headless(self) -> bool {
        matches!(self, Self::Headless)
    }
}
