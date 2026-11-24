use crate::namespace::Namespace;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Location {
    #[default]
    Home,
    Computer,
    Settings,
    Namespace {
        kind: Namespace,
        child: Option<String>,
        title: Option<String>,
    },
    Folder(PathBuf),
    Search {
        root: PathBuf,
        query: String,
    },
    PcSearch {
        query: String,
        all_drives: bool,
        from_home: bool,
    },
}

impl Location {
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Folder(path.into())
    }

    pub fn namespace(kind: Namespace) -> Self {
        Self::Namespace {
            kind,
            child: None,
            title: None,
        }
    }

    pub fn directory(&self) -> Option<&Path> {
        match self {
            Self::Folder(path) | Self::Search { root: path, .. } => Some(path),
            _ => None,
        }
    }

    pub fn is_search(&self) -> bool {
        matches!(self, Self::Search { .. } | Self::PcSearch { .. })
    }

    pub fn caption(&self) -> String {
        match self {
            Self::Home => "Home".into(),
            Self::Computer => "This PC".into(),
            Self::Settings => "Settings".into(),
            Self::Namespace { kind, title, .. } => title.clone().unwrap_or_else(|| match kind {
                Namespace::Network => "Network".into(),
                Namespace::RecycleBin => "Recycle Bin".into(),
                Namespace::Libraries => "Libraries".into(),
            }),
            Self::Search { query, .. } | Self::PcSearch { query, .. } => format!("Search: {query}"),
            Self::Folder(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into(),
        }
    }
}

#[derive(Debug)]
pub struct History {
    items: Vec<Location>,
    cursor: usize,
}

impl History {
    pub fn new(location: Location) -> Self {
        Self {
            items: vec![location],
            cursor: 0,
        }
    }
    pub fn current(&self) -> &Location {
        &self.items[self.cursor]
    }
    pub fn can_back(&self) -> bool {
        self.cursor != 0
    }
    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.items.len()
    }
    pub fn open(&mut self, location: Location) {
        if self.current() == &location {
            return;
        }
        self.items.truncate(self.cursor + 1);
        self.items.push(location);
        if self.items.len() > 100 {
            self.items.remove(0);
        }
        self.cursor = self.items.len() - 1;
    }
    pub fn back(&mut self) -> Option<&Location> {
        if !self.can_back() {
            return None;
        }
        self.cursor -= 1;
        Some(self.current())
    }
    pub fn forward(&mut self) -> Option<&Location> {
        if !self.can_forward() {
            return None;
        }
        self.cursor += 1;
        Some(self.current())
    }
}

pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub fn from_wide(units: &[u16]) -> OsString {
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    OsString::from_wide(&units[..end])
}

pub fn safe_name(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name.encode_utf16().count() > 255
        || name.chars().any(|c| c < ' ' || r#"\/:*?"<>|"#.contains(c))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    !stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hidden: bool,
    pub hide_extensions: bool,
    pub details: bool,
    pub modified_date_format: ModifiedDateFormat,
    pub theme: ThemePreference,
    pub restore_tabs: bool,
    pub active: usize,
    pub tabs: Vec<Location>,
    pub pins: Vec<PathBuf>,
    pub recent: Vec<PathBuf>,
    pub favorites: Vec<PathBuf>,
    pub home_compact: bool,
    pub views: BTreeMap<PathBuf, u32>,
    pub folder_orders: BTreeMap<PathBuf, FolderOrder>,
}

impl Settings {
    pub fn folder_order(&self, path: &Path) -> FolderOrder {
        self.folder_orders.get(path).copied().unwrap_or_default()
    }

    pub fn record_recent(&mut self, path: PathBuf) {
        self.recent.retain(|entry| entry != &path);
        self.recent.insert(0, path);
        self.recent.truncate(40);
    }

    pub fn remember_tabs(&mut self, tabs: impl IntoIterator<Item = Location>, active: usize) {
        self.tabs.clear();
        self.active = 0;
        for (index, location) in tabs.into_iter().enumerate() {
            if location == Location::Settings {
                continue;
            }
            if index <= active {
                self.active = self.tabs.len();
            }
            self.tabs.push(location);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Column {
    #[default]
    Name,
    Modified,
    Kind,
    Folder,
    Size,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Sort {
    pub const NEWEST_FIRST: Self = Self {
        column: Column::Modified,
        descending: true,
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupBy {
    None,
    #[default]
    DateModified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderOrder {
    pub sort: Sort,
    pub group: GroupBy,
}

impl Default for FolderOrder {
    fn default() -> Self {
        Self {
            sort: Sort::NEWEST_FIRST,
            group: GroupBy::DateModified,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ModifiedDateFormat {
    #[default]
    DayMonthYear,
    MonthDayYear,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
    DeepSpace,
    PaperWhite,
    CandyRush,
    Frost,
    Eclipse,
    CyberGrid,
}

impl ThemePreference {
    pub const ALL: [Self; 9] = [
        Self::System,
        Self::Light,
        Self::Dark,
        Self::DeepSpace,
        Self::PaperWhite,
        Self::CandyRush,
        Self::Frost,
        Self::Eclipse,
        Self::CyberGrid,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::DeepSpace => "Deep Space",
            Self::PaperWhite => "Paper White",
            Self::CandyRush => "Candy Rush",
            Self::Frost => "Frost",
            Self::Eclipse => "Eclipse",
            Self::CyberGrid => "Cyber Grid",
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    if std::env::var_os("BETTEREXPLORER_EPHEMERAL").is_some() {
        return None;
    }
    Some(
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
            .join("BetterExplorer")
            .join("settings.json"),
    )
}

pub fn read_settings() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return Settings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn write_settings(settings: &Settings) -> std::io::Result<()> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temp, serde_json::to_vec(settings)?)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(temp);
        return Err(error);
    }
    Ok(())
}
