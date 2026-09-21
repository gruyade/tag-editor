// Feature: tag-editor, Property 19: 孤立キャプションの特定
//
// Property 19: 任意の Image_File 名集合と Tag_File 名集合について、
// Orphan_Caption として特定される集合は「対応する basename の Image_File が
// 存在しない Tag_File」の集合とちょうど一致する。
//
// 対象: tag_editor_core::services::caption_service::find_orphan_captions
//
// find_orphan_captions はフォルダ直下（非再帰）の各 `.txt` について、同じ
// basename を持つ Image_File（jpg / jpeg / png / gif / mp4、大小無視）が同
// フォルダに 1 つでも存在すれば非孤立、1 つも無ければ Orphan_Caption と判定し、
// 孤立と判定した `.txt` のフルパス一覧を返す。本テストは実装とは独立に期待集合
// を計算し、返却集合（basename 集合として比較）とちょうど一致することを検証する。
//
// Validates: Requirements 11.1

use std::collections::{HashMap, HashSet};

use proptest::prelude::*;
use tempfile::tempdir;

/// 対応 Image_File 拡張子（小文字表記、要件 11.1）。
const IMAGE_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "gif", "mp4"];

/// 各 basename に対して生成するファイルの種類。
///
/// - `image_ext`: Some(拡張子) のとき、その拡張子で Image_File を作成する。
/// - `image_upper`: 画像拡張子を大文字化するか（大小無視判定のカバレッジ用）。
/// - `has_txt`: 対応する `.txt`（Tag_File）を作成するか。
#[derive(Debug, Clone)]
struct FileSpec {
    basename: String,
    image_ext: Option<&'static str>,
    image_upper: bool,
    has_txt: bool,
}

/// basename 生成器。拡張子・パス区切りと衝突しない安全な文字のみを用いる。
///
/// 大小無視の突き合わせを検証対象に含めるため、英大文字・小文字の双方を許可する。
fn basename_strategy() -> impl Strategy<Value = String> {
    // 先頭に英字を要求し、以降は英数字とアンダースコアのみ。ドット・空白・
    // パス区切りを除外することで basename の解釈が一意になる。
    "[A-Za-z][A-Za-z0-9_]{0,7}".prop_map(|s| s)
}

/// 1 件分の FileSpec 生成器。
///
/// 4 つの被覆ケース（txt+画像=非孤立 / txt のみ=孤立 / 画像のみ=無関係 /
/// 大小混在拡張子）をすべて生成しうるよう、各フラグを独立にサンプリングする。
fn file_spec_strategy() -> impl Strategy<Value = FileSpec> {
    (
        basename_strategy(),
        prop::option::of(prop::sample::select(IMAGE_EXTENSIONS.to_vec())),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(basename, image_ext, image_upper, has_txt)| FileSpec {
            basename,
            image_ext,
            image_upper,
            has_txt,
        })
}

/// basename 集合（大小無視で一意化）に対する FileSpec の集合を生成する。
///
/// 同一 basename の重複は挙動を曖昧にするため、大小無視で先勝ち重複排除する。
fn specs_strategy() -> impl Strategy<Value = Vec<FileSpec>> {
    prop::collection::vec(file_spec_strategy(), 0..12).prop_map(|specs| {
        let mut seen: HashSet<String> = HashSet::new();
        specs
            .into_iter()
            .filter(|s| seen.insert(s.basename.to_ascii_lowercase()))
            .collect()
    })
}

/// 拡張子（先頭のドットを除いた末尾要素）を取り出す。
fn extension_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, ext)| ext)
}

