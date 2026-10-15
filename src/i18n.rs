//! UI translations: 繁體中文 / English / 日本語 / 한국어.
//!
//! The active language is a process-wide setting (`set` / `current`), so any
//! thread (file-operation workers, the tray menu) can call `tr()`.
//! Strings with values use `{0}`, `{1}` placeholders, filled by `fill`.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    ZhTw,
    En,
    Ja,
    Ko,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::ZhTw, Lang::En, Lang::Ja, Lang::Ko];

    /// Name shown in the language picker (always in its own language).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::ZhTw => "繁體中文",
            Lang::En => "English",
            Lang::Ja => "日本語",
            Lang::Ko => "한국어",
        }
    }

    /// Id stored in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Lang::ZhTw => "zh-TW",
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|l| l.id().eq_ignore_ascii_case(s))
    }

    /// Map a BCP-47 / POSIX locale ("zh-TW", "ja_JP.UTF-8", "ko-KR" …) to a UI language.
    pub fn from_locale(loc: &str) -> Self {
        let l = loc.to_ascii_lowercase();
        let primary = l.split(['-', '_', '.', '@']).next().unwrap_or("");
        match primary {
            // No Simplified Chinese table: all Chinese locales use 繁體中文.
            "zh" => Lang::ZhTw,
            "ja" => Lang::Ja,
            "ko" => Lang::Ko,
            _ => Lang::En,
        }
    }

    fn strings(self) -> &'static Strings {
        match self {
            Lang::ZhTw => &ZH_TW,
            Lang::En => &EN,
            Lang::Ja => &JA,
            Lang::Ko => &KO,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(1); // En until `set` is called

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    Lang::ALL
        .get(CURRENT.load(Ordering::Relaxed) as usize)
        .copied()
        .unwrap_or(Lang::En)
}

/// Language of the operating system's UI.
pub fn system() -> Lang {
    sys_locale::get_locale()
        .map(|l| Lang::from_locale(&l))
        .unwrap_or(Lang::En)
}

/// `None` (setting = "follow system") resolves to the OS language.
pub fn resolve(pref: Option<Lang>) -> Lang {
    pref.unwrap_or_else(system)
}

/// Strings for the active language.
pub fn tr() -> &'static Strings {
    current().strings()
}

/// Replace `{0}`, `{1}`, … in `tpl` with `args`.
pub fn fill(tpl: &str, args: &[&dyn Display]) -> String {
    let mut out = tpl.to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &a.to_string());
    }
    out
}

