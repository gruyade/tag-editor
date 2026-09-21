// Feature: tag-editor, Property 13: 仕訳・改名における Image と Tag_File の対保存
//
// Property 13（改名系）:
//   任意の Image_File と（存在する場合の）対応 Tag_File について、改名操作
//   （正規表現置換 / 連番）後、
//     - 対応 Tag_File が存在すれば、Image_File と同じ命名規則で改名されて
//       対（同一 basename）が維持される。
//     - 対応 Tag_File が存在しなければ、Image_File のみが処理され、
//       対応する新しい `.txt` は生成されない。
//
// 対象: tag_editor_core::services::rename_service::{rename_regex, Numbering}
//       （改名系、一時ディレクトリ使用）
//
// Validates: Requirements 7.2, 8.3, 9.4, 9.5
//
// ── ジェネレータ制約（本文が前提とする「成功する改名」の入力空間へ最小限に
//    絞り、それぞれ正当化する）─────────────────────────────────────────
//
// Property 13 が主張する「対保存」は、改名が成功したケースについての不変条件
// である。禁止文字・既存衝突で当該のみスキップ/衝突記録される挙動は別プロパティ
// （要件 8.5 / 8.6 の単体テスト）で扱う。ここでは対保存そのものを検証するため、
// 生成する入力を「衝突・禁止文字が起きない」空間へ制約する:
//
//   - basename は安全な英数字（`a-z0-9`）のみから生成し、集合内で互いに
//     distinct にする。
//       → 改名前の名前が一意になり、改名アルゴリズムの列挙順（ファイル名昇順）
//         に依らず各 Image が独立に処理される。
//   - 拡張子は対応 Image 拡張子の `.png` に固定する。
//       → is_image_file_name の判定対象になり、Tag_File（`.txt`）と明確に
//         区別される。
//   - 改名の命名規則は 2 系統のみを用い、いずれも「新しい名前が互いに distinct
//     かつ禁止文字を含まず、既存ファイルと衝突しない」ことを構成的に保証する:
//       (a) 連番系: 名前全体を `img_NUM.png` へ置換し、NUM を十分な桁で
//           ゼロ埋め展開する。昇順 index ごとに異なる番号になるため新名は
//           全て distinct。接頭辞 `img_` は既存 basename 空間（英数字のみ）と
//           衝突しうるが、連番により `img_<数字>` の形は元の basename には
//           出現しない（元は `s` 始まりの固定接頭辞を用いて分離する）。
//       (b) 接頭辞系（連番なし）: 名前先頭へ固定接頭辞を付与する
//           （`^` → `pre_`）。元 basename が distinct なら新名も distinct。
//           新名は元集合の名前空間と接頭辞で分離されるため既存衝突しない。
//     いずれの置換文字列・パターンも禁止文字（`< > : " / \ | ? *`・制御）を
//     含まない ASCII のみ。
//   - Tag_File の有無は各 Image ごとに独立な bool で決める。
//       → 「対あり」「対なし」双方を同一ディレクトリ内に混在させ、対保存と
//         「対なしは Image のみ処理」の双方を同時に検証する。
//
// これらの制約下では rename_regex は全 Image を成功改名する（succeeded ==
// 対象件数、conflicted == skipped == 0）はずであり、その上で対保存を確認する。

use std::collections::BTreeSet;
use std::path::Path;

use proptest::prelude::*;
use tempfile::tempdir;

use tag_editor_core::services::rename_service::{rename_regex, Numbering};

/// 安全な basename 1 個の生成器（英数字のみ、1〜8 文字、`a-z0-9`）。
///
/// 禁止文字・制御文字・`.`（拡張子境界）を一切含まないため、
/// `<basename>.png` / `<basename>.txt` が常に妥当なファイル名になる。
fn basename_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,7}".prop_map(|s| s)
}

/// distinct な basename の集合と、各々に対の Tag_File を持たせるかの bool を
/// 生成する。返すのは `(basename, has_tag)` の並び。
///
/// basename は BTreeSet で重複排除し、集合内 distinct を保証する。件数は
/// 1〜12 件（境界 1 と複数件を含む）。
fn image_set_strategy() -> impl Strategy<Value = Vec<(String, bool)>> {
    prop::collection::vec((basename_strategy(), any::<bool>()), 1..=12).prop_map(|pairs| {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out: Vec<(String, bool)> = Vec::new();
        for (base, has_tag) in pairs {
            if seen.insert(base.clone()) {
                out.push((base, has_tag));
            }
        }
        out
    })
}

/// テスト用: 空ファイルを作成する。
fn touch(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), b"").expect("ファイル作成");
}