/// 拡張子を除いた basename を返す。
fn basename_of(name: &str) -> &str {
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 19: 返却された Orphan_Caption 集合は、対応する basename の
    /// Image_File を持たない `.txt` の集合とちょうど一致する（要件 11.1）。
    #[test]
    fn orphans_equal_txt_without_matching_image(specs in specs_strategy()) {
        use tag_editor_core::services::caption_service::find_orphan_captions;

        let dir = tempdir().expect("一時ディレクトリ作成");

        // 実ファイルを配置する。
        for spec in &specs {
            if let Some(ext) = spec.image_ext {
                let ext = if spec.image_upper {
                    ext.to_ascii_uppercase()
                } else {
                    ext.to_string()
                };
                let img_name = format!("{}.{}", spec.basename, ext);
                std::fs::write(dir.path().join(&img_name), b"img").expect("画像作成");
            }
            if spec.has_txt {
                let txt_name = format!("{}.txt", spec.basename);
                std::fs::write(dir.path().join(&txt_name), b"tag").expect("txt作成");
            }
        }

        // --- 実装とは独立な期待集合の算出 ---
        // 大小無視で「画像が存在する basename」の集合を作る。
        let mut image_basenames: HashSet<String> = HashSet::new();
        for spec in &specs {
            if spec.image_ext.is_some() {
                image_basenames.insert(spec.basename.to_ascii_lowercase());
            }
        }
        // 孤立 = 「.txt があり、かつ同一 basename の画像が存在しない」もの。
        // basename は大小無視で突き合わせる。
        let expected: HashSet<String> = specs
            .iter()
            .filter(|s| s.has_txt)
            .filter(|s| !image_basenames.contains(&s.basename.to_ascii_lowercase()))
            .map(|s| s.basename.clone())
            .collect();

        // --- 実際の返却値を basename 集合へ変換 ---
        let orphans = find_orphan_captions(dir.path()).expect("走査成功");

        // 返却は `.txt` のフルパス。ファイル名を取り出し拡張子を確認する。
        let actual: HashSet<String> = orphans
            .iter()
            .map(|full| {
                let file_name = std::path::Path::new(full)
                    .file_name()
                    .expect("ファイル名あり")
                    .to_string_lossy()
                    .into_owned();
                // 返却は必ず .txt のはず。
                prop_assert_eq!(
                    extension_of(&file_name).map(|e| e.eq_ignore_ascii_case("txt")),
                    Some(true),
                    "孤立として返却されたのが .txt ではない: {}",
                    file_name
                );
                Ok(basename_of(&file_name).to_string())
            })
            .collect::<Result<HashSet<String>, TestCaseError>>()?;

        // 集合として一致（過不足なし）。
        prop_assert_eq!(
            &actual,
            &expected,
            "孤立集合の不一致: 実際={:?} 期待={:?} (specs={:?})",
            actual,
            expected,
            specs
                .iter()
                .map(|s| (s.basename.clone(), s.image_ext, s.image_upper, s.has_txt))
                .collect::<Vec<_>>()
        );

        // 返却に重複が無いこと（basename 数 == パス数）。
        prop_assert_eq!(
            actual.len(),
            orphans.len(),
            "返却に重複が含まれる: {:?}",
            orphans
        );
    }

    /// Property 19（補足）: 大小混在拡張子でも Image_File として認識され、
    /// 対応 `.txt` は孤立にならない。Image_File のみの basename は結果に
    /// 一切現れない（無関係）。上の集合一致で包含されるが、被覆ケースを
    /// 明示的に固定して退行を防ぐ。
    #[test]
    fn image_only_never_appears_and_case_insensitive(
        base in basename_strategy(),
        upper in any::<bool>(),
        ext in prop::sample::select(IMAGE_EXTENSIONS.to_vec()),
    ) {
        use tag_editor_core::services::caption_service::find_orphan_captions;

        let dir = tempdir().expect("一時ディレクトリ作成");
        let ext_str = if upper { ext.to_ascii_uppercase() } else { ext.to_string() };

        // 画像 + txt（大小混在拡張子） -> 非孤立。
        std::fs::write(dir.path().join(format!("{}.{}", base, ext_str)), b"img")
            .expect("画像作成");
        std::fs::write(dir.path().join(format!("{}.txt", base)), b"tag")
            .expect("txt作成");
        // 画像のみ（別 basename） -> 結果に現れないはず。
        let image_only = format!("{}img_only", base);
        std::fs::write(dir.path().join(format!("{}.{}", image_only, ext_str)), b"img")
            .expect("画像のみ作成");

        let orphans = find_orphan_captions(dir.path()).expect("走査成功");
        let bases: HashMap<String, ()> = orphans
            .iter()
            .map(|f| {
                let name = std::path::Path::new(f)
                    .file_name().unwrap().to_string_lossy().into_owned();
                (basename_of(&name).to_string(), ())
            })
            .collect();

        prop_assert!(
            !bases.contains_key(&base),
            "画像を持つ basename が孤立扱い（大小無視失敗）: {}", base
        );
        prop_assert!(
            !bases.contains_key(&image_only),
            "画像のみの basename が結果に混入: {}", image_only
        );
        prop_assert!(orphans.is_empty(), "孤立は無いはず: {:?}", orphans);
    }
}
