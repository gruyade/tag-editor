// Feature: tag-editor, Property 13: 仕訳・改名における Image と Tag_File の対保存
//
// Validates: Requirements 7.2, 8.3, 9.4, 9.5
//
// 本ファイルは移動/コピー系（`SortingOperation::Move` / `SortingOperation::Copy`）
// における対保存を検証する（Property 13 の改名系はタスク 13.2 で別途検証）。
//
// 対象: `tag_editor_core::services::sort_service::sort_files` を
//        `tag_editor_core::models::SortingOperation::{Move, Copy}` で実行。
//        統合テスト（tests/）として外部から検証し、モジュール本体は編集しない。
//
// プロパティ本文:
//   任意の Image_File 集合と（存在する場合の）対応 Tag_File について、
//   移動・コピー操作後、
//     - 対応 Tag_File が存在すれば Image_File と同じ宛先へ同一操作で配置されて
//       対が維持される（Image と .txt の両方が宛先に同一 basename で存在）。
//     - 存在しなければ Image_File のみが処理され、余計な .txt は宛先に現れない。
//   加えて操作意味論:
//     - Move: 元は宛先へ移り、対象フォルダから消える。
//     - Copy: 元は対象フォルダに残り、かつ宛先にも複製が存在する。
//
// ジェネレータ制約（本文が前提とする入力空間へ最小限に絞り、それぞれ正当化）:
//   - basename は「文字列的にグローバル一意」に生成する（`img{連番}` 形式）。
//       理由: sort_files の対 Tag_File 名は `<basename>.txt`（拡張子非依存）で
//       決まるため、basename が重複すると Image 側は非衝突でも Tag_File 側が
//       宛先で衝突し得る。本プロパティは「対が維持される／余計な .txt が出ない」
//       という非衝突時の正当な帰結を検証するものなので、basename を一意にして
//       宛先衝突を排除する（要件 7.5 の衝突経路は 14.3 の単体テストで別途検証）。
//   - 拡張子は対応 Image 拡張子 {jpg, jpeg, png, gif, mp4} から各画像ごとに選ぶ。
//       basename が一意なので、拡張子違いによる basename 共有も起きない。
//   - 宛先フォルダは毎回空の一時ディレクトリを用いる。
//       既存ファイルが無いため Image・Tag_File とも宛先衝突が起きず、プロパティ
//       の前提（非衝突での対保存）が成り立つ。
//   - 各画像に対の .txt を持たせるか否かは bool で生成し、両ケースを混在させる。
//   - 操作種別（Move / Copy）も生成し、両意味論を検証する。
//   - 画像枚数は 0〜12 枚。空集合（0 枚）境界も踏む。

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use proptest::collection::vec;
use proptest::prelude::*;
use tempfile::tempdir;

use tag_editor_core::models::SortingOperation;
use tag_editor_core::services::sort_service::sort_files;

/// 対応 Image 拡張子（file_service::SUPPORTED_EXTENSIONS と一致）。
const EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "gif", "mp4"];

/// 1 枚の画像の生成仕様。
#[derive(Debug, Clone)]
struct ImageSpec {
    /// グローバル一意な basename（`.` を含まない）。
    basename: String,
    /// 拡張子（EXTENSIONS のいずれか）。
    ext: &'static str,
    /// 対の Tag_File を持つか。
    has_tag: bool,
}

impl ImageSpec {
    fn image_name(&self) -> String {
        format!("{}.{}", self.basename, self.ext)
    }
    fn tag_name(&self) -> String {
        format!("{}.txt", self.basename)
    }
}

/// 画像集合のジェネレータ。basename をインデックスで一意化する。
fn images_strategy() -> impl Strategy<Value = Vec<ImageSpec>> {
    // 各要素: (拡張子インデックス, 対タグ有無)。枚数 0..=12。
    vec((0usize..EXTENSIONS.len(), any::<bool>()), 0..=12).prop_map(|specs| {
        specs
            .into_iter()
            .enumerate()
            .map(|(i, (ext_idx, has_tag))| ImageSpec {
                basename: format!("img{i:03}"),
                ext: EXTENSIONS[ext_idx],
                has_tag,
            })
            .collect()
    })
}

