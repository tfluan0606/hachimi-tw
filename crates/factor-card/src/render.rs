//! 卡片繪圖：版面 1:1 對應網站的 stallion 展開卡（`static/stallion.css`）。
//!
//! 全部尺寸都以 CSS px 計算，最後乘上 `scale` 輸出，所以要出 2x 高解析只要傳 `scale = 2.0`。

use crate::data::{Badge, CardData, FbClass, Stat, StatValue, Tone};
use crate::text::Fonts;
use std::collections::HashMap;
use tiny_skia::{
    Color, FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

// ── 調色盤（stallion.css 的亮色 / [data-theme="night"] 暗色）──

/// 卡片主題
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    Light,
    #[default]
    Dark,
}

type Rgba = (u8, u8, u8, f32);

#[derive(Clone, Copy)]
pub struct Palette {
    card_bg: [u8; 3],
    block_bg: [u8; 3],
    ink: [u8; 3],
    ink_soft: [u8; 3],
    ink_mute: [u8; 3],
    line: Rgba,
    amber_ink: [u8; 3],
    res_bg: Rgba,
    res_border: Rgba,
    tone_blue: [u8; 3],
    tone_red: [u8; 3],
    tone_green: [u8; 3],
    dark: bool,
}

impl Theme {
    pub fn palette(self) -> Palette {
        match self {
            Theme::Light => Palette {
                card_bg: [0xfd, 0xfc, 0xf7],
                block_bg: [0xff, 0xff, 0xff],
                ink: [0x2c, 0x4a, 0x5e],
                ink_soft: [0x5a, 0x79, 0x90],
                ink_mute: [0x8a, 0xa5, 0xb8],
                line: (60, 90, 120, 0.14),
                amber_ink: [0x9a, 0x70, 0x00],
                res_bg: (240, 184, 64, 0.09),
                res_border: (240, 184, 64, 0.30),
                tone_blue: [0x1b, 0x4f, 0x7d],
                tone_red: [0xa0, 0x48, 0x48],
                tone_green: [0x1a, 0x70, 0x50],
                dark: false,
            },
            Theme::Dark => Palette {
                card_bg: [0x22, 0x30, 0x4d],
                block_bg: [0x1a, 0x26, 0x3e],
                ink: [0xe8, 0xed, 0xf5],
                ink_soft: [0xb8, 0xc4, 0xd8],
                ink_mute: [0x8a, 0x98, 0xb0],
                line: (200, 220, 255, 0.18),
                amber_ink: [0xf0, 0xc0, 0x40],
                res_bg: (240, 184, 64, 0.14),
                res_border: (240, 184, 64, 0.40),
                tone_blue: [0x7e, 0xb8, 0xf7],
                tone_red: [0xf7, 0x7e, 0x7e],
                tone_green: [0x7e, 0xf0, 0xa0],
                dark: true,
            },
        }
    }
}

impl Palette {
    fn tone(&self, tone: Tone) -> [u8; 3] {
        match tone {
            Tone::Default => self.ink,
            Tone::Blue => self.tone_blue,
            Tone::Red => self.tone_red,
            Tone::Green => self.tone_green,
        }
    }
}

// ── 版面尺寸 ────────────────────────────────────────────────
const CARD_W: f32 = 1040.0;
const PAGE_MARGIN: f32 = 0.0;
const PAD_X: f32 = 30.0;
const PAD_Y: f32 = 26.0;
const SECTION_GAP: f32 = 14.0;

const STAT_GAP: f32 = 8.0;
const STAT_H: f32 = 56.0;
const STAT_LINE_H: f32 = 22.0;
/// 縱向統計格高度：內距 + 標題 + 三行
const STAT_TALL_H: f32 = 12.0 + 17.0 + 8.0 + STAT_LINE_H * 3.0 + 10.0;
const SIDE_LABELS: [&str; 3] = ["本體", "父", "母"];
const COL_GAP: f32 = 12.0;
const BLOCK_PAD_X: f32 = 14.0;
const BLOCK_PAD_Y: f32 = 12.0;
const PORTRAIT: f32 = 56.0;

