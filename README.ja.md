# Nautra（ノートラ）

WebAssembly のワークロードをビルドし、配布し、実行するためのツールです。Linux コンテナに対するコンテナエンジンの役割を、Wasm コンポーネントに対して担います。

> **現状：まだ使えません。** このリポジトリにあるのは、名前、ライセンス構成、コントリビューション条件だけです。コードもリリースもありません。

[English](README.md)

## 目指すもの

Wasm モジュールは、それ自体がサンドボックスです。ファイルシステムもネットワークも持たない状態で起動し、渡された権限だけを使えます。Nautra はこの単位を扱う道具をそろえます。イメージとしてまとめ、レジストリで配布し、明示的に渡した権限だけで実行します。ゲストカーネルもデーモンも要りません。

開発元は [Aid-On](https://github.com/Aid-On) です。姉妹プロジェクトの [Porta](https://github.com/almide/porta) はホストのカーネルでネイティブプロセスを閉じ込めます。Nautra が扱う単位は Wasm コンポーネントです。

## エディションとライセンス

Nautra はオープンコアです。

- **Community Edition**：[`ee/`](ee/) 以外のすべて。[Apache License 2.0](LICENSE) です。
- **Nautra Enterprise**：[`ee/`](ee/) の内容を加えたもの。[Nautra Enterprise License](ee/LICENSE) のもとで、Aid-On とのサブスクリプション契約が必要です。

詳しくは [LICENSING.md](LICENSING.md) を見てください。名前とロゴは [TRADEMARKS.md](TRADEMARKS.md)、コントリビューションは [CONTRIBUTING.md](CONTRIBUTING.md)、脆弱性の報告は [SECURITY.md](SECURITY.md) にあります。

## 名前

Nautra は、Nautilus の naut- を芯にした造語です。
