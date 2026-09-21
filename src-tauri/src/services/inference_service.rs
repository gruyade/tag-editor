//! InferenceService（タスク 16.1）: 前処理と ort セッション推論。
//!
//! 本モジュールは推論エンジンの中核を 2 段に分けて提供する。
//!
//! 1. [`preprocess_image`]: 画像を入力サイズの正方形へリサイズし、チャンネル順
//!    （BGR/RGB、モデルメタから解決）に従って画素値を並べた `f32` テンソルを
//!    生成する純粋関数。実 ONNX ランタイムを一切必要とせず単体テスト可能。
//! 2. 推論グルー（[`infer_confidences`] / [`run_labeled_inference`]）: 前処理済み
//!    入力を [`SessionRunner`] へ渡して確信度ベクトルを得て、ラベル定義
//!    （[`LabelDef`]）へ 1 対 1 で対応付ける。
//!
//! 実 `ort::Session::run` は共有ライブラリ（`load-dynamic`）を要するため、その
//! 呼び出しは [`SessionRunner`] トレイト境界の背後に隔離する。単体テストは
//! モック実装を用い、実モデルファイル無しで前処理とラベル対応付けを検証する。
//!
//! _Requirements: 14.1_
//!
//! # WD14 前処理の規約（本実装が採用する取り決め）
//!
//! WD14 系タガー（SmilingWolf 系）の一般的な入力規約に合わせる:
//!
//! - 入力は `input_size × input_size`（例 448）の正方形。アスペクト比は
//!   単純リサイズ（[`image::imageops::FilterType::Triangle`]）で合わせる。
//! - 画素値は **0〜255 のレンジをそのまま `f32`** として並べる（`/255.0` の
//!   正規化は行わない）。WD14 の ONNX グラフは 0〜255 入力を前提とするため。
//! - チャンネル順は [`ChannelOrder`] に従う。`Bgr`（WD14 既定）なら B,G,R、
//!   `Rgb` なら R,G,B の順で各画素 3 要素を格納する。
//! - レイアウトは **NHWC**（行優先で `[height][width][channel]`）。1 画像分の
//!   平坦化ベクトルを返し、バッチ次元の付与は呼び出し側（後続タスク）に委ねる。
//!
//! この規約はドキュメントとして固定し、モデルメタが異なる規約を要求する場合は
//! [`ChannelOrder`] や `input_size` を通じて表現する。

use crate::error::{AppError, AppResult};
use crate::models::{ChannelOrder, ImageTagResult, LabelDef, Tag};
use image::DynamicImage;