const BADGE_H: f32 = 27.0;
const BADGE_PAD_X: f32 = 10.0;
const BADGE_GAP: f32 = 8.0; // badge 之間
const BADGE_ROW_GAP: f32 = 8.0;
const FS_BADGE: f32 = 14.0;
const FS_STAR: f32 = 12.5;

pub struct Portrait {
    pub width: u32,
    pub height: u32,
    /// 非預乘 RGBA
    pub rgba: Vec<u8>,
}

fn rgb(c: [u8; 3]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], 255)
}

fn rgba(c: (u8, u8, u8, f32)) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, (c.3 * 255.0).round() as u8)
}

fn badge_colors(t: &Palette, class: FbClass) -> (Rgba, [u8; 3], Option<Rgba>) {
    if t.dark {
        return match class {
            FbClass::Blue => ((74, 143, 200, 0.24), [0x7e, 0xb8, 0xf7], Some((74, 143, 200, 0.50))),
            FbClass::Red => ((212, 120, 120, 0.22), [0xf7, 0x7e, 0x7e], Some((212, 120, 120, 0.50))),
            FbClass::White => ((184, 168, 120, 0.22), [0xe6, 0xd4, 0xa0], Some((184, 168, 120, 0.45))),
            FbClass::WhiteOut => ((120, 130, 150, 0.18), t.ink_soft, Some((120, 130, 150, 0.35))),
            FbClass::Green => ((124, 196, 164, 0.20), [0x7e, 0xf0, 0xa0], Some((124, 196, 164, 0.50))),
            FbClass::T0 => ((240, 184, 64, 0.20), [0xf0, 0xc0, 0x40], Some((240, 184, 64, 0.50))),
            FbClass::T1 => ((100, 200, 200, 0.20), [0xa0, 0xe0, 0xe0], Some((100, 200, 200, 0.50))),
        };
    }
    match class {
        FbClass::Blue => ((74, 143, 200, 0.22), [0x1b, 0x4f, 0x7d], Some((74, 143, 200, 0.45))),
        FbClass::Red => ((212, 120, 120, 0.18), [0x93, 0x3d, 0x3d], Some((212, 120, 120, 0.45))),
        FbClass::White => ((184, 168, 120, 0.18), [0x70, 0x5e, 0x38], Some((184, 168, 120, 0.5))),
        FbClass::WhiteOut => ((120, 120, 120, 0.10), t.ink_soft, Some((120, 120, 120, 0.35))),
        FbClass::Green => ((124, 196, 164, 0.15), [0x1a, 0x70, 0x50], Some((124, 196, 164, 0.4))),
        FbClass::T0 => ((240, 184, 64, 0.15), t.amber_ink, Some((240, 184, 64, 0.4))),
        FbClass::T1 => ((100, 200, 200, 0.15), [0x1a, 0x80, 0x80], Some((100, 200, 200, 0.4))),
    }
}

fn stars_text(n: u32) -> String {
    "★".repeat(n as usize)
}

/// 畫布：把 CSS px 換算成實際像素，並提供圓角矩形／文字等基本操作
struct Canvas<'a> {
    pixmap: Pixmap,
    fonts: &'a Fonts,
    scale: f32,
    t: Palette,
}

impl<'a> Canvas<'a> {
    fn rounded_path(&self, x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
        let s = self.scale;
        let (x, y, w, h, r) = (x * s, y * s, w * s, h * s, r * s);
        let r = r.min(w / 2.0).min(h / 2.0);
        let k = r * 0.5523;
        let mut pb = PathBuilder::new();
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
        pb.line_to(x, y + r);
        pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
        pb.close();
        pb.finish()
    }

