//! 「最高評價」點法：技能點數內，讓育成評價點（rank_score）的技能分最高。
//!
//! 技能分（2026-11-01 用 431 匹完成馬的封包反推，holdout 誤差約 2 分）：
//!
//! - 每個技能 `grade_value × 跑法係數 × 距離係數`；係數 S/A 1.1、B/C 0.9、D/E/F 0.8、G 0.7。
//!   跑法看 tag 101–104，距離看 201–204，同一類有多個 tag 取最高的適性，沒有就是 1.0；
//!   場地 tag（501/502）不算。
//! - 上位取代下位、不疊加：同一列已經有 ○，買 ◎ 只多 `◎ − ○`。
//!
//! 能力值分跟點技能無關，這裡不算。

/// 適性（遊戲的數值 1＝G … 8＝S），順序：逃、先行、差、追、短、英里、中、長
pub type Propers = [i32; 8];

fn proper_coef(grade: i32) -> f64 {
    match grade {
        7..=8 => 1.1,
        5..=6 => 0.9,
        2..=4 => 0.8,
        _ => 0.7,
    }
}

/// 技能的評價分（含適性係數）
pub fn skill_score(grade_value: i32, tags: &str, propers: &Propers) -> f64 {
    let mut style: Option<i32> = None;
    let mut distance: Option<i32> = None;
    for tag in tags.split('/') {
        let Ok(tag) = tag.trim().parse::<i32>() else { continue };
        let slot = match tag {
            101..=104 => &mut style,
            201..=204 => &mut distance,
            _ => continue,
        };
        let idx = match tag {
            101..=104 => (tag - 101) as usize,
            _ => (tag - 201) as usize + 4,
        };
        let p = propers[idx];
        *slot = Some(slot.map_or(p, |cur| cur.max(p)));
    }
    let mut coef = 1.0;
    if let Some(p) = style {
        coef *= proper_coef(p);
    }
    if let Some(p) = distance {
        coef *= proper_coef(p);
    }
    grade_value as f64 * coef
}

/// 一列技能的一個選項：買到第 `tier` 階（從 0 起算，已學的那階以上），花 `cost` 點，多 `gain` 分
#[derive(Clone, Debug)]
pub struct RowOption {
    pub tier: usize,
    pub cost: i32,
    pub gain: f64,
}

