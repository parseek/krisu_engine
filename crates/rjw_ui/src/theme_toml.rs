//! **主题序列化 / 反序列化（TOML）** —— [`Theme::to_toml`] / [`Theme::from_toml`] /
//! [`Theme::apply_toml`]。
//!
//! ```no_run
//! # use rjw_ui::Theme;
//! # fn f(theme: &Theme) -> Result<(), String> {
//! // 导出：完整主题（含格式版本头）
//! let text = theme.to_toml()?;
//! std::fs::write("my-theme.toml", &text).map_err(|e| e.to_string())?;
//! // 导入：**合并到当前主题**（文件里出现的字段才覆盖）——手写小文件也能用
//! let mut t = theme.clone();
//! t.apply_toml("gap = 12.0\n[panel]\nradius = 3.0")?;
//! // 或者从零加载（缺字段回落 `Default`）
//! let t2 = Theme::from_toml(&text)?;
//! # Ok(()) }
//! ```
//!
//! # 格式
//!
//! ```toml
//! format_version = 1            # 见 `THEME_FORMAT_VERSION`（不认识 ⇒ 加载报错）
//!
//! [theme]                       # `[theme]` 之下就是 `Theme` 的字段树
//! row_h = 20.0
//! gap = 6.0
//! font_weight = 700             # u16（`Weight` 在 rjw_text，用数值代理）
//!
//! [theme.panel]
//! padding = 6.0
//! radius = { tl = 6.0, tr = 6.0, br = 6.0, bl = 6.0 }
//! shadow = { blur = 8.0, offset = { x = 0.0, y = 2.0 }, color = { ... } }
//! ```
//!
//! # 语义（三条）
//!
//! 1. **`to_toml` 是全量导出**：所有样式字段都在（`PanelStyle::bg_image` 除外 ——
//!    纹理 `uid` 不可移植，见那里的 `serde(skip)`）；导入回来逐字段一致（有单测守着
//!    "再导出一次文本完全相同"）。
//! 2. **`apply_toml` 是**合并覆盖**：先把当前主题序列化成 TOML 值，再把文件里的值**递归**
//!    合并上去 ⇒ 只在文件里写 `gap = 12` 就只改间距、其余保持当前值（手写小文件最常用的
//!    用法）。`from_toml` = 在 `Theme::default()` 上 `apply_toml`。
//! 3. **认不出 / 缺字段都不致命**：缺字段回落 `Default`（每个样式结构体都 `serde(default)`）；
//!    多余的键被忽略（向前兼容）；只有 **`format_version` 比本引擎新**、或 TOML 语法 /
//!    字段类型不合法才返回 `Err(String)`（消息带 `to_string` 的原始错误，直接给用户看）。
use serde::{Deserialize, Serialize};

use crate::Theme;

/// **主题序列化格式版本**（写进导出文件的 `format_version`）。
///
/// 加载时要求 `<= THEME_FORMAT_VERSION`：比本引擎**新**的文件拒绝加载（消息里说明），
/// 免得"新字段被静默丢弃"。向后兼容的字段新增**不需要**改这个值（缺字段会回落默认）。
pub const THEME_FORMAT_VERSION: u32 = 1;

/// 导出文件的外层：版本头 + `[theme]` 表。
///
/// 单独一个外壳（而不是把版本号塞进 [`Theme`]）的原因：`Theme` 是给控件用的纯样式，
/// 多一个"文件格式"字段会污染它；同时 `format_version` **故意不给默认值** ——
/// 于是"裸主题表"（只有 `gap = 12` 这种）会解析外壳失败，正常走到"直接当主题表"那条路。
#[derive(Serialize, Deserialize)]
struct ThemeFile {
    format_version: u32,
    theme: Theme,
}

/// 递归合并：`over` 覆盖 `base`（表 → 逐键递归；其余 → 整体替换）。
fn merge(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

impl Theme {
    /// **导出为 TOML 文本**（全量：所有样式字段 + `format_version` 头）。
    ///
    /// 失败只可能是"某个字段不能表示成 TOML"（本主题里不会有：都是数字 / 字符串 /
    /// 表 / 数组）——仍然返回 `Result` 以免将来加字段时悄悄改变 API。
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(&ThemeFile {
            format_version: THEME_FORMAT_VERSION,
            theme: self.clone(),
        })
        .map_err(|e| format!("主题序列化失败：{e}"))
    }

    /// **从 TOML 文本加载**（缺字段回落 [`Theme::default`]）。
    ///
    /// 想要"在当前主题上只改几个字段"请用 [`Theme::apply_toml`]。
    pub fn from_toml(s: &str) -> Result<Theme, String> {
        let mut t = Theme::default();
        t.apply_toml(s)?;
        Ok(t)
    }

    /// **在**当前**主题上合并** TOML 文本（文件里出现的字段才覆盖）。
    ///
    /// - 带版本头的导出文件：校验 `format_version` 后取 `[theme]` 子树；
    /// - **裸主题表**（没有 `format_version`）：整份文档就是 `Theme` 的字段树 ——
    ///   于是 `gap = 12` 这样两行的手写文件也直接能用。
    pub fn apply_toml(&mut self, s: &str) -> Result<(), String> {
        let doc: toml::Value =
            toml::from_str(s).map_err(|e| format!("主题 TOML 解析失败：{e}"))?;
        let over = if let Some(v) = doc.get("format_version") {
            let ver = v.as_integer().unwrap_or(-1);
            if ver != THEME_FORMAT_VERSION as i64 {
                return Err(format!(
                    "主题格式版本 {ver} 不受支持（本引擎支持 {THEME_FORMAT_VERSION}）"
                ));
            }
            doc.get("theme")
                .cloned()
                .ok_or_else(|| "主题文件有 format_version 但缺少 [theme] 表".to_owned())?
        } else {
            doc
        };
        // 当前主题 → TOML 值 → 递归合并 → 反序列化回来（这就是"合并覆盖"的实现）。
        let mut base =
            toml::Value::try_from(&*self).map_err(|e| format!("当前主题无法序列化：{e}"))?;
        merge(&mut base, over);
        *self = base
            .try_into()
            .map_err(|e| format!("主题字段不合法：{e}"))?;
        Ok(())
    }
}

