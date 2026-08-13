//! 字型載入、量測與光柵化。
//!
//! 卡片是中英數混排，但不需要複雜整形（no shaping / ligature），所以直接用 `ab_glyph`
//! 逐字取 glyph + advance 就夠了；缺字時往 fallback 字型鏈找。

use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont as _};
use tiny_skia::{Pixmap, PremultipliedColorU8};

/// 一種字重的字型鏈（第一個有該字的就用它）
#[derive(Default)]
struct Chain(Vec<FontVec>);

impl Chain {
    fn face_for(&self, c: char) -> Option<(&FontVec, ab_glyph::GlyphId)> {
        let mut first = None;
        for f in &self.0 {
            let gid = f.glyph_id(c);
            if gid.0 != 0 {
                return Some((f, gid));
            }
            first.get_or_insert((f, gid));
        }
        first
    }
}

pub struct Fonts {
    regular: Chain,
    bold: Chain,
}

/// 字型檔（可為 .ttc，需指定 index）
pub struct FontSource {
    pub data: Vec<u8>,
    pub index: u32,
}

impl Fonts {
    pub fn new(regular: Vec<FontSource>, bold: Vec<FontSource>) -> Result<Fonts, String> {
        let load = |v: Vec<FontSource>| -> Result<Chain, String> {
            let mut chain = Chain::default();
            for s in v {
                let f = FontVec::try_from_vec_and_index(s.data, s.index).map_err(|e| e.to_string())?;
                chain.0.push(f);
            }
            Ok(chain)
        };
        let regular = load(regular)?;
        if regular.0.is_empty() {
            return Err("沒有可用的字型".to_owned());
        }
        let mut bold = load(bold)?;
        if bold.0.is_empty() {
            // 沒有粗體就退回一般字重（畫粗體時再做假粗）
            bold = Chain(Vec::new());
        }
        Ok(Fonts { regular, bold })
    }

    /// 由檔案路徑載入；找不到的檔案直接略過。
    pub fn from_paths(regular: &[(&str, u32)], bold: &[(&str, u32)]) -> Result<Fonts, String> {
        let read = |list: &[(&str, u32)]| -> Vec<FontSource> {
            list.iter()
                .filter_map(|(p, i)| std::fs::read(p).ok().map(|data| FontSource { data, index: *i }))
                .collect()
        };
        Fonts::new(read(regular), read(bold))
    }