/// 1 画像を推論入力テンソル（NHWC・0〜255・指定チャンネル順）へ前処理する。
///
/// `input_size × input_size` へリサイズし、各画素を [`ChannelOrder`] の順で
/// 3 要素ずつ格納した平坦な `f32` ベクトルを返す。
///
/// # 戻り値の長さ
///
/// `input_size * input_size * 3`。`input_size == 0` の場合は空ベクトル。
///
/// # 引数
///
/// - `img`: 入力画像。
/// - `input_size`: 出力の一辺（例 448）。
/// - `channel_order`: 画素内のチャンネル並び（BGR/RGB）。
pub fn preprocess_image(
    img: &DynamicImage,
    input_size: u32,
    channel_order: ChannelOrder,
) -> Vec<f32> {
    if input_size == 0 {
        return Vec::new();
    }

    // RGBA8 を経由し、input_size 正方形へリサイズ。
    let resized = img
        .resize_exact(
            input_size,
            input_size,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8();

    let mut tensor = Vec::with_capacity((input_size * input_size * 3) as usize);
    // NHWC: 行優先で走査し、各画素をチャンネル順に格納。
    for pixel in resized.pixels() {
        let [r, g, b, _a] = pixel.0;
        let (r, g, b) = (r as f32, g as f32, b as f32);
        match channel_order {
            ChannelOrder::Bgr => {
                tensor.push(b);
                tensor.push(g);
                tensor.push(r);
            }
            ChannelOrder::Rgb => {
                tensor.push(r);
                tensor.push(g);
                tensor.push(b);
            }
        }
    }
    tensor
}

/// セッション実行の抽象境界。
///
/// 実 `ort::Session::run` はこのトレイトの背後に隔離し、単体テストでは
/// モック実装へ差し替える。実装は前処理済み入力（1 画像分の平坦テンソル）を
/// 受け取り、ラベル定義に対応する確信度ベクトルを返す。
///
/// # 契約
///
/// - 戻り値の長さはモデルのラベル定義数（`labels.len()`）と一致すること。
/// - 各要素は 0.0〜1.0 の確信度であること。
///
/// 上記契約は [`run_labeled_inference`] 側でも検証し、破られた場合は
/// [`AppError`] を返す（Property 21 の保証）。
pub trait SessionRunner {
    /// 前処理済み入力に対し推論を実行し、確信度ベクトルを返す。
    fn run(&self, input: &[f32]) -> AppResult<Vec<f32>>;
}

/// 確信度ベクトルをラベル定義へ対応付け、[`Tag`] 列を生成する。
///
/// `confidences[i]` を `labels[i]` に対応付け、`Tag { body: labels[i].name,
/// confidence: Some(clamp(confidences[i], 0.0, 1.0)) }` を生成する。
///
/// # エラー
///
/// `confidences.len() != labels.len()` の場合は [`AppErrorKind::ModelLoad`] を
/// 返す（ラベル定義と出力次元の不整合）。
///
/// # 値域
///
/// 各確信度は 0.0〜1.0 にクランプする。NaN は 0.0 として扱う。これにより
/// 生成される全 [`Tag`] の `confidence` が 0.0〜1.0 に収まる（Property 21）。
///
/// [`AppErrorKind::ModelLoad`]: crate::error::AppErrorKind::ModelLoad
pub fn map_confidences_to_tags(labels: &[LabelDef], confidences: &[f32]) -> AppResult<Vec<Tag>> {
    if confidences.len() != labels.len() {
        return Err(AppError::model_load(format!(
            "推論出力の次元 {} がラベル定義数 {} と一致しない",
            confidences.len(),
            labels.len()
        )));
    }
    let tags = labels
        .iter()
        .zip(confidences.iter())
        .map(|(label, &conf)| {
            let clamped = if conf.is_nan() { 0.0 } else { conf.clamp(0.0, 1.0) };
            Tag::with_confidence(label.name.clone(), clamped)
        })
        .collect();
    Ok(tags)
}

/// 前処理済み入力に対し推論を実行し、ラベル対応済みの [`Tag`] 列を返す。
///
/// [`SessionRunner`] 経由で確信度ベクトルを得て [`map_confidences_to_tags`] で
/// ラベルへ対応付ける。結果タグ数は必ず `labels.len()` と一致し、各確信度は
/// 0.0〜1.0 に収まる（Property 21 の対象）。
pub fn run_labeled_inference(
    runner: &dyn SessionRunner,
    labels: &[LabelDef],
    input: &[f32],
) -> AppResult<Vec<Tag>> {
    let confidences = runner.run(input)?;
    map_confidences_to_tags(labels, &confidences)
}

/// 画像・入力サイズ・チャンネル順・ラベル定義から 1 画像分の推論を実行する。
///
/// 前処理（[`preprocess_image`]）と推論グルー（[`run_labeled_inference`]）を
/// 連結し、[`ImageTagResult`] を組み立てる。閾値適用前の全確信度を保持する
/// （採用は呼び出し側で判定、要件 14.4 は別タスク）。
///
/// # 引数
///
/// - `runner`: セッション実行の抽象（実 ort もモックも可）。
/// - `image_path`: 結果に記録する画像パス。
/// - `img`: 入力画像。
/// - `input_size` / `channel_order` / `labels`: モデルメタ由来の前処理・対応情報。
pub fn infer_image(
    runner: &dyn SessionRunner,
    image_path: impl Into<String>,
    img: &DynamicImage,
    input_size: u32,
    channel_order: ChannelOrder,
    labels: &[LabelDef],
) -> AppResult<ImageTagResult> {
    let input = preprocess_image(img, input_size, channel_order);
    let tags = run_labeled_inference(runner, labels, &input)?;
    Ok(ImageTagResult {
        image_path: image_path.into(),
        tags,
    })
}

/// 実 `ort::Session` を用いた [`SessionRunner`] 実装。
///
/// 実行には ONNX Runtime 共有ライブラリ（`load-dynamic`）が必要なため、単体
/// テストからは使用しない。前処理済み入力を NHWC の 4 次元テンソル
/// `[1, input_size, input_size, 3]` として名前付き入力へ渡し、単一の出力
/// テンソルを確信度ベクトルとして取り出す。
///
/// 入出力名やレイアウトはモデルにより異なりうるため、本実装は「単一入力・単一
/// 出力・NHWC」という WD14 系の一般形を前提とする最小実装であり、後続タスク
/// （16.3 のバッチ実行）で拡張する余地を残す。
pub struct OrtSessionRunner<'a> {
    /// 実行対象セッション。`ort::Session::run` が `&mut self` を要するため
    /// [`RefCell`] で内部可変性を与え、[`SessionRunner::run`] の `&self`
    /// 契約を保つ。
    pub session: std::cell::RefCell<&'a mut ort::session::Session>,
    /// 入力の一辺サイズ（NHWC のテンソル形状構築に用いる）。
    pub input_size: u32,
}

impl<'a> OrtSessionRunner<'a> {
    /// 実セッションと入力サイズから実行器を生成する。
    pub fn new(session: &'a mut ort::session::Session, input_size: u32) -> Self {
        Self {
            session: std::cell::RefCell::new(session),
            input_size,
        }
    }
}