#[cfg(feature = "serde")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{GripShape, Palette};
    use rjw_text::Weight;

    /// 造一个"每个角落都改过"的主题：字段改动越分散，round-trip 越能发现问题。
    fn touched() -> Theme {
        let mut t = Theme::themed(&Palette::dark());
        t.gap = 12.5;
        t.row_h = 33.0;
        t.feather = 2.5;
        t.line_spacing = 1.4;
        t.font_weight = Weight::BOLD;
        t.panel.radius = crate::draw::CornerRadius::all(9.0);
        t.panel.grip.shape = GripShape::Diagonal;
        t.button.font_family = Some(std::sync::Arc::from("SomeFont"));
        t.label.align = rjw_text::Align::Right;
        t
    }

    #[test]
    fn round_trip_is_field_exact() {
        let t = touched();
        let text = t.to_toml().expect("序列化");
        // 人类可读：版本头 + `[theme]` 表 + 数值字段。
        assert!(text.contains("format_version = 1"), "{text}");
        assert!(text.contains("[theme]"), "{text}");
        assert!(text.contains("font_weight = 700"), "{text}");
        assert!(text.contains("[theme.panel]"), "{text}");
        let back = Theme::from_toml(&text).expect("反序列化");
        // 逐字段一致的**最强**断言：再导出一次文本完全相同。
        assert_eq!(back.to_toml().unwrap(), text, "round-trip 后导出文本必须逐字相同");
        assert_eq!(back.gap, 12.5);
        assert_eq!(back.font_weight, Weight::BOLD);
        assert_eq!(back.label.align, rjw_text::Align::Right);
        assert_eq!(back.button.font_family.as_deref(), Some("SomeFont"));
    }

    #[test]
    fn apply_merges_onto_the_current_theme() {
        // **合并覆盖**：只写到的字段变，其余保持"当前主题"的值（手写小文件的用法）。
        let mut t = touched();
        let before_gap = t.gap;
        let before_row = t.row_h;
        t.apply_toml("gap = 3.0\nrow_h = 21.0\n[panel]\nradius = 2.0")
            .expect("合并");
        assert_eq!(t.gap, 3.0);
        assert_eq!(t.row_h, 21.0);
        assert_eq!(t.panel.radius.tl, 2.0);
        assert_ne!(before_gap, t.gap, "文件里写了的字段必须被覆盖");
        assert_ne!(before_row, t.row_h);
        // 文件没提的字段保持原样（不是回落 Default）——这正是 apply 与 from 的区别。
        assert_eq!(t.feather, 2.5);
        assert_eq!(t.font_weight, Weight::BOLD);
        assert_eq!(t.panel.grip.shape, GripShape::Diagonal);
        // `from_toml` 则是在 `Default` 上合并 ⇒ 没写的字段是默认值。
        let fresh = Theme::from_toml("gap = 3.0").expect("加载");
        assert_eq!(fresh.gap, 3.0);
        assert_ne!(fresh.feather, 2.5, "from_toml 从 Default 起，不继承当前主题");
    }

    #[test]
    fn newer_format_version_is_rejected_and_bad_toml_is_readable() {
        let e = Theme::from_toml("format_version = 99\n[theme]\ngap = 1.0").unwrap_err();
        assert!(e.contains("99") && e.contains("不受支持"), "消息要点明版本：{e}");
        let e = Theme::from_toml("gap = ").unwrap_err();
        assert!(e.starts_with("主题 TOML 解析失败"), "解析错误要带原文：{e}");
        // 有版本头但缺 `[theme]` 表 ⇒ 明确报错（而不是静默加载一个默认主题）。
        let e = Theme::from_toml("format_version = 1").unwrap_err();
        assert!(e.contains("[theme]"), "{e}");
    }

    #[test]
    fn background_image_is_not_serialized() {
        // 纹理 uid 跨进程不可移植 ⇒ `bg_image` 不序列化（加载后回落 None）。
        let mut t = touched();
        t.panel.bg_image = Some(
            crate::draw::ImageBg::new(42, glam::Vec2::new(64.0, 64.0)),
        );
        let text = t.to_toml().unwrap();
        assert!(!text.contains("bg_image"), "背景图不该出现在主题文件里：{text}");
        let back = Theme::from_toml(&text).unwrap();
        assert!(back.panel.bg_image.is_none());
    }
}