#[cfg_attr(not(windows), allow(dead_code))] // some strings are Windows-only
pub struct Strings {
    // --- toasts / file operations
    pub refreshed: &'static str,
    pub copied_n: &'static str,
    pub cut_n: &'static str,
    pub fs_busy: &'static str,
    pub clipboard_no_files: &'static str,
    pub already_here: &'static str,
    pub moved_n: &'static str,
    pub pasted_n: &'static str,
    pub cancelled: &'static str,
    pub recycled_n: &'static str,
    pub rotation_discarded: &'static str,
    pub rotating_n: &'static str,
    pub converting_n: &'static str,
    pub batch_done: &'static str,
    pub cannot_open: &'static str,
    // --- delete confirmation
    pub delete_title: &'static str,
    pub delete_confirm_n: &'static str,
    pub and_more_n: &'static str,
    pub dont_ask_again: &'static str,
    pub delete_enter: &'static str,
    pub cancel_esc: &'static str,
    // --- browser toolbar / status bar
    pub up: &'static str,
    pub refresh_tip: &'static str,
    pub rotate_left: &'static str,
    pub rotate_right: &'static str,
    pub convert_resize_menu: &'static str,
    pub paste: &'static str,
    pub delete: &'static str,
    pub delete_tip: &'static str,
    pub thumbs: &'static str,
    pub thumbs_tip: &'static str,
    pub sort: &'static str,
    pub descending: &'static str,
    pub ascending: &'static str,
    pub sort_dir_tip: &'static str,
    pub settings_btn: &'static str,
    pub status_counts: &'static str,
    pub selected_n: &'static str,
    // --- context menu
    pub open: &'static str,
    pub cut_menu: &'static str,
    pub copy_menu: &'static str,
    pub paste_here_menu: &'static str,
    pub delete_menu: &'static str,
    // --- viewer toolbar
    pub back_enter: &'static str,
    pub prev_tip: &'static str,
    pub next_tip: &'static str,
    pub rotate_left_tip: &'static str,
    pub rotate_right_tip: &'static str,
    pub save_rotation: &'static str,
    // --- convert dialog
    pub convert_title: &'static str,
    pub n_files: &'static str,
    pub includes_rotation: &'static str,
    pub format: &'static str,
    pub quality: &'static str,
    pub lossless: &'static str,
    pub png_lossless: &'static str,
    pub resize: &'static str,
    pub resize_kinds: [&'static str; 5],
    pub width_prefix: &'static str,
    pub height_prefix: &'static str,
    pub no_upscale: &'static str,
    pub output: &'static str,
    pub same_folder: &'static str,
    pub subfolder: &'static str,
    pub overwrite: &'static str,
    pub start: &'static str,
    pub cancel: &'static str,
    pub keep_format: &'static str,
    // --- batch progress
    pub done_errors_n: &'static str,
    pub close: &'static str,
    // --- settings
    pub settings_title: &'static str,
    pub language: &'static str,
    pub lang_system: &'static str,
    pub scroll_per_notch: &'static str,
    pub thumb_rows: &'static str,
    pub smooth_scroll: &'static str,
    pub confirm_delete: &'static str,
    pub window_on_start: &'static str,
    pub remember_window: &'static str,
    pub always_maximized: &'static str,
    pub after_close: &'static str,
    pub stay_resident: &'static str,
    pub idle_exit: &'static str,
    // --- sort keys
    pub sort_name: &'static str,
    pub sort_modified: &'static str,
    pub sort_created: &'static str,
    pub sort_size: &'static str,
    pub sort_type: &'static str,
    // --- clipboard / shell errors
    pub nothing_selected: &'static str,
    pub clipboard_busy: &'static str,
    pub clipboard_clear_failed: &'static str,
    pub clipboard_write_failed: &'static str,
    // Only read by the #[cfg(not(windows))] fallbacks in clipboard.rs / fileops.rs
    #[cfg_attr(windows, allow(dead_code))]
    pub clipboard_unsupported: &'static str,
    pub fileop_failed: &'static str,
    // Only read by the #[cfg(not(windows))] fallbacks in clipboard.rs / fileops.rs
    #[cfg_attr(windows, allow(dead_code))]
    pub paste_unsupported: &'static str,
    // Only read by the #[cfg(not(windows))] fallbacks in clipboard.rs / fileops.rs
    #[cfg_attr(windows, allow(dead_code))]
    pub delete_unsupported: &'static str,
    // --- tray menu
    pub tray_open: &'static str,
    pub tray_exit: &'static str,
}

static ZH_TW: Strings = Strings {
    refreshed: "已重新整理",
    copied_n: "已複製 {0} 個項目，可到檔案總管或桌面貼上",
    cut_n: "已剪下 {0} 個項目，可到檔案總管或桌面貼上",
    fs_busy: "上一個檔案操作還在進行中",
    clipboard_no_files: "剪貼簿裡沒有檔案",
    already_here: "檔案已經在這個資料夾",
    moved_n: "已移動 {0} 個項目",
    pasted_n: "已貼上 {0} 個項目",
    cancelled: "已取消",
    recycled_n: "已將 {0} 個項目移到資源回收筒",
    rotation_discarded: "旋轉未儲存（已捨棄）",
    rotating_n: "旋轉 {0} 個檔案",
    converting_n: "轉換 {0} 個檔案",
    batch_done: "{0} 完成",
    cannot_open: "無法開啟：{0}",
    delete_title: "刪除",
    delete_confirm_n: "要將 {0} 個項目移到資源回收筒嗎？",
    and_more_n: "…還有 {0} 個",
    dont_ask_again: "以後不要再問（可在 ⚙ 設定改回來）",
    delete_enter: "🗑 刪除 (Enter)",
    cancel_esc: "取消 (Esc)",
    up: "⬆ 上一層",
    refresh_tip: "重新整理 (F5)",
    rotate_left: "↺ 左轉",
    rotate_right: "↻ 右轉",
    convert_resize_menu: "轉換 / 縮放…",
    paste: "📋 貼上",
    delete: "🗑 刪除",
    delete_tip: "Delete（移到資源回收筒）",
    thumbs: "縮圖",
    thumbs_tip: "Ctrl + 滾輪也可以調整",
    sort: "排序",
    descending: "⬇ 遞減",
    ascending: "⬆ 遞增",
    sort_dir_tip: "切換遞增 / 遞減",
    settings_btn: "⚙ 設定",
    status_counts: "{0} 張圖片, {1} 個資料夾",
    selected_n: "已選 {0}",
    open: "開啟",
    cut_menu: "剪下 (Ctrl+X)",
    copy_menu: "複製 (Ctrl+C)",
    paste_here_menu: "貼上到這個資料夾 (Ctrl+V)",
    delete_menu: "🗑 刪除 (Delete)",
    back_enter: "◀ 返回 (Enter)",
    prev_tip: "上一張 (←)",
    next_tip: "下一張 (→)",
    rotate_left_tip: "左轉 (L)",
    rotate_right_tip: "右轉 (R)",
    save_rotation: "💾 儲存旋轉 (Ctrl+S)",
    convert_title: "轉換 / 縮放",
    n_files: "{0} 個檔案",
    includes_rotation: "（包含旋轉 {0}°）",
    format: "格式",
    quality: "品質",
    lossless: "(無損)",
    png_lossless: "(PNG 無損，不影響)",
    resize: "縮放",
    resize_kinds: [
        "不縮放",
        "百分比",
        "長邊",
        "限制寬高 (等比)",
        "指定寬高 (不等比)",
    ],
    width_prefix: "寬 ",
    height_prefix: "高 ",
    no_upscale: "不放大較小的圖片",
    output: "輸出",
    same_folder: "同資料夾",
    subfolder: "子資料夾",
    overwrite: "同名檔案直接覆寫（否則自動加 _1）",
    start: "開始",
    cancel: "取消",
    keep_format: "保持原格式",
    done_errors_n: "完成，{0} 個錯誤：",
    close: "關閉",
    settings_title: "設定",
    language: "語言 / Language",
    lang_system: "跟隨系統",
    scroll_per_notch: "滾輪一格捲動",
    thumb_rows: "列縮圖",
    smooth_scroll: "平滑捲動",
    confirm_delete: "刪除前確認",
    window_on_start: "開啟時視窗",
    remember_window: "記憶上次的狀態（大小 / 位置 / 最大化）",
    always_maximized: "總是最大化",
    after_close: "關閉視窗後",
    stay_resident: "常駐在系統匣（下次開圖更快）",
    idle_exit: "分鐘沒開圖就自動結束（0 = 不自動結束）",
    sort_name: "名稱",
    sort_modified: "修改時間",
    sort_created: "建立時間",
    sort_size: "檔案大小",
    sort_type: "類型",
    nothing_selected: "沒有選取任何項目",
    clipboard_busy: "剪貼簿正被其他程式使用，請再試一次",
    clipboard_clear_failed: "無法清空剪貼簿",
    clipboard_write_failed: "無法寫入剪貼簿",
    clipboard_unsupported: "此平台尚未支援複製檔案到剪貼簿",
    fileop_failed: "檔案操作失敗（錯誤碼 {0}）",
    paste_unsupported: "此平台尚未支援貼上檔案",
    delete_unsupported: "此平台尚未支援刪除檔案",
    tray_open: "開啟 Hayabubusa",
    tray_exit: "結束",
};

static EN: Strings = Strings {
    refreshed: "Refreshed",
    copied_n: "Copied {0} item(s) — paste them in Explorer or on the desktop",
    cut_n: "Cut {0} item(s) — paste them in Explorer or on the desktop",
    fs_busy: "The previous file operation is still running",
    clipboard_no_files: "No files on the clipboard",
    already_here: "The files are already in this folder",
    moved_n: "Moved {0} item(s)",
    pasted_n: "Pasted {0} item(s)",
    cancelled: "Cancelled",
    recycled_n: "Moved {0} item(s) to the Recycle Bin",
    rotation_discarded: "Rotation not saved (discarded)",
    rotating_n: "Rotating {0} file(s)",
    converting_n: "Converting {0} file(s)",
    batch_done: "{0} — done",
    cannot_open: "Cannot open: {0}",
    delete_title: "Delete",
    delete_confirm_n: "Move {0} item(s) to the Recycle Bin?",
    and_more_n: "…and {0} more",
    dont_ask_again: "Don't ask again (can be changed in ⚙ Settings)",
    delete_enter: "🗑 Delete (Enter)",
    cancel_esc: "Cancel (Esc)",
    up: "⬆ Up",
    refresh_tip: "Refresh (F5)",
    rotate_left: "↺ Rotate left",
    rotate_right: "↻ Rotate right",
    convert_resize_menu: "Convert / Resize…",
    paste: "📋 Paste",
    delete: "🗑 Delete",
    delete_tip: "Delete (move to Recycle Bin)",
    thumbs: "Thumbnails",
    thumbs_tip: "Ctrl + mouse wheel also works",
    sort: "Sort",
    descending: "⬇ Descending",
    ascending: "⬆ Ascending",
    sort_dir_tip: "Toggle ascending / descending",
    settings_btn: "⚙ Settings",
    status_counts: "{0} images, {1} folders",
    selected_n: "{0} selected",
    open: "Open",
    cut_menu: "Cut (Ctrl+X)",
    copy_menu: "Copy (Ctrl+C)",
    paste_here_menu: "Paste into this folder (Ctrl+V)",
    delete_menu: "🗑 Delete (Delete)",
    back_enter: "◀ Back (Enter)",
    prev_tip: "Previous (←)",
    next_tip: "Next (→)",
    rotate_left_tip: "Rotate left (L)",
    rotate_right_tip: "Rotate right (R)",
    save_rotation: "💾 Save rotation (Ctrl+S)",
    convert_title: "Convert / Resize",
    n_files: "{0} file(s)",
    includes_rotation: "(includes {0}° rotation)",
    format: "Format",
    quality: "Quality",
    lossless: "(lossless)",
    png_lossless: "(PNG is lossless — no effect)",
    resize: "Resize",
    resize_kinds: [
        "No resize",
        "Percentage",
        "Long side",
        "Fit within (keep ratio)",
        "Exact size (stretch)",
    ],
    width_prefix: "W ",
    height_prefix: "H ",
    no_upscale: "Don't enlarge smaller images",
    output: "Output",
    same_folder: "Same folder",
    subfolder: "Subfolder",
    overwrite: "Overwrite files with the same name (otherwise add _1)",
    start: "Start",
    cancel: "Cancel",
    keep_format: "Keep format",
    done_errors_n: "Done, {0} error(s):",
    close: "Close",
    settings_title: "Settings",
    language: "Language",
    lang_system: "System default",
    scroll_per_notch: "Mouse wheel scrolls",
    thumb_rows: "row(s) per notch",
    smooth_scroll: "Smooth scrolling",
    confirm_delete: "Confirm before deleting",
    window_on_start: "Window at startup",
    remember_window: "Remember last state (size / position / maximized)",
    always_maximized: "Always maximized",
    after_close: "After closing the window",
    stay_resident: "Stay in the system tray (opens images faster next time)",
    idle_exit: "min without opening an image, then exit (0 = never)",
    sort_name: "Name",
    sort_modified: "Date modified",
    sort_created: "Date created",
    sort_size: "Size",
    sort_type: "Type",
    nothing_selected: "Nothing is selected",
    clipboard_busy: "The clipboard is in use by another program — please try again",
    clipboard_clear_failed: "Could not clear the clipboard",
    clipboard_write_failed: "Could not write to the clipboard",
    clipboard_unsupported: "Copying files to the clipboard isn't supported on this platform yet",
    fileop_failed: "File operation failed (error {0})",
    paste_unsupported: "Pasting files isn't supported on this platform yet",
    delete_unsupported: "Deleting files isn't supported on this platform yet",
    tray_open: "Open Hayabubusa",
    tray_exit: "Exit",
};

static JA: Strings = Strings {
    refreshed: "更新しました",
    copied_n: "{0} 個の項目をコピーしました。エクスプローラーやデスクトップに貼り付けできます",
    cut_n: "{0} 個の項目を切り取りました。エクスプローラーやデスクトップに貼り付けできます",
    fs_busy: "前のファイル操作がまだ実行中です",
    clipboard_no_files: "クリップボードにファイルがありません",
    already_here: "ファイルはすでにこのフォルダーにあります",
    moved_n: "{0} 個の項目を移動しました",
    pasted_n: "{0} 個の項目を貼り付けました",
    cancelled: "キャンセルしました",
    recycled_n: "{0} 個の項目をごみ箱に移動しました",
    rotation_discarded: "回転は保存されていません（破棄しました）",
    rotating_n: "{0} 個のファイルを回転",
    converting_n: "{0} 個のファイルを変換",
    batch_done: "{0} 完了",
    cannot_open: "開けません：{0}",
    delete_title: "削除",
    delete_confirm_n: "{0} 個の項目をごみ箱に移動しますか？",
    and_more_n: "…ほか {0} 個",
    dont_ask_again: "今後は確認しない（⚙ 設定で戻せます）",
    delete_enter: "🗑 削除 (Enter)",
    cancel_esc: "キャンセル (Esc)",
    up: "⬆ 上へ",
    refresh_tip: "最新の情報に更新 (F5)",
    rotate_left: "↺ 左に回転",
    rotate_right: "↻ 右に回転",
    convert_resize_menu: "変換 / リサイズ…",
    paste: "📋 貼り付け",
    delete: "🗑 削除",
    delete_tip: "Delete（ごみ箱に移動）",
    thumbs: "サムネイル",
    thumbs_tip: "Ctrl + ホイールでも調整できます",
    sort: "並べ替え",
    descending: "⬇ 降順",
    ascending: "⬆ 昇順",
    sort_dir_tip: "昇順 / 降順を切り替え",
    settings_btn: "⚙ 設定",
    status_counts: "画像 {0} 枚、フォルダー {1} 個",
    selected_n: "{0} 個選択",
    open: "開く",
    cut_menu: "切り取り (Ctrl+X)",
    copy_menu: "コピー (Ctrl+C)",
    paste_here_menu: "このフォルダーに貼り付け (Ctrl+V)",
    delete_menu: "🗑 削除 (Delete)",
    back_enter: "◀ 戻る (Enter)",
    prev_tip: "前へ (←)",
    next_tip: "次へ (→)",
    rotate_left_tip: "左に回転 (L)",
    rotate_right_tip: "右に回転 (R)",
    save_rotation: "💾 回転を保存 (Ctrl+S)",
    convert_title: "変換 / リサイズ",
    n_files: "{0} 個のファイル",
    includes_rotation: "（{0}° の回転を含む）",
    format: "形式",
    quality: "品質",
    lossless: "(ロスレス)",
    png_lossless: "(PNG はロスレスのため影響なし)",
    resize: "リサイズ",
    resize_kinds: [
        "リサイズしない",
        "パーセント",
        "長辺",
        "幅・高さに収める（比率維持）",
        "幅・高さを指定（比率無視）",
    ],
    width_prefix: "幅 ",
    height_prefix: "高さ ",
    no_upscale: "小さい画像は拡大しない",
    output: "出力",
    same_folder: "同じフォルダー",
    subfolder: "サブフォルダー",
    overwrite: "同名ファイルは上書き（しない場合は _1 を付加）",
    start: "開始",
    cancel: "キャンセル",
    keep_format: "元の形式",
    done_errors_n: "完了、エラー {0} 件：",
    close: "閉じる",
    settings_title: "設定",
    language: "言語 / Language",
    lang_system: "システムに従う",
    scroll_per_notch: "ホイール 1 目盛りで",
    thumb_rows: "行スクロール",
    smooth_scroll: "スムーズスクロール",
    confirm_delete: "削除前に確認",
    window_on_start: "起動時のウィンドウ",
    remember_window: "前回の状態を記憶（サイズ / 位置 / 最大化）",
    always_maximized: "常に最大化",
    after_close: "ウィンドウを閉じた後",
    stay_resident: "システムトレイに常駐（次回すばやく開ける）",
    idle_exit: "分間画像を開かなければ自動終了（0 = 終了しない）",
    sort_name: "名前",
    sort_modified: "更新日時",
    sort_created: "作成日時",
    sort_size: "サイズ",
    sort_type: "種類",
    nothing_selected: "何も選択されていません",
    clipboard_busy: "クリップボードは他のプログラムが使用中です。もう一度お試しください",
    clipboard_clear_failed: "クリップボードを空にできませんでした",
    clipboard_write_failed: "クリップボードに書き込めませんでした",
    clipboard_unsupported: "このプラットフォームではファイルのクリップボードへのコピーに未対応です",
    fileop_failed: "ファイル操作に失敗しました（エラーコード {0}）",
    paste_unsupported: "このプラットフォームではファイルの貼り付けに未対応です",
    delete_unsupported: "このプラットフォームではファイルの削除に未対応です",
    tray_open: "Hayabubusa を開く",
    tray_exit: "終了",
};

static KO: Strings = Strings {
    refreshed: "새로 고쳤습니다",
    copied_n: "항목 {0}개를 복사했습니다. 탐색기나 바탕 화면에 붙여 넣을 수 있습니다",
    cut_n: "항목 {0}개를 잘라냈습니다. 탐색기나 바탕 화면에 붙여 넣을 수 있습니다",
    fs_busy: "이전 파일 작업이 아직 진행 중입니다",
    clipboard_no_files: "클립보드에 파일이 없습니다",
    already_here: "파일이 이미 이 폴더에 있습니다",
    moved_n: "항목 {0}개를 이동했습니다",
    pasted_n: "항목 {0}개를 붙여 넣었습니다",
    cancelled: "취소했습니다",
    recycled_n: "항목 {0}개를 휴지통으로 이동했습니다",
    rotation_discarded: "회전이 저장되지 않았습니다(취소됨)",
    rotating_n: "파일 {0}개 회전",
    converting_n: "파일 {0}개 변환",
    batch_done: "{0} 완료",
    cannot_open: "열 수 없음: {0}",
    delete_title: "삭제",
    delete_confirm_n: "항목 {0}개를 휴지통으로 이동할까요?",
    and_more_n: "…외 {0}개",
    dont_ask_again: "다시 묻지 않기(⚙ 설정에서 되돌릴 수 있음)",
    delete_enter: "🗑 삭제 (Enter)",
    cancel_esc: "취소 (Esc)",
    up: "⬆ 상위 폴더",
    refresh_tip: "새로 고침 (F5)",
    rotate_left: "↺ 왼쪽으로 회전",
    rotate_right: "↻ 오른쪽으로 회전",
    convert_resize_menu: "변환 / 크기 조정…",
    paste: "📋 붙여넣기",
    delete: "🗑 삭제",
    delete_tip: "Delete (휴지통으로 이동)",
    thumbs: "썸네일",
    thumbs_tip: "Ctrl + 마우스 휠로도 조절할 수 있습니다",
    sort: "정렬",
    descending: "⬇ 내림차순",
    ascending: "⬆ 오름차순",
    sort_dir_tip: "오름차순 / 내림차순 전환",
    settings_btn: "⚙ 설정",
    status_counts: "이미지 {0}개, 폴더 {1}개",
    selected_n: "{0}개 선택됨",
    open: "열기",
    cut_menu: "잘라내기 (Ctrl+X)",
    copy_menu: "복사 (Ctrl+C)",
    paste_here_menu: "이 폴더에 붙여넣기 (Ctrl+V)",
    delete_menu: "🗑 삭제 (Delete)",
    back_enter: "◀ 돌아가기 (Enter)",
    prev_tip: "이전 (←)",
    next_tip: "다음 (→)",
    rotate_left_tip: "왼쪽으로 회전 (L)",
    rotate_right_tip: "오른쪽으로 회전 (R)",
    save_rotation: "💾 회전 저장 (Ctrl+S)",
    convert_title: "변환 / 크기 조정",
    n_files: "파일 {0}개",
    includes_rotation: "({0}° 회전 포함)",
    format: "형식",
    quality: "품질",
    lossless: "(무손실)",
    png_lossless: "(PNG는 무손실이라 영향 없음)",
    resize: "크기 조정",
    resize_kinds: [
        "크기 조정 안 함",
        "백분율",
        "긴 변",
        "너비·높이 안에 맞춤 (비율 유지)",
        "너비·높이 지정 (비율 무시)",
    ],
    width_prefix: "너비 ",
    height_prefix: "높이 ",
    no_upscale: "작은 이미지는 확대하지 않음",
    output: "출력",
    same_folder: "같은 폴더",
    subfolder: "하위 폴더",
    overwrite: "같은 이름의 파일 덮어쓰기(아니면 _1 추가)",
    start: "시작",
    cancel: "취소",
    keep_format: "원래 형식",
    done_errors_n: "완료, 오류 {0}개:",
    close: "닫기",
    settings_title: "설정",
    language: "언어 / Language",
    lang_system: "시스템 설정 따름",
    scroll_per_notch: "휠 한 칸에",
    thumb_rows: "줄 스크롤",
    smooth_scroll: "부드러운 스크롤",
    confirm_delete: "삭제 전 확인",
    window_on_start: "시작 시 창",
    remember_window: "마지막 상태 기억 (크기 / 위치 / 최대화)",
    always_maximized: "항상 최대화",
    after_close: "창을 닫은 후",
    stay_resident: "시스템 트레이에 상주 (다음에 더 빨리 열림)",
    idle_exit: "분 동안 이미지를 열지 않으면 자동 종료 (0 = 종료 안 함)",
    sort_name: "이름",
    sort_modified: "수정한 날짜",
    sort_created: "만든 날짜",
    sort_size: "크기",
    sort_type: "유형",
    nothing_selected: "선택된 항목이 없습니다",
    clipboard_busy: "다른 프로그램이 클립보드를 사용 중입니다. 다시 시도하세요",
    clipboard_clear_failed: "클립보드를 비울 수 없습니다",
    clipboard_write_failed: "클립보드에 쓸 수 없습니다",
    clipboard_unsupported: "이 플랫폼에서는 아직 파일을 클립보드에 복사할 수 없습니다",
    fileop_failed: "파일 작업 실패 (오류 코드 {0})",
    paste_unsupported: "이 플랫폼에서는 아직 파일 붙여넣기를 지원하지 않습니다",
    delete_unsupported: "이 플랫폼에서는 아직 파일 삭제를 지원하지 않습니다",
    tray_open: "Hayabubusa 열기",
    tray_exit: "종료",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_mapping() {
        assert_eq!(Lang::from_locale("zh-TW"), Lang::ZhTw);
        assert_eq!(Lang::from_locale("zh_HK.UTF-8"), Lang::ZhTw);
        assert_eq!(Lang::from_locale("ja-JP"), Lang::Ja);
        assert_eq!(Lang::from_locale("ko_KR"), Lang::Ko);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("fr-FR"), Lang::En);
    }

