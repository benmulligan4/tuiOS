/// Settings state — enums and structs for the Settings page UI state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCategory {
    About,
    WifiBluetooth,
    Audio,
    Display,
    Appearance,
    ButtonMapping,
    Hotkeys,
    Git,
    Utilities,
    Power,
}

impl SettingsCategory {
    pub const ALL: &'static [SettingsCategory] = &[
        SettingsCategory::About,
        SettingsCategory::WifiBluetooth,
        SettingsCategory::Audio,
        SettingsCategory::Display,
        SettingsCategory::Appearance,
        SettingsCategory::ButtonMapping,
        SettingsCategory::Hotkeys,
        SettingsCategory::Git,
        SettingsCategory::Utilities,
        SettingsCategory::Power,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SettingsCategory::About => "About tuiOS",
            SettingsCategory::WifiBluetooth => "WiFi & Bluetooth",
            SettingsCategory::Audio => "Audio",
            SettingsCategory::Display => "Display",
            SettingsCategory::Appearance => "Appearance",
            SettingsCategory::ButtonMapping => "Button Mapping",
            SettingsCategory::Hotkeys => "Hotkeys",
            SettingsCategory::Git => "Scripts & Git",
            SettingsCategory::Utilities => "Utilities",
            SettingsCategory::Power => "Power",
        }
    }

    /// Returns true if this category is only available on Raspberry Pi.
    pub fn is_pi_only(&self) -> bool {
        matches!(
            self,
            SettingsCategory::WifiBluetooth | SettingsCategory::Audio
        )
    }

    /// Returns true if the current platform can use this category.
    pub fn is_available(&self) -> bool {
        if self.is_pi_only() {
            cfg!(target_os = "linux")
        } else {
            true
        }
    }
}

/// A page opened from inside a category rather than from the category list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubPage {
    BootAnimation,
}

impl SubPage {
    pub fn label(&self) -> &'static str {
        match self {
            SubPage::BootAnimation => "Boot Up Animation",
        }
    }
}

/// UI state for the Settings page.
#[derive(Debug, Clone)]
pub struct SettingsState {
    /// Index into SettingsCategory::ALL for the left pane.
    pub category_cursor: usize,
    /// Whether the right (content) pane is focused.
    pub in_right_pane: bool,
    /// Cursor position within the right pane items.
    pub right_cursor: usize,
    /// Scroll offset for the right pane content.
    pub right_scroll: usize,
    /// When true, the next key press will be captured as a button mapping.
    pub awaiting_key: bool,
    /// Terminal output from script execution (cleared on category switch).
    pub terminal_output: Vec<String>,
    /// Whether focus is inside the terminal output panel (Scripts & Git).
    pub terminal_focused: bool,
    /// Scroll offset for the terminal output panel.
    pub terminal_scroll: usize,
    /// When true, the current multi-option setting is in left/right cycling mode.
    pub editing_setting: bool,
    /// Sub-page opened from within the selected category.
    pub subpage: Option<SubPage>,
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            category_cursor: 0,
            in_right_pane: false,
            right_cursor: 0,
            right_scroll: 0,
            awaiting_key: false,
            terminal_output: Vec::new(),
            terminal_focused: false,
            terminal_scroll: 0,
            editing_setting: false,
            subpage: None,
        }
    }

    /// Get the currently selected category.
    pub fn selected_category(&self) -> SettingsCategory {
        SettingsCategory::ALL[self.category_cursor]
    }

    /// Reset right pane state when switching categories.
    pub fn reset_right_pane(&mut self) {
        self.right_cursor = 0;
        self.right_scroll = 0;
        self.terminal_output.clear();
        self.terminal_focused = false;
        self.terminal_scroll = 0;
        self.editing_setting = false;
        self.subpage = None;
    }
}