    fn fill_round(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color) {
        let Some(path) = self.rounded_path(x, y, w, h, r) else { return };
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    fn stroke_round(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color, width: f32) {
        let Some(path) = self.rounded_path(x, y, w, h, r) else { return };
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke { width: width * self.scale, ..Default::default() };
        self.pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    /// 實心矩形（不做 AA，避免 1px 線在 tiny-skia hairline 路徑上出問題）
    fn fill_rect_px(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let s = self.scale;
        let Some(rect) = Rect::from_xywh(x * s, y * s, (w * s).max(1.0), (h * s).max(1.0)) else { return };
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = false;
        self.pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }

    fn box_(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, bg: Color, border: Option<Color>) {
        self.fill_round(x, y, w, h, r, bg);
        if let Some(b) = border {
            self.stroke_round(x, y, w, h, r, b, 1.0);
        }
    }

    /// 以 `baseline` 為基線畫字（CSS px 座標）
    /// 回傳結束的 x（CSS px）
    fn text(&mut self, s: &str, size: f32, x: f32, baseline: f32, color: [u8; 3], bold: bool) -> f32 {
        self.fonts.draw(
            &mut self.pixmap,
            s,
            size * self.scale,
            x * self.scale,
            baseline * self.scale,
            color,
            bold,
        ) / self.scale
    }

    fn measure(&self, s: &str, size: f32, bold: bool) -> f32 {
        self.fonts.measure(s, size * self.scale, bold) / self.scale
    }

    /// 在 (x,y,size) 的圓角框裡畫一張圖（等比 cover）
    fn image(&mut self, p: &Portrait, x: f32, y: f32, size: f32, radius: f32) {
        if p.width == 0 || p.height == 0 {
            return;
        }
        let Some(src) = Pixmap::from_vec(
            premultiply(&p.rgba),
            tiny_skia::IntSize::from_wh(p.width, p.height).unwrap(),
        ) else {
            return;
        };
        let s = self.scale;
        let target = size * s;
        let k = (target / p.width as f32).max(target / p.height as f32);
        let dx = x * s + (target - p.width as f32 * k) / 2.0;
        let dy = y * s + (target - p.height as f32 * k) / 2.0;

        let mask = self.rounded_path(x, y, size, size, radius).map(|path| {
            let mut m = Mask::new(self.pixmap.width(), self.pixmap.height()).unwrap();
            m.fill_path(&path, FillRule::Winding, true, Transform::identity());
            m
        });
        self.pixmap.draw_pixmap(
            0,
            0,
            src.as_ref(),
            &PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() },
            Transform::from_row(k, 0.0, 0.0, k, dx, dy),
            mask.as_ref(),
        );
    }
}

impl<'a> Canvas<'a> {
    /// 置中的半透明浮水印（寬度＝畫布寬 * `ratio`）
    fn watermark(&mut self, p: &Portrait, ratio: f32, opacity: f32) {
        // 暗色主題把圖提亮，不然疊在深底上會糊成一團
        let brighten = if self.t.dark { 0.55 } else { 0.0 };
        if p.width == 0 || p.height == 0 {
            return;
        }
        // LOGO 是白底加黑框：先裁掉外框，再把接近白色的底去掉，只留角色本身
        let margin = (p.width.min(p.height) as f32 * 0.04) as u32;
        let (cw, ch) = (p.width.saturating_sub(margin * 2), p.height.saturating_sub(margin * 2));
        if cw == 0 || ch == 0 {
            return;
        }
        // 只去掉「從邊緣連通的白底」，角色臉上的白、頭髮反光要留著
        let idx = |x: u32, y: u32| ((y * cw + x) as usize) * 4;
        let src_at = |x: u32, y: u32| (((y + margin) * p.width + x + margin) as usize) * 4;
        let is_bg = |i: usize| {
            p.rgba[i].min(p.rgba[i + 1]).min(p.rgba[i + 2]) > 225 && p.rgba[i + 3] > 0
        };

        let mut transparent = vec![false; (cw * ch) as usize];
        let mut queue: Vec<(u32, u32)> = Vec::new();
        let mut push = |queue: &mut Vec<(u32, u32)>, transparent: &mut Vec<bool>, x: u32, y: u32| {
            let flat = (y * cw + x) as usize;
            if !transparent[flat] && is_bg(src_at(x, y)) {
                transparent[flat] = true;
                queue.push((x, y));
            }
        };
        for x in 0..cw {
            push(&mut queue, &mut transparent, x, 0);
            push(&mut queue, &mut transparent, x, ch - 1);
        }
        for y in 0..ch {
            push(&mut queue, &mut transparent, 0, y);
            push(&mut queue, &mut transparent, cw - 1, y);
        }
        while let Some((x, y)) = queue.pop() {
            if x > 0 {
                push(&mut queue, &mut transparent, x - 1, y);
            }
            if x + 1 < cw {
                push(&mut queue, &mut transparent, x + 1, y);
            }
            if y > 0 {
                push(&mut queue, &mut transparent, x, y - 1);
            }
            if y + 1 < ch {
                push(&mut queue, &mut transparent, x, y + 1);
            }
        }

        // 上方「ASKR牛逼！」字樣：筆畫圍起來的白色不與邊緣相連，洪水填充碰不到，
        // 疊在暗底上會變成亮斑，這裡把文字帶內剩下的白色一併去掉
        let text_band = (ch as f32 * 0.24) as u32;
        for y in 0..text_band {
            for x in 0..cw {
                if is_bg(src_at(x, y)) {
                    transparent[(y * cw + x) as usize] = true;
                }
            }
        }

        let mut rgba = vec![0u8; (cw * ch * 4) as usize];
        // 暗底上原圖太沉，整體往白色提亮
        let lift = |c: u8| (c as f32 + (255.0 - c as f32) * brighten) as u8;
        for y in 0..ch {
            for x in 0..cw {
                let (d, sp) = (idx(x, y), src_at(x, y));
                if transparent[(y * cw + x) as usize] {
                    continue;
                }
                rgba[d] = lift(p.rgba[sp]);
                rgba[d + 1] = lift(p.rgba[sp + 1]);
                rgba[d + 2] = lift(p.rgba[sp + 2]);
                rgba[d + 3] = p.rgba[sp + 3];
            }
        }
        let Some(src) =
            Pixmap::from_vec(premultiply(&rgba), tiny_skia::IntSize::from_wh(cw, ch).unwrap())
        else {
            return;
        };
        let (pw, ph) = (cw as f32, ch as f32);
        let target = self.pixmap.width() as f32 * ratio;
        let k = target / pw;
        let dx = (self.pixmap.width() as f32 - pw * k) / 2.0;
        let dy = (self.pixmap.height() as f32 - ph * k) / 2.0;
        self.pixmap.draw_pixmap(
            0,
            0,
            src.as_ref(),
            &PixmapPaint { opacity, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() },
            Transform::from_row(k, 0.0, 0.0, k, dx, dy),
            None,
        );
    }
}

fn premultiply(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u32;
            [
                (p[0] as u32 * a / 255) as u8,
                (p[1] as u32 * a / 255) as u8,
                (p[2] as u32 * a / 255) as u8,
                p[3],
            ]
        })
        .collect()
}