    #[test]
    fn fill_placeholders() {
        assert_eq!(fill("{0} / {1} / {0}", &[&3, &"x"]), "3 / x / 3");
    }

    #[test]
    fn placeholders_consistent() {
        // Every language must use the same placeholders as English.
        fn ph(s: &str) -> Vec<&str> {
            let mut v: Vec<&str> = ["{0}", "{1}"]
                .into_iter()
                .filter(|p| s.contains(p))
                .collect();
            v.sort();
            v
        }
        for l in Lang::ALL {
            for (x, y) in l
                .strings()
                .with_placeholders()
                .iter()
                .zip(EN.with_placeholders())
            {
                assert_eq!(ph(x), ph(y), "{l:?}: {x:?} vs {y:?}");
            }
        }
    }

    impl Strings {
        fn with_placeholders(&self) -> Vec<&'static str> {
            vec![
                self.copied_n,
                self.cut_n,
                self.moved_n,
                self.pasted_n,
                self.recycled_n,
                self.rotating_n,
                self.converting_n,
                self.batch_done,
                self.cannot_open,
                self.delete_confirm_n,
                self.and_more_n,
                self.status_counts,
                self.selected_n,
                self.n_files,
                self.includes_rotation,
                self.done_errors_n,
                self.fileop_failed,
            ]
        }
    }
}
