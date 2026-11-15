<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/brand/nautra-cover-dark.png">
    <img src="./docs/assets/brand/nautra-cover.png" alt="Nautra" width="720">
  </picture>
</p>

<p align="center"><strong>WebAssembly のワークロードをビルドし、配布し、実行する。</strong></p>

<p align="center">
  <a href="./LICENSING.md"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20Enterprise-6447f0.svg" alt="License: Apache-2.0 + Nautra Enterprise"></a>
  <img src="https://img.shields.io/badge/status-pre--alpha-c679fc.svg" alt="Status: pre-alpha">
</p>

<p align="center">
  <a href="#目指すもの">目指すもの</a> ·
  <a href="#現状">現状</a> ·
  <a href="#エディションとライセンス">エディション</a> ·
  <a href="./LICENSING.md">ライセンス</a> ·
  <a href="./CONTRIBUTING.md">コントリビューション</a> ·
  <a href="./SECURITY.md">セキュリティ</a>
</p>

<p align="center">
  <a href="README.md"><img alt="English" src="https://img.shields.io/badge/English-d0d7de?style=flat-square"></a>
  <a href="README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-24292f?style=flat-square"></a>
</p>

## 目指すもの

Nautra は、Linux コンテナに対するコンテナエンジンの役割を、Wasm コンポーネントに対して担います。

Wasm モジュールは、それ自体がサンドボックスです。ファイルシステムもネットワークも持たない状態で起動し、渡された権限だけを使えます。Nautra はこの単位を扱う道具をそろえます。イメージとしてまとめ、レジストリで配布し、明示的に渡した権限だけで実行します。ゲストカーネルもデーモンも要りません。

開発元は [Aid-On](https://github.com/Aid-On) です。姉妹プロジェクトの [Porta](https://github.com/almide/porta) はホストのカーネルでネイティブプロセスを閉じ込めます。Nautra が扱う単位は Wasm コンポーネントです。

## 現状

**まだ使えません。** このリポジトリにあるのは、名前、ブランド素材、ライセンス構成、コントリビューション条件だけです。コードもリリースもありません。

## エディションとライセンス

Nautra はオープンコアです。

| | Community Edition | Nautra Enterprise |
|---|---|---|
| ソース | [`ee/`](ee/) 以外のすべて | [`ee/`](ee/) の内容を加えたもの |
| ライセンス | [Apache License 2.0](LICENSE) | [Nautra Enterprise License](ee/LICENSE)。Aid-On とのサブスクリプション契約が必要 |

2つの境界と、それを保つためのルールは [LICENSING.md](LICENSING.md) にあります。名前とロゴの扱いは [TRADEMARKS.md](TRADEMARKS.md) で定めています。

## コントリビューション

[CONTRIBUTING.md](CONTRIBUTING.md) を見てください。コントリビューションは [Contributor License Agreement](CLA.md) のもとで受け付けます。脆弱性は [SECURITY.md](SECURITY.md) の手順で非公開のまま報告してください。

## 名前

Nautra は、nautilus（オウムガイ）の naut- をもとにした造語です。ロゴのオウムガイは、殻の部屋を後ろから順に閉じていき、いちばん外側の部屋で暮らします。