// ── badge 排版 ─────────────────────────────────────────────

struct BadgeLayout<'a> {
    badge: &'a Badge,
    x: f32,
    w: f32,
    text_w: f32,
    stars: String,
}

/// 把 badge 依寬度自動換行，回傳每一列
fn wrap_badges<'a>(canvas: &Canvas, badges: &'a [Badge], max_w: f32) -> Vec<Vec<BadgeLayout<'a>>> {
    let mut rows: Vec<Vec<BadgeLayout>> = Vec::new();
    let mut row: Vec<BadgeLayout> = Vec::new();
    let mut x = 0.0f32;
    for b in badges {
        let stars = stars_text(b.stars);
        let text_w = canvas.measure(&b.name, FS_BADGE, false);
        let star_w = if stars.is_empty() { 0.0 } else { canvas.measure(&stars, FS_STAR, false) + 4.0 };
        let w = BADGE_PAD_X * 2.0 + text_w + star_w;
        if !row.is_empty() && x + w > max_w {
            rows.push(std::mem::take(&mut row));
            x = 0.0;
        }
        row.push(BadgeLayout { badge: b, x, w, text_w, stars });
        x += w + BADGE_GAP;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn badge_rows_height(rows: usize) -> f32 {
    if rows == 0 {
        0.0
    } else {
        rows as f32 * BADGE_H + (rows - 1) as f32 * BADGE_ROW_GAP
    }
}

fn draw_badges(canvas: &mut Canvas, rows: &[Vec<BadgeLayout>], x0: f32, y0: f32) {
    let t = canvas.t;
    for (ri, row) in rows.iter().enumerate() {
        let y = y0 + ri as f32 * (BADGE_H + BADGE_ROW_GAP);
        for b in row {
            let (bg, fg, border) = badge_colors(&t, b.badge.class);
            canvas.box_(x0 + b.x, y, b.w, BADGE_H, 6.0, rgba(bg), border.map(rgba));
            // 文字垂直置中（以視覺基線微調）
            let baseline = y + BADGE_H / 2.0 + FS_BADGE * 0.36;
            canvas.text(&b.badge.name, FS_BADGE, x0 + b.x + BADGE_PAD_X, baseline, fg, false);
            if !b.stars.is_empty() {
                canvas.text(
                    &b.stars,
                    FS_STAR,
                    x0 + b.x + BADGE_PAD_X + b.text_w + 4.0,
                    y + BADGE_H / 2.0 + FS_STAR * 0.36,
                    fg,
                    false,
                );
            }
        }
    }
}

// ── 主流程 ─────────────────────────────────────────────────

/// 畫出卡片。`portraits` 以 card_id 為 key，缺圖就畫佔位色塊。
/// 浮水印大小（畫布寬度比例）與不透明度
const WATERMARK_RATIO: f32 = 0.34;
const WATERMARK_OPACITY: f32 = 0.10;

pub fn render(
    data: &CardData,
    portraits: &HashMap<i64, Portrait>,
    fonts: &Fonts,
    scale: f32,
    theme: Theme,
    watermark: Option<&Portrait>,
) -> Pixmap {
    let t = theme.palette();
    let content_w = CARD_W - PAD_X * 2.0;
    let col_w = (content_w - COL_GAP * 2.0) / 3.0;
    let block_content_w = col_w - BLOCK_PAD_X * 2.0;

    // ── 先量測，算出總高度 ──
    let probe = Canvas { pixmap: Pixmap::new(1, 1).unwrap(), fonts, scale, t };

    let block_rows: Vec<Vec<Vec<BadgeLayout>>> = data
        .blocks
        .iter()
        .map(|b| wrap_badges(&probe, &b.badges, block_content_w))
        .collect();
    // tag(14) + 6 + 立繪列(48) + 8 + badges
    let block_body_h = |rows: &Vec<Vec<BadgeLayout>>| -> f32 {
        BLOCK_PAD_Y * 2.0 + 24.0 + 6.0 + PORTRAIT + 8.0 + badge_rows_height(rows.len())
    };
    let blocks_h = block_rows.iter().map(block_body_h).fold(0.0f32, f32::max);

    let res_chips: Vec<(String, String, FbClass, f32, f32)> = data
        .resonance
        .iter()
        .map(|r| {
            let name = r.name.clone();
            let stars = format!("★{}/{}/{}", r.self_stars, r.father_stars, r.mother_stars);
            let nw = probe.measure(&name, FS_BADGE, false);
            let sw = probe.measure(&stars, FS_STAR, true);
            (name, stars, r.class, nw, sw)
        })
        .collect();
    // 這一列一定會出現（至少有遺傳子分布那格）
    let res_h = 12.0 * 2.0 + 24.0 + 8.0 + BADGE_H;
    let row_h = res_h;

    let header_h = 28.0;
    let card_h = PAD_Y * 2.0
        + header_h
        + SECTION_GAP
        + STAT_TALL_H
        + SECTION_GAP
        + if res_chips.is_empty() { 0.0 } else { row_h + SECTION_GAP }
        + blocks_h;

    let img_w = ((CARD_W + PAGE_MARGIN * 2.0) * scale).ceil() as u32;
    let img_h = ((card_h + PAGE_MARGIN * 2.0) * scale).ceil() as u32;
    let mut pixmap = Pixmap::new(img_w, img_h).unwrap();
    pixmap.fill(rgb(t.card_bg));
    let mut canvas = Canvas { pixmap, fonts, scale, t };

    // ── 卡片底 ──
    let (cx, cy) = (PAGE_MARGIN, PAGE_MARGIN);
    canvas.box_(cx, cy, CARD_W, card_h, 0.0, rgb(t.card_bg), None);

    let x0 = cx + PAD_X;
    let mut y = cy + PAD_Y;

    // ── 標題列 ──
    let name_size = 19.0;
    let baseline = y + 20.0;
    let end = canvas.text(&data.name, name_size, x0, baseline, t.ink, true);
    let mut hx = end + 10.0;
    if data.viewer_id != 0 {
        hx = canvas.text(&format!("UID {}", data.viewer_id), 14.0, hx, baseline, t.ink_soft, true);
    }
    if let Some(n) = data.follower_num {
        canvas.text(&format!("追蹤 {n}"), 14.0, hx + 14.0, baseline, t.ink_soft, true);
    }
    y += header_h + SECTION_GAP;

    // ── 統計列（每格縱向列出 本體／父／母）──
    {
        // 等寬；真的塞不下時才讓超寬的那格borrow旁邊的空間
        let n = data.stats.len() as f32;
        let even_w = (content_w - STAT_GAP * (n - 1.0)) / n;
        let mut widths: Vec<f32> = data
            .stats
            .iter()
            .map(|s| stat_cell_width(&canvas, s).max(even_w))
            .collect();
        let need: f32 = widths.iter().sum::<f32>() + STAT_GAP * (n - 1.0);
        if need > content_w {
            let shrink = (need - content_w) / n;
            for w in widths.iter_mut() {
                *w -= shrink;
            }
        }
        let mut sx = x0;
        for (s, w) in data.stats.iter().zip(widths.iter()) {
            draw_stat_cell(&mut canvas, s, sx, y, *w);
            sx += w + STAT_GAP;
        }
        y += STAT_TALL_H + SECTION_GAP;
    }

    // ── 三方共鳴（整列）──
    if !res_chips.is_empty() {
        canvas.box_(
            x0,
            y,
            content_w,
            row_h,
            10.0,
            rgba(t.res_bg),
            Some(rgba(t.res_border)),
        );
        canvas.text("三方共鳴白因", 17.0, x0 + 14.0, y + 12.0 + 17.0, t.amber_ink, true);
        let mut chip_x = x0 + 14.0;
        let chip_y = y + 12.0 + 24.0 + 8.0;
        for (name, stars, class, nw, sw) in &res_chips {
            let w = BADGE_PAD_X * 2.0 + nw + 10.0 + sw;
            let (bg, fg, border) = badge_colors(&t, *class);
            canvas.box_(chip_x, chip_y, w, BADGE_H, 6.0, rgba(bg), border.map(rgba));
            canvas.text(name, FS_BADGE, chip_x + BADGE_PAD_X,
                        chip_y + BADGE_H / 2.0 + FS_BADGE * 0.36, fg, false);
            let sep_x = chip_x + BADGE_PAD_X + nw + 5.0;
            canvas.fill_rect_px(sep_x, chip_y + 4.0, 1.0, BADGE_H - 8.0,
                                Color::from_rgba8(fg[0], fg[1], fg[2], 110));
            canvas.text(stars, FS_STAR, sep_x + 5.0,
                        chip_y + BADGE_H / 2.0 + FS_STAR * 0.36, fg, true);
            chip_x += w + 6.0;
        }
        y += row_h + SECTION_GAP;
    }

    // ── 本體 / 父 / 母 ──
    for (i, (block, rows)) in data.blocks.iter().zip(block_rows.iter()).enumerate() {
        let bx = x0 + i as f32 * (col_w + COL_GAP);
        canvas.box_(bx, y, col_w, blocks_h, 10.0, rgb(t.block_bg), Some(rgba(t.line)));
        let ix = bx + BLOCK_PAD_X;
        let mut iy = y + BLOCK_PAD_Y;
        canvas.text(block.label, 17.0, ix, iy + 17.0, t.ink, true);
        iy += 24.0 + 6.0;

        match (&block.name, block.card_id) {
            (Some(name), Some(cid)) => {
                match portraits.get(&cid) {
                    Some(p) => canvas.image(p, ix, iy, PORTRAIT, 10.0),
                    None => canvas.box_(ix, iy, PORTRAIT, PORTRAIT, 10.0, rgba(t.line), None),
                }
                canvas.text(name, 15.0, ix + PORTRAIT + 10.0, iy + PORTRAIT / 2.0 + 5.5, t.ink, true);
            }
            _ => {
                canvas.text("無資料", 14.0, ix, iy + 16.0, t.ink_mute, false);
            }
        }
        iy += PORTRAIT + 8.0;
        draw_badges(&mut canvas, rows, ix, iy);
    }

    if let Some(w) = watermark {
        canvas.watermark(w, WATERMARK_RATIO, WATERMARK_OPACITY);
    }

    canvas.pixmap
}


/// 縱向統計格所需寬度
fn stat_cell_width(canvas: &Canvas, stat: &Stat) -> f32 {
    let label_w = canvas.measure(stat.key, 14.0, true);
    match &stat.value {
        StatValue::Single(v) => label_w.max(canvas.measure(v, 17.0, true)) + 28.0,
        StatValue::PerSide(vals) => {
            let side_w =
                SIDE_LABELS.iter().map(|s| canvas.measure(s, 14.0, true)).fold(0.0f32, f32::max);
            let value_w = vals.iter().map(|segs| segs_width(canvas, segs)).fold(0.0f32, f32::max);
            label_w.max(side_w + 10.0 + value_w) + 28.0
        }
    }
}

/// 只畫統計格的文字（不畫外框）
fn draw_stat_lines(canvas: &mut Canvas, stat: &Stat, x: f32, y: f32) {
    let t = canvas.t;
    canvas.text(stat.key, 14.0, x + 14.0, y + 12.0 + 17.0, t.ink_soft, true);
    let color = t.tone(stat.tone);
    match &stat.value {
        StatValue::Single(v) => {
            canvas.text(v, 17.0, x + 14.0, y + 12.0 + 17.0 + 8.0 + 16.0, color, true);
        }
        StatValue::PerSide(vals) => {
            let side_w =
                SIDE_LABELS.iter().map(|s| canvas.measure(s, 14.0, true)).fold(0.0f32, f32::max);
            for (i, (side, segs)) in SIDE_LABELS.iter().zip(vals.iter()).enumerate() {
                let baseline = y + 12.0 + 17.0 + 8.0 + STAT_LINE_H * i as f32 + 16.0;
                canvas.text(side, 14.0, x + 14.0, baseline, t.ink_soft, true);
                let mut vx = x + 14.0 + side_w + 10.0;
                for (j, seg) in segs.iter().enumerate() {
                    if j > 0 {
                        vx = canvas.text(SEG_SEP, 17.0, vx, baseline, t.ink_mute, false);
                    }
                    vx = canvas.text(&seg.text, 17.0, vx, baseline, t.tone(seg.tone), true);
                }
            }
        }
    }
}

/// 多段數值之間的分隔符
const SEG_SEP: &str = " / ";

fn segs_width(canvas: &Canvas, segs: &[crate::data::Seg]) -> f32 {
    let sep = canvas.measure(SEG_SEP, 17.0, false) * (segs.len().saturating_sub(1)) as f32;
    segs.iter().map(|s| canvas.measure(&s.text, 17.0, true)).sum::<f32>() + sep
}

fn draw_stat_cell(canvas: &mut Canvas, stat: &Stat, x: f32, y: f32, w: f32) {
    let t = canvas.t;
    canvas.box_(x, y, w, STAT_TALL_H, 8.0, rgb(t.block_bg), Some(rgba(t.line)));
    draw_stat_lines(canvas, stat, x, y);
}

/// 共鳴區（含內距）所需寬度
fn res_chips_width(canvas: &Canvas, chips: &[(String, String, FbClass, f32, f32)]) -> f32 {
    if chips.is_empty() {
        return 0.0;
    }
    let chips_w: f32 = chips
        .iter()
        .map(|(_, _, _, nw, sw)| BADGE_PAD_X * 2.0 + nw + 10.0 + sw + 6.0)
        .sum::<f32>()
        - 6.0;
    chips_w.max(canvas.measure("三方共鳴白因", 17.0, true)) + 28.0
}
