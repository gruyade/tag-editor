//! Tauri アプリのビルドスクリプト。
//!
//! `tauri.conf.json` を読み取り、`tauri::generate_context!` が参照する
//! コンパイル時コンテキスト（アプリ設定・ケイパビリティ）を用意する。
//! テスト（lib のみ）でも build.rs は走るが、`tauri_build::build()` は
//! 設定の検証とコード生成のみを行い、実行時のウィンドウ生成には関与しない。

fn main() {
    tauri_build::build();
}