impl SessionRunner for OrtSessionRunner<'_> {
    fn run(&self, input: &[f32]) -> AppResult<Vec<f32>> {
        use ort::value::TensorRef;

        let side = self.input_size as usize;
        let expected = side * side * 3;
        if input.len() != expected {
            return Err(AppError::invalid_input(format!(
                "前処理テンソル長 {} が期待値 {} と一致しない",
                input.len(),
                expected
            )));
        }

        let mut session = self.session.borrow_mut();

        // NHWC: [batch=1, H, W, C=3]。
        let shape = [1_i64, side as i64, side as i64, 3];
        let tensor = TensorRef::from_array_view((shape, input))
            .map_err(|e| AppError::model_load(format!("入力テンソル構築に失敗: {e}")))?;

        // 単一入力・単一出力を前提に、最初の入力名へ束ねて実行する。
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| AppError::model_load("モデルに入力が定義されていない"))?;

        let outputs = session
            .run(ort::inputs![input_name.as_str() => tensor])
            .map_err(|e| AppError::model_load(format!("推論実行に失敗: {e}")))?;

        // 最初の出力テンソルを確信度ベクトルとして取り出す。
        let output = outputs
            .iter()
            .next()
            .ok_or_else(|| AppError::model_load("モデル出力が空"))?;
        let (_shape, data) = output
            .1
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::model_load(format!("出力テンソル抽出に失敗: {e}")))?;

        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod preprocess_tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// 全画素が単色の 2x2 画像を生成する。
    fn solid_image(r: u8, g: u8, b: u8) -> DynamicImage {
        let mut img = RgbImage::new(2, 2);
        for pixel in img.pixels_mut() {
            *pixel = Rgb([r, g, b]);
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn tensor_length_matches_input_size() {
        let img = solid_image(10, 20, 30);
        let tensor = preprocess_image(&img, 4, ChannelOrder::Rgb);
        assert_eq!(tensor.len(), 4 * 4 * 3);
    }

    #[test]
    fn rgb_order_places_channels_r_g_b() {
        // 単色画像はリサイズしても同色。先頭画素で並びを検証。
        let img = solid_image(10, 20, 30);
        let tensor = preprocess_image(&img, 2, ChannelOrder::Rgb);
        assert_eq!(&tensor[0..3], &[10.0, 20.0, 30.0]);
    }

    #[test]
    fn bgr_order_places_channels_b_g_r() {
        let img = solid_image(10, 20, 30);
        let tensor = preprocess_image(&img, 2, ChannelOrder::Bgr);
        assert_eq!(&tensor[0..3], &[30.0, 20.0, 10.0]);
    }

    #[test]
    fn values_are_in_0_255_range_not_normalized() {
        // 255 が 1.0 に正規化されず 255.0 のまま入ることを確認。
        let img = solid_image(255, 128, 0);
        let tensor = preprocess_image(&img, 2, ChannelOrder::Rgb);
        assert_eq!(&tensor[0..3], &[255.0, 128.0, 0.0]);
    }

    #[test]
    fn zero_input_size_yields_empty() {
        let img = solid_image(1, 2, 3);
        assert!(preprocess_image(&img, 0, ChannelOrder::Rgb).is_empty());
    }

    #[test]
    fn resizes_non_square_input_to_square() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(7, 3));
        let tensor = preprocess_image(&img, 5, ChannelOrder::Bgr);
        assert_eq!(tensor.len(), 5 * 5 * 3);
    }
}

#[cfg(test)]
mod inference_mapping_tests {
    use super::*;
    use crate::error::AppErrorKind;
    use crate::models::TagCategory;

    /// 固定の確信度ベクトルを返すモック実行器。
    struct MockRunner {
        output: Vec<f32>,
    }

    impl SessionRunner for MockRunner {
        fn run(&self, _input: &[f32]) -> AppResult<Vec<f32>> {
            Ok(self.output.clone())
        }
    }

    fn labels(names: &[&str]) -> Vec<LabelDef> {
        names
            .iter()
            .map(|n| LabelDef {
                name: (*n).to_string(),
                category: TagCategory::General,
            })
            .collect()
    }

    #[test]
    fn yields_one_tag_per_label() {
        let labels = labels(&["cat", "dog", "bird"]);
        let runner = MockRunner {
            output: vec![0.9, 0.1, 0.5],
        };
        let tags = run_labeled_inference(&runner, &labels, &[]).unwrap();
        assert_eq!(tags.len(), labels.len());
        let bodies: Vec<&str> = tags.iter().map(|t| t.body.as_str()).collect();
        assert_eq!(bodies, vec!["cat", "dog", "bird"]);
    }

    #[test]
    fn confidences_are_within_0_1() {
        let labels = labels(&["a", "b", "c"]);
        let runner = MockRunner {
            output: vec![0.0, 0.5, 1.0],
        };
        let tags = run_labeled_inference(&runner, &labels, &[]).unwrap();
        for tag in &tags {
            let c = tag.confidence.expect("推論結果には信頼度が付く");
            assert!((0.0..=1.0).contains(&c));
        }
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let labels = labels(&["a", "b"]);
        // 範囲外・NaN を含む出力もクランプで 0.0〜1.0 に収める。
        let runner = MockRunner {
            output: vec![1.5, -0.3],
        };
        let tags = run_labeled_inference(&runner, &labels, &[]).unwrap();
        assert_eq!(tags[0].confidence, Some(1.0));
        assert_eq!(tags[1].confidence, Some(0.0));
    }

