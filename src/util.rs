use std::cmp::Ordering;
use std::path::{Path, PathBuf};

pub const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "webp", "gif", "bmp", "tif", "tiff", "ico", "tga",
];

pub fn ext_lower(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn is_image(p: &Path) -> bool {
    let e = ext_lower(p);
    IMAGE_EXTS.contains(&e.as_str())
}

pub fn is_jpeg(p: &Path) -> bool {
    matches!(ext_lower(p).as_str(), "jpg" | "jpeg" | "jfif")
}

/// Case-insensitive "natural" compare: img2 < img10.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a: Vec<char> = a.chars().flat_map(|c| c.to_lowercase()).collect();
    let b: Vec<char> = b.chars().flat_map(|c| c.to_lowercase()).collect();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = a[si..i].iter().collect::<String>();
            let nb: String = b[sj..j].iter().collect::<String>();
            let na = na.trim_start_matches('0');
            let nb = nb.trim_start_matches('0');
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let ord = a[i].cmp(&b[j]);
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// Stable 64-bit FNV-1a hash (stable across builds, used for cache keys).
pub fn fnv64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn human_size(n: u64) -> String {
    let f = n as f64;
    if f >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", f / 1024.0 / 1024.0 / 1024.0)
    } else if f >= 1024.0 * 1024.0 {
        format!("{:.1} MB", f / 1024.0 / 1024.0)
    } else if f >= 1024.0 {
        format!("{:.0} KB", f / 1024.0)
    } else {
        format!("{n} B")
    }
}

pub fn app_dir(kind: fn() -> Option<PathBuf>) -> PathBuf {
    kind().unwrap_or_else(std::env::temp_dir).join("hayabubusa")
}

pub fn config_file() -> PathBuf {
    app_dir(dirs::config_dir).join("last_dir.txt")
}

pub fn load_last_dir() -> Option<PathBuf> {
    let s = std::fs::read_to_string(config_file()).ok()?;
    let p = PathBuf::from(s.trim());
    p.is_dir().then_some(p)
}

pub fn save_last_dir(p: &Path) {
    let f = config_file();
    if let Some(parent) = f.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(f, p.to_string_lossy().as_bytes());
}

/// Pick a file name that does not exist yet: name.ext, name_1.ext, ...
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let p = dir.join(format!("{stem}.{ext}"));
    if !p.exists() {
        return p;
    }
    for i in 1.. {
        let p = dir.join(format!("{stem}_{i}.{ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural() {
        let mut v = vec!["img10.jpg", "img2.jpg", "IMG1.jpg", "a.jpg", "img02b.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            vec!["a.jpg", "IMG1.jpg", "img2.jpg", "img02b.jpg", "img10.jpg"]
        );
    }
}

/// canonicalize, but without the Windows `\\?\` prefix (keeps paths readable and comparable).
pub fn canon(p: PathBuf) -> PathBuf {
    let c = match p.canonicalize() {
        Ok(c) => c,
        Err(_) => return p,
    };
    let s = c.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC") {
            return PathBuf::from(rest.to_string());
        }
    }
    c
}

// ---------------------------------------------------------------------- settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowMode {
    /// Restore last size / position / maximized state.
    Remember,
    /// Always start maximized.
    Maximized,
}

/// Sort order for the folder (thumbnail) view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Modified,
    Created,
    Size,
    Type,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [
        SortKey::Name,
        SortKey::Modified,
        SortKey::Created,
        SortKey::Size,
        SortKey::Type,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => crate::i18n::tr().sort_name,
            SortKey::Modified => crate::i18n::tr().sort_modified,
            SortKey::Created => crate::i18n::tr().sort_created,
            SortKey::Size => crate::i18n::tr().sort_size,
            SortKey::Type => crate::i18n::tr().sort_type,
        }
    }

    fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Modified => "modified",
            SortKey::Created => "created",
            SortKey::Size => "size",
            SortKey::Type => "type",
        }
    }

    fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Thumbnail rows scrolled per mouse-wheel notch.
    pub scroll_rows: f32,
    /// Animate wheel scrolling in the thumbnail grid.
    pub smooth_scroll: bool,
    pub window_mode: WindowMode,
    /// Last window state (logical points). `None` = never saved.
    pub win_maximized: Option<bool>,
    pub win_size: Option<[f32; 2]>,
    pub win_pos: Option<[f32; 2]>,
    pub thumb_size: f32,
    pub sort_key: SortKey,
    pub sort_desc: bool,
    /// Ask before sending files to the Recycle Bin.
    pub confirm_delete: bool,
    /// Closing the window keeps Hayabubusa running in the tray (Windows).
    pub stay_resident: bool,
    /// While resident, exit after this many minutes without opening anything (0 = never).
    pub idle_exit_min: u32,
    /// UI language; `None` = follow the system language.
    pub lang: Option<crate::i18n::Lang>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scroll_rows: 1.0,
            smooth_scroll: true,
            window_mode: WindowMode::Remember,
            win_maximized: None,
            win_size: None,
            win_pos: None,
            thumb_size: 160.0,
            sort_key: SortKey::Name,
            sort_desc: false,
            confirm_delete: true,
            stay_resident: true,
            idle_exit_min: 5,
            lang: None,
        }
    }
}