/// テスト用: ファイルの存在判定。
fn exists(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

/// フォルダ直下の `.txt` ファイル名を集合で返す（余分な `.txt` 生成の検出用）。
fn txt_names(dir: &Path) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.to_ascii_lowercase().ends_with(".txt") {
            set.insert(name);
        }
    }
    set
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 13（連番系）: 名前全体を `img_NUM.png` へ置換して連番改名する。
    ///
    /// 元 basename は英数字のみで distinct、拡張子は `.png`。連番展開により
    /// 新しい Image 名は全て distinct（`img_000..` 形式）で、元の名前空間とは
    /// 分離されるため衝突・禁止文字なし。
    ///
    /// 検証:
    ///   - 全 Image が成功改名（succeeded == 件数、conflicted == skipped == 0）。
    ///   - 元 Image / 元 Tag_File はいずれも残っていない。
    ///   - 対を持っていた Image は、改名後 Image と同一 basename の `.txt` が
    ///     存在し対が維持される（要件 8.3）。
    ///   - 対を持たなかった Image は、改名後 Image のみ存在し、対応する `.txt`
    ///     が生成されていない。
    ///   - ディレクトリ内の `.txt` 総数は「対を持っていた Image の件数」と一致
    ///     （新規 `.txt` の湧出・消失がない）。
    #[test]
    fn numbering_rename_preserves_pairs(images in image_set_strategy()) {
        let dir = tempdir().expect("一時ディレクトリ");
        let path = dir.path();

        // 事前配置。Image は全て `.png`、対を持つものだけ `.txt` も作る。
        let mut paired_count = 0usize;
        for (base, has_tag) in &images {
            touch(path, &format!("{base}.png"));
            if *has_tag {
                touch(path, &format!("{base}.txt"));
                paired_count += 1;
            }
        }

        // 昇順 index に一意な番号を割り当てるため、件数を収容できる桁数を選ぶ。
        // 12 件以下なので 3 桁で十分（000..011）。開始 0。
        let numbering = Numbering { start: 0, width: 3 };
        let report = rename_regex(path, r"^.*\.png$", "img_NUM.png", Some(numbering))
            .expect("rename_regex は成功する（正規表現は妥当）");

        // 全 Image が成功改名し、衝突・スキップは無い。
        prop_assert_eq!(report.succeeded, images.len(),
            "成功件数が Image 件数と不一致: report={:?}", report);
        prop_assert_eq!(report.conflicted, 0, "衝突が発生: report={:?}", report);
        prop_assert_eq!(report.skipped, 0, "スキップが発生: report={:?}", report);

        // 元ファイルはいずれも残っていない。
        for (base, has_tag) in &images {
            prop_assert!(!exists(path, &format!("{base}.png")),
                "元 Image が残存: {}.png", base);
            if *has_tag {
                prop_assert!(!exists(path, &format!("{base}.txt")),
                    "元 Tag_File が残存: {}.txt", base);
            }
        }

        // 改名後の Image は昇順 index に対応する `img_00X.png`。
        // 対を持っていた Image は同 basename の `.txt` が存在するはず。
        // 列挙順を再現するため元 Image 名を昇順に並べ、対の有無を引く。
        let mut sorted: Vec<(String, bool)> = images.clone();
        sorted.sort_by(|a, b| format!("{}.png", a.0).cmp(&format!("{}.png", b.0)));

        for (index, (_base, has_tag)) in sorted.iter().enumerate() {
            let new_base = format!("img_{index:03}");
            prop_assert!(exists(path, &format!("{new_base}.png")),
                "改名後 Image が存在しない: {}.png", new_base);
            if *has_tag {
                prop_assert!(exists(path, &format!("{new_base}.txt")),
                    "対を持つ Image の Tag_File が改名されていない: {}.txt", new_base);
            } else {
                prop_assert!(!exists(path, &format!("{new_base}.txt")),
                    "対を持たない Image に不要な Tag_File が生成された: {}.txt", new_base);
            }
        }

        // `.txt` の総数は対を持っていた件数と一致（湧出・消失なし）。
        prop_assert_eq!(txt_names(path).len(), paired_count,
            "Tag_File の総数が対の件数と不一致");
    }

    /// Property 13（接頭辞系・連番なし）: 名前先頭へ固定接頭辞を付与する。
    ///
    /// パターン `^`（先頭アンカー）を接頭辞 `pre_` で置換する。元 basename が
    /// distinct なら新 basename（`pre_<元>`）も distinct。新名は接頭辞で
    /// 既存名前空間と分離されるため衝突なし。禁止文字も含まない。
    ///
    /// 検証内容は連番系と同じ（対保存・対なしは Image のみ・`.txt` 総数保存）。
    #[test]
    fn prefix_rename_preserves_pairs(images in image_set_strategy()) {
        let dir = tempdir().expect("一時ディレクトリ");
        let path = dir.path();

        let mut paired_count = 0usize;
        for (base, has_tag) in &images {
            touch(path, &format!("{base}.png"));
            if *has_tag {
                touch(path, &format!("{base}.txt"));
                paired_count += 1;
            }
        }

        // 先頭に `pre_` を挿入。`^` は空マッチのため replace_all は先頭のみ置換。
        let report = rename_regex(path, r"^", "pre_", None)
            .expect("rename_regex は成功する（正規表現は妥当）");

        prop_assert_eq!(report.succeeded, images.len(),
            "成功件数が Image 件数と不一致: report={:?}", report);
        prop_assert_eq!(report.conflicted, 0, "衝突が発生: report={:?}", report);
        prop_assert_eq!(report.skipped, 0, "スキップが発生: report={:?}", report);

        for (base, has_tag) in &images {
            // 元は残っていない。
            prop_assert!(!exists(path, &format!("{base}.png")),
                "元 Image が残存: {}.png", base);
            // 新 Image が存在。
            let new_base = format!("pre_{base}");
            prop_assert!(exists(path, &format!("{new_base}.png")),
                "改名後 Image が存在しない: {}.png", new_base);

            if *has_tag {
                // 対を持つ場合、元 `.txt` は消え、新 basename の `.txt` が存在。
                prop_assert!(!exists(path, &format!("{base}.txt")),
                    "元 Tag_File が残存: {}.txt", base);
                prop_assert!(exists(path, &format!("{new_base}.txt")),
                    "対を持つ Image の Tag_File が改名されていない: {}.txt", new_base);
            } else {
                // 対を持たない場合、新 basename の `.txt` は生成されない。
                prop_assert!(!exists(path, &format!("{new_base}.txt")),
                    "対を持たない Image に不要な Tag_File が生成された: {}.txt", new_base);
            }
        }

        prop_assert_eq!(txt_names(path).len(), paired_count,
            "Tag_File の総数が対の件数と不一致");
    }
}