    #[test]
    fn nan_maps_to_zero() {
        let labels = labels(&["a"]);
        let runner = MockRunner {
            output: vec![f32::NAN],
        };
        let tags = run_labeled_inference(&runner, &labels, &[]).unwrap();
        assert_eq!(tags[0].confidence, Some(0.0));
    }

    #[test]
    fn dimension_mismatch_is_model_load_error() {
        let labels = labels(&["a", "b", "c"]);
        let runner = MockRunner {
            output: vec![0.5, 0.5],
        };
        let err = run_labeled_inference(&runner, &labels, &[]).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::ModelLoad);
    }

    #[test]
    fn infer_image_produces_result_with_path_and_all_labels() {
        use image::{Rgb, RgbImage};
        let mut rgb = RgbImage::new(2, 2);
        for p in rgb.pixels_mut() {
            *p = Rgb([10, 20, 30]);
        }
        let img = DynamicImage::ImageRgb8(rgb);
        let labels = labels(&["x", "y"]);
        let runner = MockRunner {
            output: vec![0.2, 0.8],
        };
        let result = infer_image(&runner, "C:/imgs/a.png", &img, 4, ChannelOrder::Bgr, &labels)
            .unwrap();
        assert_eq!(result.image_path, "C:/imgs/a.png");
        assert_eq!(result.tags.len(), labels.len());
    }
}

// ============================================================================
// タスク 16.3: run_inference（単一/バッチ・並列前処理・進捗・キャンセル）
// ============================================================================
//
// 本節は、複数画像に対する推論オーケストレーションを提供する。純粋ロジック
// （[`crate::logic::inference_aux`] の mp4 除外・Batch_Size 解決・バッチ分割・
// 閾値採用）と Tag_File 書込（[`crate::services::tag_file::write_tag_file`]）を
// 結線し、以下の振る舞いを実現する。
//
// - mp4 を推論対象から除外する（要件 14.8）。
// - Batch_Size を解決し（既定 8・範囲 1〜64）、対象列を Batch_Size 単位の
//   バッチへ分割する（要件 17.2, 17.3, 17.4, 17.5）。
// - バッチ内は rayon で並列に画像読込＋前処理する（要件 17.1）。
// - 各画像を [`SessionRunner`] へ入力して確信度を得て、閾値以上のタグを採用
//   （要件 14.1, 14.4）、採用タグを `, ` 連結で同名 Tag_File へ書込（既存は
//   上書き、要件 14.5, 14.6）。
// - 画像読込不可・個別推論失敗・書込失敗はスキップして残りを継続し、失敗件数を
//   記録する（要件 14.3, 14.7, 17.8）。
// - 1 件処理するごとに進捗（処理済み/総数）を通知する（要件 17.6）。
// - キャンセルフラグをバッチ境界で確認し、要求されていれば未処理を中止する
//   （要件 17.7）。

use crate::logic::inference_aux::{
    adopt_by_threshold, exclude_videos, resolve_batch_size, split_into_batches,
};
use crate::logic::tag_format::render_tags;
use crate::models::Progress;
use std::sync::atomic::{AtomicBool, Ordering};

/// [`run_inference`] の結果。
///
/// 単一/バッチ推論の集計結果を保持する。UI へは件数と付随メッセージを提示する。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InferResult {
    /// Tag_File 書込まで成功した件数。
    pub succeeded: usize,
    /// 読込不可・推論失敗・書込失敗でスキップした件数（要件 14.3, 14.7, 17.8）。
    pub failed: usize,
    /// mp4 除外により推論対象から外した件数（要件 14.8）。
    pub excluded: usize,
    /// キャンセルにより未処理のまま中止した件数（要件 17.7）。
    pub cancelled: usize,
    /// 付随メッセージ（失敗理由など）。
    pub messages: Vec<String>,
}