fn touch(path: &Path, content: &str) {
    fs::write(path, content.as_bytes()).unwrap();
}

fn exists(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

/// ディレクトリ直下の `.txt` ファイル名を集合で返す（余計な .txt 検出用）。
fn txt_files_in(dir: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.to_ascii_lowercase().ends_with(".txt") {
                set.insert(name);
            }
        }
    }
    set
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// 移動/コピー系での Image と Tag_File の対保存を検証する。
    #[test]
    fn move_copy_preserves_image_tag_pair(
        specs in images_strategy(),
        is_copy in any::<bool>(),
    ) {
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();

        // --- ソースを構築 ---
        for spec in &specs {
            touch(&src.path().join(spec.image_name()), "image-bytes");
            if spec.has_tag {
                touch(&src.path().join(spec.tag_name()), "tag-content");
            }
        }

        let op = if is_copy {
            SortingOperation::Copy
        } else {
            SortingOperation::Move
        };

        let report = sort_files(op, src.path(), dst.path()).unwrap();

        // 非衝突・非失敗を前提とした生成なので、全画像が成功（対単位）する。
        prop_assert_eq!(
            report.succeeded,
            specs.len(),
            "成功件数が画像枚数と一致しない: report={:?}",
            report
        );
        prop_assert_eq!(report.conflicted, 0, "衝突が発生した: {:?}", report);
        prop_assert_eq!(report.skipped, 0, "スキップが発生した: {:?}", report);

        // 宛先に現れてよい .txt は「対を持つ画像の tag_name」のみ。
        let mut expected_dst_txts: HashSet<String> = HashSet::new();

        for spec in &specs {
            let img = spec.image_name();
            let tag = spec.tag_name();

            // --- 宛先: Image は必ず存在する ---
            prop_assert!(
                exists(dst.path(), &img),
                "Image が宛先に存在しない: {}",
                img
            );

            if spec.has_tag {
                // 対保存: Image と同じ宛先に同一 basename の .txt が存在する。
                prop_assert!(
                    exists(dst.path(), &tag),
                    "対の Tag_File が宛先に存在せず対が壊れた: {}",
                    tag
                );
                expected_dst_txts.insert(tag.clone());
            } else {
                // 対を持たない画像について、余計な .txt が宛先に作られていない。
                prop_assert!(
                    !exists(dst.path(), &tag),
                    "対を持たない画像に .txt が宛先へ出現した: {}",
                    tag
                );
            }

            // --- 操作意味論: Move / Copy で元の残り方が異なる ---
            if is_copy {
                // Copy: 元は残る。
                prop_assert!(
                    exists(src.path(), &img),
                    "Copy 後に元 Image が消えた: {}",
                    img
                );
                if spec.has_tag {
                    prop_assert!(
                        exists(src.path(), &tag),
                        "Copy 後に元 Tag_File が消えた: {}",
                        tag
                    );
                }
            } else {
                // Move: 元は消える。
                prop_assert!(
                    !exists(src.path(), &img),
                    "Move 後に元 Image が残った: {}",
                    img
                );
                if spec.has_tag {
                    prop_assert!(
                        !exists(src.path(), &tag),
                        "Move 後に元 Tag_File が残った: {}",
                        tag
                    );
                }
            }
        }

        // 宛先の .txt 集合は「対を持つ画像の tag_name」だけと厳密に一致する
        // （余計な .txt が一切現れないことの全体確認）。
        let actual_dst_txts = txt_files_in(dst.path());
        prop_assert_eq!(
            actual_dst_txts,
            expected_dst_txts,
            "宛先の .txt 集合が期待と一致しない"
        );
    }
}
