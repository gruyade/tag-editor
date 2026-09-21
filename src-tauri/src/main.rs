//! TagEditor アプリのバイナリ入口。
//!
//! ロジック本体はライブラリクレット（`tag_editor_core`）に置き、ここは
//! ライブラリの [`tag_editor_core::run`] を呼ぶだけの薄い入口とする。これにより
//! ユニット/プロパティテストは lib を対象に実行でき、アプリ起動コードとテスト
//! 対象コードを分離できる。
//!
//! Windows のリリースビルドでコンソールウィンドウを出さないよう
//! `windows_subsystem = "windows"` を指定する（デバッグビルドでは付与しない
//! ため、開発時はログをコンソールで確認できる）。

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    tag_editor_core::run();
}