    /// Windows 系統字型（微軟正黑體，缺字時退到微軟雅黑／Arial）
    #[cfg(target_os = "windows")]
    pub fn system() -> Result<Fonts, String> {
        let root = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned());
        let p = |n: &str| format!("{root}\\Fonts\\{n}");
        Fonts::from_paths(
            &[(&p("msjh.ttc"), 0), (&p("msyh.ttc"), 0), (&p("arial.ttf"), 0)],
            &[(&p("msjhbd.ttc"), 0), (&p("msyhbd.ttc"), 0), (&p("arialbd.ttf"), 0)],
        )
    }

    /// 非 Windows 的便利路徑（Noto Sans CJK）。
    ///
    /// ⚠️ ttc index 這裡一律給 0，而 Debian 的 `NotoSansCJK-*.ttc` 第 0 面是 **JP**，
    /// 部分漢字會是日文字形。要正體字形請改用 [`Fonts::from_paths`] 明確指定 TC 的
    /// index（用 `fc-query -i` 查）。這個函式只是給開發／臨時測試用的方便入口。
    #[cfg(not(target_os = "windows"))]
    pub fn system() -> Result<Fonts, String> {
        const DIRS: &[&str] = &["/usr/share/fonts/opentype/noto", "/usr/share/fonts/truetype/noto"];
        let find = |name: &str| -> Vec<(String, u32)> {
            DIRS.iter().map(|d| (format!("{d}/{name}"), 0u32)).collect()
        };
        // 自由函式而非 closure：closure 推不出「回傳值借用參數」的 lifetime
        fn borrow(v: &[(String, u32)]) -> Vec<(&str, u32)> {
            v.iter().map(|(p, i)| (p.as_str(), *i)).collect()
        }
        let (reg, bold) = (find("NotoSansCJK-Regular.ttc"), find("NotoSansCJK-Bold.ttc"));
        Fonts::from_paths(&borrow(&reg), &borrow(&bold))
    }

    fn chain(&self, bold: bool) -> &Chain {
        if bold && !self.bold.0.is_empty() {
            &self.bold
        } else {
            &self.regular
        }
    }

    /// 字型在該字級下的 ascent / descent（正值向上 / 正值向下）
    pub fn v_metrics(&self, size: f32, bold: bool) -> (f32, f32) {
        let chain = self.chain(bold);
        let f = chain.0[0].as_scaled(PxScale::from(size));
        (f.ascent(), -f.descent())
    }

    pub fn measure(&self, text: &str, size: f32, bold: bool) -> f32 {
        let chain = self.chain(bold);
        let scale = PxScale::from(size);
        let mut w = 0.0;
        for c in text.chars() {
            if let Some((face, gid)) = chain.face_for(c) {
                w += face.as_scaled(scale).h_advance(gid);
            }
        }
        w
    }

    /// 以 `baseline` 為基線畫一段文字，回傳結束的 x。
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        text: &str,
        size: f32,
        x: f32,
        baseline: f32,
        color: [u8; 3],
        bold: bool,
    ) -> f32 {
        // 沒有粗體字型時用「微量重描」假造粗體
        let fake_bold = bold && self.bold.0.is_empty();
        let chain = self.chain(bold);
        let scale = PxScale::from(size);
        let mut pen = x;
        for c in text.chars() {
            let Some((face, gid)) = chain.face_for(c) else { continue };
            let scaled = face.as_scaled(scale);
            let advance = scaled.h_advance(gid);
            // 一般字重再描一次（位移不到半個像素）＝ stem darkening，讓小字不會太細；
            // 沒有粗體字型時位移大一點來假造粗體。
            let passes: &[f32] = if fake_bold {
                &[0.0, 0.5]
            } else if bold {
                &[0.0]
            } else {
                &[0.0, 0.3]
            };
            for dx in passes {
                let glyph = gid.with_scale_and_position(scale, ab_glyph::point(pen + dx, baseline));
                if let Some(outlined) = face.outline_glyph(glyph) {
                    let bounds = outlined.px_bounds();
                    outlined.draw(|gx, gy, cov| {
                        let px = bounds.min.x as i32 + gx as i32;
                        let py = bounds.min.y as i32 + gy as i32;
                        blend(pixmap, px, py, color, cov);
                    });
                }
            }
            pen += advance;
        }
        pen
    }
}

/// 把 `color`（不透明）以 `alpha` 覆蓋率混到 pixmap 上（背景視為不透明）
fn blend(pixmap: &mut Pixmap, x: i32, y: i32, color: [u8; 3], alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 || x >= pixmap.width() as i32 || y >= pixmap.height() as i32 {
        return;
    }
    // 純覆蓋率混色在小字上會顯得太細（瀏覽器會做 gamma 校正＋stem darkening），
    // 這裡把覆蓋率做 gamma 提升，讓筆畫看起來厚一點、比較好讀。
    let a = alpha.min(1.0).powf(0.70);
    let idx = y as usize * pixmap.width() as usize + x as usize;
    let px = pixmap.pixels_mut();
    let dst = px[idx];
    let mix = |s: u8, d: u8| -> u8 { (s as f32 * a + d as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8 };
    let (r, g, b) = (mix(color[0], dst.red()), mix(color[1], dst.green()), mix(color[2], dst.blue()));
    let alpha_out = mix(255, dst.alpha());
    px[idx] = PremultipliedColorU8::from_rgba(
        r.min(alpha_out),
        g.min(alpha_out),
        b.min(alpha_out),
        alpha_out,
    )
    .unwrap_or(dst);
}
