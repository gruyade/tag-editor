// Feature: local-model-management, Property 16: 任意の画像の FilterOutcome について、Tag_File へ書き込んだ後に読み込むと、その内容は Adopted_Tags を規定の区切り（カンマ＋スペース）で連結した文字列と一致し、Discarded_Tags を含まない。
//
// Property 16: Adopted_Tags の Tag_File 書込一致
//
// inference_service の run_inference と同一経路で検証する:
//   1. logic::tag_format::render_tags(&adopted, false) で文字列化
//   2. services::tag_file::write_tag_file(image_path, &content) で書込
//   3. services::tag_file::read_tag_file(image_path) で読込
// 読み戻した内容が Adopted のタグ body をカンマ＋スペース（", "）で連結した
// 文字列と一致し、Discarded のタグ body を含まないことを検証する。
//
// render_tags(show_confidence=false) は全タグを body のみで ", " 連結する
// （logic::tag_format::render_tags 参照）。write/read は原子的置換による完全
// ラウンドトリップで、妥当な UTF-8 を書いて読み戻すため損失なく一致する
// （services::tag_file 参照）。
//
// 偽陽性回避: 「Discarded を含まない」検証は部分文字列 contains ではなく、
// 読み戻した内容を ", " で分割したタグトークン単位の完全一致で判定する。
// これにより adopted の連結タグ（例 "猫a"）が discarded body（例 "猫"）を
// 部分文字列として含んでも偽陽性にならない。加えてジェネレータで
// adopted / discarded の body 集合を素にし、body に区切り（','）や空白を
// 含めないため、トークン分割と包含否定判定が body 単位で厳密に成立する。
//
// Validates: Requirements 9.9

use std::collections::HashSet;

use proptest::prelude::*;
use tempfile::tempdir;

use tag_editor_core::logic::tag_format::render_tags;
use tag_editor_core::models::{FilterOutcome, Tag};
use tag_editor_core::services::tag_file::{read_tag_file, write_tag_file};

/// タグ body の生成器。
///
/// 区切り文字（','）・空白・制御文字を含まない非空トークンに限定する。
/// これにより連結文字列 `body0, body1, ...` の完全一致と、Discarded body の
/// 包含否定判定が body 単位で厳密に成立する。マルチバイト文字は許容する。
fn body_strategy() -> impl Strategy<Value = String> {
    // ASCII 英数記号（区切り/空白を除く）とマルチバイト代表文字を混ぜる。
    let ascii = prop::sample::select(vec![
        'a', 'b', 'Z', '0', '9', '_', '-', '.', '!', '?', '(', ')', ':',
    ]);
    let multibyte = prop::sample::select(vec!['猫', '髪', 'あ', '한', '🎨', '🐈']);
    let ch = prop_oneof![ascii, multibyte];
    prop::collection::vec(ch, 1..12).prop_map(|v| v.into_iter().collect())
}

/// 信頼度は任意（Some/None）の Tag 生成器。
fn tag_strategy() -> impl Strategy<Value = Tag> {
    (body_strategy(), prop::option::of(0.0f32..=1.0f32)).prop_map(|(body, conf)| match conf {
        Some(c) => Tag::with_confidence(body, c),
        None => Tag::new(body),
    })
}

/// adopted / discarded の body 集合が素になる FilterOutcome 生成器。
///
/// タグ列を生成後、body の重複を除去し、さらに adopted に現れた body を
/// discarded から取り除くことで両集合を素にする。
fn outcome_strategy() -> impl Strategy<Value = FilterOutcome> {
    (
        prop::collection::vec(tag_strategy(), 0..8),
        prop::collection::vec(tag_strategy(), 0..8),
    )
        .prop_map(|(adopted_raw, discarded_raw)| {
            // adopted: body 重複を除去。
            let mut seen: HashSet<String> = HashSet::new();
            let adopted: Vec<Tag> = adopted_raw
                .into_iter()
                .filter(|t| seen.insert(t.body.clone()))
                .collect();

            // discarded: body 重複除去 かつ adopted の body を除外。
            let mut discarded: Vec<Tag> = Vec::new();
            let mut disc_seen: HashSet<String> = HashSet::new();
            for t in discarded_raw {
                if seen.contains(&t.body) {
                    continue;
                }
                if disc_seen.insert(t.body.clone()) {
                    discarded.push(t);
                }
            }

            FilterOutcome { adopted, discarded }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 16: Adopted を書込→読込した内容は Adopted body の ", " 連結と
    /// 一致し、Discarded body を含まない（要件 9.9）。
    #[test]
    fn adopted_written_matches_and_excludes_discarded(outcome in outcome_strategy()) {
        let dir = tempdir().expect("一時ディレクトリ作成");
        let image = dir.path().join("img.png");

        // run_inference と同一経路: render_tags(show_confidence=false) で文字列化。
        let content = render_tags(&outcome.adopted, false);

        // Tag_File（img.txt）へ書込。
        write_tag_file(&image, &content).expect("書き込み成功");

        // 読み戻す。
        let read = read_tag_file(&image).expect("読み込み成功");
        prop_assert!(read.exists, "書込後は Tag_File が存在するはず");

        // 期待値: Adopted の body をカンマ＋スペースで連結した文字列。
        let expected = outcome
            .adopted
            .iter()
            .map(|t| t.body.clone())
            .collect::<Vec<_>>()
            .join(", ");

        prop_assert_eq!(
            &read.content,
            &expected,
            "書込一致不成立: 読込={:?} 期待={:?}",
            read.content,
            expected
        );

        // Discarded の body を含まないこと。
        // 部分文字列 contains では、adopted の連結タグ（例 "猫a"）が discarded
        // body（例 "猫"）を部分文字列として含み偽陽性になる。よって読み戻した
        // 内容を ", " で分割したタグトークン単位の完全一致で判定する。
        let written_tokens: HashSet<&str> = if read.content.is_empty() {
            HashSet::new()
        } else {
            read.content.split(", ").collect()
        };
        // Discarded の body はタグトークンとして現れない（adopted と body 集合が素なので成立）。
        for d in &outcome.discarded {
            prop_assert!(
                !written_tokens.contains(d.body.as_str()),
                "Discarded body がタグとして含まれている: discarded={:?} tokens={:?}",
                d.body,
                written_tokens
            );
        }
    }
}