/// 把共用技能的列併成同一組。遊戲的技能列表裡，同一個白技能會自己一列、又出現在「白→金」那列；
/// 不併的話兩列都會被選，白技能的分數和點數就重複算了（上位取代下位，實際只算一次）。
/// 金技能那列也可能只有 [金]（白沒列進去、試選時遊戲自動連白扣點），所以呼叫端要傳
/// 升級鏈 id（mdb `skill_data.group_id`）而不是技能 id，白、金才會併在一起。
/// 輸入每列的 id，回傳每組包含哪些列（依第一次出現的順序）。
pub fn families(rows: &[Vec<i32>]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..rows.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut owner: std::collections::HashMap<i32, usize> = Default::default();
    for (r, ids) in rows.iter().enumerate() {
        for id in ids {
            match owner.get(id) {
                Some(&o) => {
                    let (a, b) = (find(&mut parent, o), find(&mut parent, r));
                    if a != b {
                        parent[b.max(a)] = a.min(b);
                    }
                }
                None => {
                    owner.insert(*id, r);
                }
            }
        }
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut index: std::collections::HashMap<usize, usize> = Default::default();
    for r in 0..rows.len() {
        let root = find(&mut parent, r);
        let i = *index.entry(root).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[i].push(r);
    }
    out
}

/// 分組背包：每列最多選一個選項，總花費 ≤ `budget`，總分最高；同分取花費少的。
/// 回傳每列選到的選項索引（`None`＝不買）。
pub fn optimize(rows: &[Vec<RowOption>], budget: i32) -> Vec<Option<usize>> {
    let budget = budget.max(0) as usize;
    const EPS: f64 = 1e-6;
    // best[c]＝花費恰好 ≤ c 時的最高分；choice[r][c]＝第 r 列在預算 c 時的選擇
    let mut best = vec![0.0f64; budget + 1];
    let mut choice: Vec<Vec<Option<usize>>> = Vec::with_capacity(rows.len());
    for options in rows {
        let prev = best.clone();
        let mut pick = vec![None; budget + 1];
        for c in 0..=budget {
            for (i, opt) in options.iter().enumerate() {
                if opt.cost < 0 || opt.gain <= EPS {
                    continue;
                }
                let cost = opt.cost as usize;
                if cost > c {
                    continue;
                }
                let v = prev[c - cost] + opt.gain;
                if v > best[c] + EPS {
                    best[c] = v;
                    pick[c] = Some(i);
                }
            }
        }
        choice.push(pick);
    }

    // 最高分裡取花費最少的預算點，再往回推
    let top = best[budget];
    let mut c = (0..=budget).find(|&c| best[c] + EPS >= top).unwrap_or(budget);
    let mut result = vec![None; rows.len()];
    for r in (0..rows.len()).rev() {
        if let Some(i) = choice[r][c] {
            result[r] = Some(i);
            c -= rows[r][i].cost as usize;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // 逃A 先行A 差C 追G / 短C 英A 中S 長B
    const P: Propers = [7, 7, 5, 1, 5, 7, 8, 6];

    #[test]
    fn coefficients() {
        // 無跑法/距離 tag
        assert!((skill_score(217, "401", &P) - 217.0).abs() < 1e-9);
        // 追 G
        assert!((skill_score(508, "104/303/401", &P) - 508.0 * 0.7).abs() < 1e-9);
        // 追 G × 中 S（君臨）
        assert!((skill_score(508, "104/203/303/403", &P) - 508.0 * 0.7 * 1.1).abs() < 1e-9);
        // 同類多 tag 取最高：逃 A、追 G → 1.1
        assert!((skill_score(129, "101/104", &P) - 129.0 * 1.1).abs() < 1e-9);
        // 場地 tag 不算
        assert!((skill_score(508, "401/502", &P) - 508.0).abs() < 1e-9);
    }

    #[test]
    fn knapsack_picks_best_combo() {
        let rows = vec![
            vec![RowOption { tier: 0, cost: 100, gain: 120.0 }, RowOption { tier: 1, cost: 250, gain: 400.0 }],
            vec![RowOption { tier: 0, cost: 150, gain: 200.0 }],
            vec![RowOption { tier: 0, cost: 120, gain: 130.0 }],
        ];
        // 預算 400：250+150＝600 分，勝過 100+150+120＝450
        assert_eq!(optimize(&rows, 400), vec![Some(1), Some(0), None]);
        // 預算 370：250+120＝530 分，勝過 100+150+120＝450
        assert_eq!(optimize(&rows, 370), vec![Some(1), None, Some(0)]);
        // 預算 0：什麼都不買
        assert_eq!(optimize(&rows, 0), vec![None, None, None]);
    }

    #[test]
    fn families_merge_shared_skills() {
        // 列 0＝[白A]、列 1＝[白B]、列 2＝[白A, 金A]、列 3＝[白C, 金C]、列 4＝[白C]
        let rows = vec![vec![1], vec![2], vec![1, 10], vec![3, 30], vec![3]];
        assert_eq!(families(&rows), vec![vec![0, 2], vec![1], vec![3, 4]]);
    }

    #[test]
    fn families_merge_gold_only_row_by_chain() {
        // 2026-11-03 實機：開拓活路！(203652) 自己一列、開拓者(203651) 那列只有金 → 用 group_id 20365 才併得起來
        let rows = vec![vec![20131], vec![20365], vec![20201], vec![20365]];
        assert_eq!(families(&rows), vec![vec![0], vec![1, 3], vec![2]]);
    }

    #[test]
    fn tie_prefers_cheaper() {
        let rows = vec![
            vec![RowOption { tier: 0, cost: 200, gain: 100.0 }],
            vec![RowOption { tier: 0, cost: 100, gain: 100.0 }],
        ];
        assert_eq!(optimize(&rows, 200), vec![None, Some(0)]);
    }
}