pub fn settings_file() -> PathBuf {
    app_dir(dirs::config_dir).join("settings.txt")
}

fn parse_pair(v: &str) -> Option<[f32; 2]> {
    let (a, b) = v.split_once(',')?;
    let a: f32 = a.trim().parse().ok()?;
    let b: f32 = b.trim().parse().ok()?;
    (a.is_finite() && b.is_finite()).then_some([a, b])
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Ok(text) = std::fs::read_to_string(settings_file()) else {
            return s;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "scroll_rows" => {
                    if let Ok(x) = v.parse::<f32>() {
                        if x.is_finite() {
                            s.scroll_rows = x.clamp(0.1, 20.0);
                        }
                    }
                }
                "smooth_scroll" => s.smooth_scroll = v == "1" || v == "true",
                "window_mode" => {
                    s.window_mode = if v == "maximized" {
                        WindowMode::Maximized
                    } else {
                        WindowMode::Remember
                    }
                }
                "win_maximized" => s.win_maximized = Some(v == "1" || v == "true"),
                "win_size" => s.win_size = parse_pair(v).filter(|p| p[0] >= 200.0 && p[1] >= 150.0),
                "win_pos" => {
                    s.win_pos = parse_pair(v).filter(|p| p[0].abs() < 30000.0 && p[1].abs() < 30000.0)
                }
                "thumb_size" => {
                    if let Ok(x) = v.parse::<f32>() {
                        if x.is_finite() {
                            s.thumb_size = x.clamp(64.0, 256.0);
                        }
                    }
                }
                "sort_key" => {
                    if let Some(k) = SortKey::from_id(v) {
                        s.sort_key = k;
                    }
                }
                "sort_desc" => s.sort_desc = v == "1" || v == "true",
                "confirm_delete" => s.confirm_delete = v == "1" || v == "true",
                "stay_resident" => s.stay_resident = v == "1" || v == "true",
                "idle_exit_min" => {
                    if let Ok(x) = v.parse::<u32>() {
                        s.idle_exit_min = x.min(24 * 60);
                    }
                }
                "lang" => s.lang = crate::i18n::Lang::from_id(v),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let mut out = String::new();
        out += &format!("scroll_rows={}\n", self.scroll_rows);
        out += &format!("smooth_scroll={}\n", self.smooth_scroll as u8);
        out += &format!(
            "window_mode={}\n",
            match self.window_mode {
                WindowMode::Remember => "remember",
                WindowMode::Maximized => "maximized",
            }
        );
        if let Some(m) = self.win_maximized {
            out += &format!("win_maximized={}\n", m as u8);
        }
        if let Some([w, h]) = self.win_size {
            out += &format!("win_size={w},{h}\n");
        }
        if let Some([x, y]) = self.win_pos {
            out += &format!("win_pos={x},{y}\n");
        }
        out += &format!("thumb_size={}\n", self.thumb_size);
        out += &format!("sort_key={}\n", self.sort_key.id());
        out += &format!("sort_desc={}\n", self.sort_desc as u8);
        out += &format!("confirm_delete={}\n", self.confirm_delete as u8);
        out += &format!("stay_resident={}\n", self.stay_resident as u8);
        out += &format!("idle_exit_min={}\n", self.idle_exit_min);
        out += &format!("lang={}\n", self.lang.map_or("auto", |l| l.id()));
        let f = settings_file();
        if let Some(parent) = f.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(f, out);
    }
}
