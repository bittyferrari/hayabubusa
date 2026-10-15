//! Load system fonts with CJK glyphs so Chinese / Japanese / Korean text
//! (UI and file names) renders.
//!
//! The first font found for the UI language becomes the primary CJK font, so
//! shared Han characters get that language's glyph shapes. If it has no Hangul,
//! a Korean font is appended as a fallback so Korean file names still render.

use crate::i18n::Lang;
use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

type Candidate = (&'static str, u32);

const ZH_TW: &[Candidate] = &[
    (r"C:\Windows\Fonts\msjh.ttc", 0), // Microsoft JhengHei (繁中)
    (r"C:\Windows\Fonts\msyh.ttc", 0), // Microsoft YaHei
    (r"C:\Windows\Fonts\mingliu.ttc", 0),
    ("/System/Library/Fonts/PingFang.ttc", 2),
    ("/System/Library/Fonts/STHeiti Medium.ttc", 0),
    ("/System/Library/Fonts/STHeiti Light.ttc", 0),
];

const JA: &[Candidate] = &[
    (r"C:\Windows\Fonts\YuGothM.ttc", 0), // Yu Gothic Medium
    (r"C:\Windows\Fonts\meiryo.ttc", 0),
    (r"C:\Windows\Fonts\msgothic.ttc", 0),
    ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", 0),
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
];

const KO: &[Candidate] = &[
    (r"C:\Windows\Fonts\malgun.ttf", 0), // Malgun Gothic
    ("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0),
];

/// Fonts that cover everything (or are a last resort).
const COMMON: &[Candidate] = &[
    ("/Library/Fonts/Arial Unicode.ttf", 0),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
    (
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        0,
    ),
];

/// Fonts known to contain Hangul.
fn has_hangul(path: &str) -> bool {
    KO.iter().any(|(p, _)| *p == path)
        || path.contains("NotoSansCJK")
        || path.contains("Arial Unicode")
        || path.contains("DroidSansFallback")
}

fn search_order(lang: Lang) -> Vec<Candidate> {
    let groups: [&[Candidate]; 3] = match lang {
        Lang::ZhTw | Lang::En => [ZH_TW, JA, KO],
        Lang::Ja => [JA, ZH_TW, KO],
        Lang::Ko => [KO, ZH_TW, JA],
    };
    groups
        .into_iter()
        .flatten()
        .chain(COMMON)
        .copied()
        .collect()
}

fn load(c: &Candidate) -> Option<FontData> {
    let bytes = std::fs::read(c.0).ok()?;
    let mut fd = FontData::from_owned(bytes);
    fd.index = c.1;
    Some(fd)
}

pub fn cjk_font_definitions(lang: Lang) -> Option<FontDefinitions> {
    let order = search_order(lang);
    let (primary_path, primary) = order.iter().find_map(|c| load(c).map(|f| (c.0, f)))?;

    let mut defs = FontDefinitions::default();
    let mut names = vec!["cjk"];
    defs.font_data.insert("cjk".into(), Arc::new(primary));
    if !has_hangul(primary_path) {
        if let Some(fd) = KO.iter().find_map(load) {
            defs.font_data.insert("hangul".into(), Arc::new(fd));
            names.push("hangul");
        }
    }
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = defs.families.entry(fam).or_default();
        list.extend(names.iter().map(|n| n.to_string()));
    }
    Some(defs)
}
