// Feature: tag-editor, Property 21: 推論出力の値域とラベル対応
//
// Validates: Requirements 14.1
//
// *任意の* モデル出力について、各タグに付与される信頼度は 0.0〜1.0 の範囲に
// 収まり、結果タグ数はモデルのラベル定義数と一致する。
//
// 本テストは統合テスト（tests/）として、推論グルー
// tag_editor_core::services::inference_service::{run_labeled_inference,
// map_confidences_to_tags} を外部から検証する。SessionRunner トレイトは
// モック実装へ差し替え、実 ONNX ランタイムやモデルファイルを一切必要としない。
// モジュール本体は編集しない。

use proptest::prelude::*;

use tag_editor_core::models::{LabelDef, Tag, TagCategory};
use tag_editor_core::services::inference_service::{run_labeled_inference, SessionRunner};

/// 生成した確信度ベクトルをそのまま返すモック実行器。
///
/// 入力（前処理テンソル）は無視し、テストが与えた「モデル出力」を返すことで
/// 任意のモデル出力（範囲外・NaN を含む）を再現する。
struct MockRunner {
    output: Vec<f32>,
}

impl SessionRunner for MockRunner {
    fn run(&self, _input: &[f32]) -> tag_editor_core::AppResult<Vec<f32>> {
        Ok(self.output.clone())
    }
}

/// カテゴリのジェネレータ。Property 21 はカテゴリに依存しないが、実データを
/// 模して 3 種を均等に混ぜる。
fn category_strategy() -> impl Strategy<Value = TagCategory> {
    prop_oneof![
        Just(TagCategory::Rating),
        Just(TagCategory::General),
        Just(TagCategory::Character),
    ]
}

/// ラベル定義のジェネレータ。名前は書式・一意性に依存しないため単純な識別子。
fn label_strategy() -> impl Strategy<Value = LabelDef> {
    ("[a-z0-9_]{1,10}", category_strategy())
        .prop_map(|(name, category)| LabelDef { name, category })
}

/// (ラベル, 生の確信度) の対をラベル数に応じて生成する。両者を同時に生成する
/// ことで長さが常に一致し、proptest のシュリンクも自然に働く。
fn labels_and_confidences_strategy() -> impl Strategy<Value = (Vec<LabelDef>, Vec<f32>)> {
    prop::collection::vec((label_strategy(), raw_confidence_strategy()), 0..64).prop_map(|pairs| {
        let labels = pairs.iter().map(|(l, _)| l.clone()).collect();
        let confidences = pairs.iter().map(|(_, c)| *c).collect();
        (labels, confidences)
    })
}

/// 「任意のモデル出力」を模した確信度のジェネレータ。
///
/// 正常範囲（0.0〜1.0）に加え、範囲外の正/負・巨大値・NaN・無限大を意図的に
/// 混ぜ、クランプ経路（Property 21 の値域保証）を確実に踏ませる。
fn raw_confidence_strategy() -> impl Strategy<Value = f32> {
    prop_oneof![
        // 正常範囲。
        (0.0_f32..=1.0_f32),
        // 範囲外（正側）。
        (1.0_f32..1000.0_f32),
        // 範囲外（負側）。
        (-1000.0_f32..0.0_f32),
        // 特殊値。
        Just(f32::NAN),
        Just(f32::INFINITY),
        Just(f32::NEG_INFINITY),
        Just(0.0_f32),
        Just(1.0_f32),
    ]
}

proptest! {
    // 最小 100 ケースを十分に上回る。
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Property 21 本体: ラベル定義数と一致する長さの確信度ベクトルを与えたとき、
    /// 結果タグ数は labels.len() と一致し、各タグの信頼度は 0.0〜1.0 に収まる。
    #[test]
    fn output_range_and_label_correspondence(
        (labels, output) in labels_and_confidences_strategy(),
    ) {
        let runner = MockRunner { output };
        let tags: Vec<Tag> = run_labeled_inference(&runner, &labels, &[]).unwrap();

        // 結果タグ数はラベル定義数と一致する。
        prop_assert_eq!(tags.len(), labels.len(), "タグ数がラベル定義数と不一致");

        // 各タグは信頼度を持ち、0.0〜1.0 に収まる。
        for tag in &tags {
            let c = tag.confidence.expect("推論結果には信頼度が付く");
            prop_assert!(!c.is_nan(), "信頼度が NaN");
            prop_assert!((0.0..=1.0).contains(&c), "信頼度 {} が 0.0〜1.0 の範囲外", c);
        }

        // 各タグの本体は対応するラベル名と一致する（ラベル対応）。
        for (tag, label) in tags.iter().zip(labels.iter()) {
            prop_assert_eq!(&tag.body, &label.name, "タグ本体がラベル名と不一致");
        }
    }

    /// ガード検証（プロパティ本体ではない）: 出力次元がラベル定義数と一致しない
    /// 場合、run_labeled_inference はエラーを返す（黙って不整合結果を作らない）。
    #[test]
    fn dimension_mismatch_returns_error(
        labels in prop::collection::vec(label_strategy(), 1..32),
        // ラベル数と必ず異なる長さの出力を生成する（差分 1〜7 を上乗せ）。
        extra in prop::collection::vec(raw_confidence_strategy(), 1..8),
    ) {
        // labels と長さの異なる出力を作る（labels.len() + extra.len() != labels.len()）。
        let mut output = vec![0.5_f32; labels.len()];
        output.extend(extra);

        let runner = MockRunner { output };
        let result = run_labeled_inference(&runner, &labels, &[]);
        prop_assert!(result.is_err(), "次元不一致でもエラーにならなかった");
    }
}

/// 空ラベル（labels.len() == 0）の確定的検証。結果は空、エラーにならない。
#[test]
fn empty_labels_yield_empty_result() {
    let runner = MockRunner { output: vec![] };
    let tags = run_labeled_inference(&runner, &[], &[]).unwrap();
    assert!(tags.is_empty());
}