/// 複数画像に対して推論を実行し、採用タグを同名 Tag_File へ書き込む。
///
/// # 引数
///
/// - `runner`: セッション実行の抽象（実 ort もモックも可）。1 画像分の前処理済み
///   テンソルを受け取り確信度ベクトルを返す。
/// - `image_paths`: 対象 Image_File のパス列。mp4 は除外される（要件 14.8）。
/// - `threshold`: 採用の下限信頼度（0.0〜1.0）。これ以上のタグのみ採用（要件 14.4）。
/// - `batch_size`: Batch_Size（未指定は既定 8、範囲 1〜64、範囲外は丸め）
///   （要件 17.3, 17.4, 17.5）。
/// - `labels`: ラベル定義。確信度ベクトルと 1 対 1 対応する。
/// - `input_size` / `channel_order`: 前処理のパラメータ（モデルメタ由来）。
/// - `operation_id`: 進捗イベントに載せる処理識別子。
/// - `cancel`: キャンセル要求フラグ。バッチ境界で確認する（要件 17.7）。
/// - `progress`: 1 件処理ごとに `処理済み/総数` を通知するコールバック（要件 17.6）。
///
/// # 並列前処理（要件 17.1）
///
/// 各バッチ内の画像読込＋前処理は rayon の並列イテレータで実行する。個々の
/// 画像読込結果（成功/失敗）は入力順を保ったまま集約し、以降の逐次処理
/// （推論・書込・進捗通知）へ渡す。これにより順序と決定的な進捗通知を保つ。
///
/// # 進捗の総数
///
/// 進捗の総数は mp4 除外後の対象件数とする。除外された mp4 は総数に含めない。
///
/// # 戻り値
///
/// [`InferResult`]。成功/失敗/除外/キャンセル件数と付随メッセージを保持する。
#[allow(clippy::too_many_arguments)]
pub fn run_inference(
    runner: &dyn SessionRunner,
    image_paths: &[String],
    threshold: f32,
    batch_size: Option<u32>,
    labels: &[LabelDef],
    input_size: u32,
    channel_order: ChannelOrder,
    operation_id: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> InferResult {
    use rayon::prelude::*;

    // mp4 を推論対象から除外する（要件 14.8）。
    let targets = exclude_videos(image_paths);
    let excluded = image_paths.len() - targets.len();

    let total = targets.len();
    let mut result = InferResult {
        excluded,
        ..Default::default()
    };

    // Batch_Size を解決し、対象をバッチへ分割する（要件 17.2, 17.3, 17.4, 17.5）。
    let resolved_batch = resolve_batch_size(batch_size);
    let batches = split_into_batches(&targets, resolved_batch);

    let mut done = 0usize;

    for batch in &batches {
        // キャンセルはバッチ境界で確認する（要件 17.7）。
        if cancel.load(Ordering::Relaxed) {
            // 未処理（このバッチ以降）を中止し、件数を記録する。
            result.cancelled = total - done;
            result
                .messages
                .push(format!("キャンセルにより {} 件を中止", result.cancelled));
            break;
        }

        // バッチ内の画像読込＋前処理を並列実行する（要件 17.1）。
        // 入力順を保ったまま (パス, 前処理結果) を集約する。
        let preprocessed: Vec<(String, Result<Vec<f32>, String>)> = batch
            .par_iter()
            .map(|path| {
                let loaded = image::open(path)
                    .map(|img| preprocess_image(&img, input_size, channel_order))
                    .map_err(|e| format!("画像読込に失敗: {e}"));
                (path.clone(), loaded)
            })
            .collect();

        // 逐次に推論・採用・書込・進捗通知を行う（決定的な進捗のため順序保存）。
        for (path, pre) in preprocessed {
            match pre {
                // 読込/前処理失敗はスキップ＋記録（要件 14.7, 17.8）。
                Err(msg) => {
                    result.failed += 1;
                    result.messages.push(format!("{path}: {msg}"));
                }
                Ok(input) => match run_labeled_inference(runner, labels, &input) {
                    // 個別推論失敗はスキップ＋記録（要件 14.3, 17.8）。
                    Err(e) => {
                        result.failed += 1;
                        result.messages.push(format!("{path}: 推論に失敗: {e}"));
                    }
                    Ok(tags) => {
                        // 閾値以上のタグのみ採用（要件 14.4）。
                        let adopted = adopt_by_threshold(&tags, threshold);
                        // 採用タグを `, ` 連結で描画（信頼度は付けず本体のみ）。
                        let content = render_tags(&adopted, false);
                        // 採用タグを同名 Tag_File へ書込（既存は上書き）（要件 14.5, 14.6）。
                        match crate::services::tag_file::write_tag_file(&path, &content) {
                            Ok(()) => result.succeeded += 1,
                            Err(e) => {
                                result.failed += 1;
                                result.messages.push(format!("{path}: 書込に失敗: {e}"));
                            }
                        }
                    }
                },
            }

            // 1 件処理ごとに進捗を通知する（要件 17.6）。
            done += 1;
            progress(Progress {
                operation_id: operation_id.to_string(),
                done,
                total,
            });
        }
    }

    result
}

#[cfg(test)]
mod run_inference_tests {
    use super::*;
    use crate::models::TagCategory;
    use image::{Rgb, RgbImage};
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use tempfile::tempdir;

    /// ラベルごとに固定の確信度を返すモック実行器。
    struct MockRunner {
        output: Vec<f32>,
    }

    impl SessionRunner for MockRunner {
        fn run(&self, _input: &[f32]) -> AppResult<Vec<f32>> {
            Ok(self.output.clone())
        }
    }

    /// 常に推論失敗するモック実行器。
    struct FailRunner;

    impl SessionRunner for FailRunner {
        fn run(&self, _input: &[f32]) -> AppResult<Vec<f32>> {
            Err(AppError::model_load("mock failure"))
        }
    }

    fn labels(names: &[&str]) -> Vec<LabelDef> {
        names
            .iter()
            .map(|n| LabelDef {
                name: (*n).to_string(),
                category: TagCategory::General,
            })
            .collect()
    }

    /// 単色 2x2 の PNG 画像を実ファイルとして作成する。
    fn write_image(path: &Path) {
        let mut img = RgbImage::new(2, 2);
        for p in img.pixels_mut() {
            *p = Rgb([120, 130, 140]);
        }
        img.save(path).unwrap();
    }

    /// 進捗を記録しないダミーコールバック。
    fn noop_progress() -> impl FnMut(Progress) {
        |_p: Progress| {}
    }

    #[test]
    fn mp4_files_are_excluded_from_processing() {
        let dir = tempdir().unwrap();
        let png = dir.path().join("a.png");
        write_image(&png);
        let mp4 = dir.path().join("b.mp4");
        std::fs::write(&mp4, b"not a real video").unwrap();

        let runner = MockRunner {
            output: vec![0.9, 0.9],
        };
        let labels = labels(&["cat", "dog"]);
        let cancel = AtomicBool::new(false);
        let mut progress = noop_progress();

        let paths = vec![
            png.to_string_lossy().into_owned(),
            mp4.to_string_lossy().into_owned(),
        ];
        let result = run_inference(
            &runner,
            &paths,
            0.5,
            None,
            &labels,
            4,
            ChannelOrder::Bgr,
            "op1",
            &cancel,
            &mut progress,
        );

        assert_eq!(result.excluded, 1);
        assert_eq!(result.succeeded, 1);
        // mp4 の Tag_File は生成されない。
        assert!(!dir.path().join("b.txt").exists());
        // png の Tag_File は生成される。
        assert!(dir.path().join("a.txt").exists());
    }

    #[test]
    fn adopted_tags_written_to_tag_file_overwriting_existing() {
        let dir = tempdir().unwrap();
        let png = dir.path().join("img.png");
        write_image(&png);
        // 既存の Tag_File を用意し、上書きを確認する（要件 14.6）。
        let tag_path = dir.path().join("img.txt");
        std::fs::write(&tag_path, "old, stale, content").unwrap();

        let runner = MockRunner {
            output: vec![0.9, 0.2, 0.8],
        };
        let labels = labels(&["cat", "dog", "bird"]);
        let cancel = AtomicBool::new(false);
        let mut progress = noop_progress();

        let paths = vec![png.to_string_lossy().into_owned()];
        let result = run_inference(
            &runner,
            &paths,
            0.5,
            None,
            &labels,
            4,
            ChannelOrder::Bgr,
            "op1",
            &cancel,
            &mut progress,
        );

        assert_eq!(result.succeeded, 1);
        let written = std::fs::read_to_string(&tag_path).unwrap();
        // 閾値 0.5 以上の cat(0.9), bird(0.8) のみ採用、dog(0.2) は不採用。
        assert_eq!(written, "cat, bird");
        assert!(!written.contains("old"));
    }

    #[test]
    fn threshold_filters_tags() {
        let dir = tempdir().unwrap();
        let png = dir.path().join("img.png");
        write_image(&png);

        let runner = MockRunner {
            output: vec![0.95, 0.4, 0.6],
        };
        let labels = labels(&["a", "b", "c"]);
        let cancel = AtomicBool::new(false);
        let mut progress = noop_progress();

        let paths = vec![png.to_string_lossy().into_owned()];
        run_inference(
            &runner,
            &paths,
            0.9,
            None,
            &labels,
            4,
            ChannelOrder::Bgr,
            "op1",
            &cancel,
            &mut progress,
        );

        let written = std::fs::read_to_string(dir.path().join("img.txt")).unwrap();
        // 閾値 0.9 以上は a(0.95) のみ。
        assert_eq!(written, "a");
    }

    #[test]
    fn per_item_load_failure_is_skipped_and_counted_while_others_succeed() {
        let dir = tempdir().unwrap();
        // 有効画像。
        let good = dir.path().join("good.png");
        write_image(&good);
        // 画像として読めない壊れたファイル（拡張子は png だが中身は不正）。
        let bad = dir.path().join("bad.png");
        std::fs::write(&bad, b"this is not a valid image").unwrap();

        let runner = MockRunner {
            output: vec![0.9, 0.9],
        };
        let labels = labels(&["x", "y"]);
        let cancel = AtomicBool::new(false);
        let mut progress = noop_progress();

        let paths = vec![
            good.to_string_lossy().into_owned(),
            bad.to_string_lossy().into_owned(),
        ];
        let result = run_inference(
            &runner,
            &paths,
            0.5,
            None,
            &labels,
            4,
            ChannelOrder::Bgr,
            "op1",
            &cancel,
            &mut progress,
        );

        assert_eq!(result.succeeded, 1);
        assert_eq!(result.failed, 1);
        // 有効画像は書込成功、壊れたファイルは Tag_File 未生成。
        assert!(dir.path().join("good.txt").exists());
        assert!(!dir.path().join("bad.txt").exists());
    }

    #[test]
    fn per_item_inference_failure_is_skipped_and_counted() {
        let dir = tempdir().unwrap();
        let png = dir.path().join("img.png");
        write_image(&png);

        let runner = FailRunner;
        let labels = labels(&["a", "b"]);
        let cancel = AtomicBool::new(false);
        let mut progress = noop_progress();

        let paths = vec![png.to_string_lossy().into_owned()];
        let result = run_inference(
            &runner,
            &paths,
            0.5,
            None,
            &labels,
            4,
            ChannelOrder::Bgr,
            "op1",
            &cancel,
            &mut progress,
        );

        assert_eq!(result.succeeded, 0);
        assert_eq!(result.failed, 1);
        assert!(!dir.path().join("img.txt").exists());
    }

    #[test]
    fn cancel_stops_processing_remaining() {
        let dir = tempdir().unwrap();
        // 2 バッチに分かれるよう batch_size=1 で 3 枚用意する。
        let mut paths = Vec::new();
        for i in 0..3 {
            let p = dir.path().join(format!("img{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner {
            output: vec![0.9],
        };
        let labels = labels(&["a"]);
        // 最初のバッチ処理後にキャンセルを立てる。
        let cancel = AtomicBool::new(false);
        let mut processed = 0usize;
        let mut progress = |_p: Progress| {
            processed += 1;
            if processed == 1 {
                cancel.store(true, Ordering::Relaxed);
            }
        };

        let result = run_inference(
            &runner,
            &paths,
            0.5,
            Some(1),
            &labels,
            4,
            ChannelOrder::Bgr,
            "op1",
            &cancel,
            &mut progress,
        );

        // 1 件目は成功、残り 2 件はキャンセルで中止。
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.cancelled, 2);
        assert!(dir.path().join("img0.txt").exists());
        assert!(!dir.path().join("img1.txt").exists());
        assert!(!dir.path().join("img2.txt").exists());
    }

    #[test]
    fn progress_callback_invoked_with_increasing_done_up_to_total() {
        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..3 {
            let p = dir.path().join(format!("p{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }
        // mp4 を混ぜても総数には含まれない。
        let mp4 = dir.path().join("v.mp4");
        std::fs::write(&mp4, b"x").unwrap();
        paths.push(mp4.to_string_lossy().into_owned());

        let runner = MockRunner {
            output: vec![0.9],
        };
        let labels = labels(&["a"]);
        let cancel = AtomicBool::new(false);

        let mut events: Vec<(usize, usize)> = Vec::new();
        let mut progress = |p: Progress| {
            assert_eq!(p.operation_id, "op-progress");
            events.push((p.done, p.total));
        };

        run_inference(
            &runner,
            &paths,
            0.5,
            Some(2),
            &labels,
            4,
            ChannelOrder::Bgr,
            "op-progress",
            &cancel,
            &mut progress,
        );

        // 総数は mp4 を除いた 3。done は 1,2,3 と単調増加。
        assert_eq!(events, vec![(1, 3), (2, 3), (3, 3)]);
    }

    // ------------------------------------------------------------------
    // タスク 16.4: 複数バッチにまたがる部分失敗継続・読込不可・キャンセル
    // ------------------------------------------------------------------

    /// 呼び出し順に沿って成功/失敗を切り替えるモック実行器。
    ///
    /// `fail_on_call[n]` が true の呼び出し（0 始まり）は推論失敗を返し、
    /// それ以外は固定確信度を返す。`run_labeled_inference` は入力順に逐次
    /// 呼ばれるため、成功と失敗をインターリーブして再現できる。
    struct SequencedRunner {
        output: Vec<f32>,
        fail_on_call: Vec<bool>,
        calls: std::cell::RefCell<usize>,
    }

    impl SessionRunner for SequencedRunner {
        fn run(&self, _input: &[f32]) -> AppResult<Vec<f32>> {
            let idx = {
                let mut c = self.calls.borrow_mut();
                let cur = *c;
                *c += 1;
                cur
            };
            if self.fail_on_call.get(idx).copied().unwrap_or(false) {
                Err(AppError::model_load(format!("mock failure at call {idx}")))
            } else {
                Ok(self.output.clone())
            }
        }
    }

    /// 複数バッチにわたり、読込不可・推論失敗・成功が混在しても、成功/失敗の
    /// 件数が正確で、失敗後も残りの処理が継続することを検証する
    /// （要件 14.3, 14.7, 17.8）。
    ///
    /// 構成（batch_size=2, 6 枚 → 3 バッチ）:
    ///   バッチ0: [good0(成功), bad0(読込不可)]
    ///   バッチ1: [good1(推論失敗), good2(成功)]
    ///   バッチ2: [bad1(読込不可), good3(成功)]
    /// 期待: succeeded=3, failed=3。
    #[test]
    fn partial_failures_across_multiple_batches_are_counted_and_processing_continues() {
        let dir = tempdir().unwrap();

        // 入力順を固定して読込不可と有効画像をインターリーブする。
        let good0 = dir.path().join("good0.png");
        write_image(&good0);
        let bad0 = dir.path().join("bad0.png");
        std::fs::write(&bad0, b"not an image 0").unwrap();
        let good1 = dir.path().join("good1.png");
        write_image(&good1);
        let good2 = dir.path().join("good2.png");
        write_image(&good2);
        let bad1 = dir.path().join("bad1.png");
        std::fs::write(&bad1, b"not an image 1").unwrap();
        let good3 = dir.path().join("good3.png");
        write_image(&good3);

        // run_labeled_inference は読込成功した画像に対してのみ入力順に呼ばれる。
        // 呼び出し順: good0, good1, good2, good3。good1（2 回目）だけ推論失敗させる。
        let runner = SequencedRunner {
            output: vec![0.9, 0.9],
            fail_on_call: vec![false, true, false, false],
            calls: std::cell::RefCell::new(0),
        };
        let labels = labels(&["a", "b"]);
        let cancel = AtomicBool::new(false);
        let mut progress = noop_progress();

        let paths = vec![
            good0.to_string_lossy().into_owned(),
            bad0.to_string_lossy().into_owned(),
            good1.to_string_lossy().into_owned(),
            good2.to_string_lossy().into_owned(),
            bad1.to_string_lossy().into_owned(),
            good3.to_string_lossy().into_owned(),
        ];
        let result = run_inference(
            &runner,
            &paths,
            0.5,
            Some(2),
            &labels,
            4,
            ChannelOrder::Bgr,
            "op-partial",
            &cancel,
            &mut progress,
        );

        // 成功=good0/good2/good3 の 3、失敗=bad0/good1/bad1 の 3。
        assert_eq!(result.succeeded, 3);
        assert_eq!(result.failed, 3);
        assert_eq!(result.excluded, 0);
        assert_eq!(result.cancelled, 0);
        // 失敗 3 件それぞれのメッセージが記録される（件数記録の裏付け）。
        assert_eq!(result.messages.len(), 3);

        // 成功した画像のみ Tag_File が生成される。
        assert!(dir.path().join("good0.txt").exists());
        assert!(dir.path().join("good2.txt").exists());
        assert!(dir.path().join("good3.txt").exists());
        // 失敗した画像は Tag_File を残さない。
        assert!(!dir.path().join("bad0.txt").exists());
        assert!(!dir.path().join("good1.txt").exists());
        assert!(!dir.path().join("bad1.txt").exists());
    }

    /// バッチシーケンスの途中でキャンセルすると、残り全バッチが中止され、
    /// キャンセル件数が正確であることを検証する（要件 17.7）。
    ///
    /// batch_size=2, 6 枚 → 3 バッチ。最初のバッチ（2 件）処理後にキャンセルを
    /// 立てると、次のバッチ境界で中止し、残り 4 件が cancelled になる。
    #[test]
    fn cancel_mid_batch_sequence_stops_all_remaining_with_exact_count() {
        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..6 {
            let p = dir.path().join(format!("m{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner {
            output: vec![0.9],
        };
        let labels = labels(&["a"]);
        let cancel = AtomicBool::new(false);
        // 最初のバッチ（2 件）を処理し終えたところでキャンセルを立てる。
        let mut processed = 0usize;
        let mut progress = |_p: Progress| {
            processed += 1;
            if processed == 2 {
                cancel.store(true, Ordering::Relaxed);
            }
        };

        let result = run_inference(
            &runner,
            &paths,
            0.5,
            Some(2),
            &labels,
            4,
            ChannelOrder::Bgr,
            "op-cancel-mid",
            &cancel,
            &mut progress,
        );

        // 先頭バッチ 2 件は成功、残り 4 件はキャンセルで中止。
        assert_eq!(result.succeeded, 2);
        assert_eq!(result.cancelled, 4);
        assert_eq!(result.failed, 0);
        // 処理済みの 2 件のみ Tag_File があり、残りは未生成。
        assert!(dir.path().join("m0.txt").exists());
        assert!(dir.path().join("m1.txt").exists());
        for i in 2..6 {
            assert!(!dir.path().join(format!("m{i}.txt")).exists());
        }
    }

    /// 最初のバッチが処理される前にキャンセル済みなら、何も処理されず全件が
    /// cancelled になることを検証する（要件 17.7）。
    #[test]
    fn cancel_before_first_batch_processes_nothing() {
        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..4 {
            let p = dir.path().join(format!("pre{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner {
            output: vec![0.9],
        };
        let labels = labels(&["a"]);
        // 開始前からキャンセル要求済み。
        let cancel = AtomicBool::new(true);

        // 1 件でも処理されたら進捗が飛ぶはず。飛ばないことを確認する。
        let mut count = 0usize;
        let mut progress = |_p: Progress| {
            count += 1;
        };

        let result = run_inference(
            &runner,
            &paths,
            0.5,
            Some(2),
            &labels,
            4,
            ChannelOrder::Bgr,
            "op-cancel-pre",
            &cancel,
            &mut progress,
        );

        assert_eq!(result.succeeded, 0);
        assert_eq!(result.failed, 0);
        assert_eq!(result.cancelled, 4);
        // 進捗は一度も通知されない。
        assert_eq!(count, 0);
        // Tag_File は 1 つも生成されない。
        for i in 0..4 {
            assert!(!dir.path().join(format!("pre{i}.txt")).exists());
        }
    }
}
