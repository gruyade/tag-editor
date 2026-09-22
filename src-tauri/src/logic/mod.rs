//! 純粋ロジック層。
//!
//! ファイル I/O や推論に依存しない決定的関数群を集約する。
//! 後続タスク（2〜8）はこれらのモジュールへ実装を追加していく。

pub mod inference_aux; // タスク 8: 閾値採用・動画除外・Batch_Size 解決・バッチ分割
pub mod naming; // タスク 5: ファイル名正規化・連番・gather/distribute 命名
pub mod path_convert; // タスク 7: Windows/Linux パス変換
pub mod sorting; // タスク 6: サイズ振分決定・閾値判定・タグ振分決定
pub mod tag_batch; // タスク 7: Fraction_Threshold 適用・Tag_Overview 構築
pub mod tag_filter;
pub mod tag_format; // タスク 2: 正規化キー・トークン分割・booru 変換・信頼度 parse/render
pub mod tag_ops; // タスク 4: タグ追加/削除/重複除去のコアロジック
pub mod tag_stats; // タスク 3: 集計・フィルタ述語 // タスク 6: Tag_Filter コンパイル・単一画像フィルタ適用
