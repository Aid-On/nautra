<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/brand/nautra-cover-dark.png">
    <img src="./docs/assets/brand/nautra-cover.png" alt="Nautra" width="720">
  </picture>
</p>

<p align="center"><strong>WebAssembly のワークロードをビルドし、配布し、実行する。渡した権限だけで。</strong></p>

<p align="center">
  <a href="https://github.com/Aid-On/nautra/actions/workflows/ci.yml"><img src="https://github.com/Aid-On/nautra/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="./LICENSING.md"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20Enterprise-6447f0.svg" alt="License: Apache-2.0 + Nautra Enterprise"></a>
  <img src="https://img.shields.io/badge/status-pre--alpha-c679fc.svg" alt="Status: pre-alpha">
</p>

<p align="center">
  <a href="#クイックスタート">クイックスタート</a> ·
  <a href="#権限">権限</a> ·
  <a href="#サービス">サービス</a> ·
  <a href="#エージェントとツール">エージェント</a> ·
  <a href="#バックグラウンド実行">バックグラウンド</a> ·
  <a href="#イメージ">イメージ</a> ·
  <a href="#レジストリ">レジストリ</a> ·
  <a href="#現状">現状</a> ·
  <a href="./docs/adr/">設計</a> ·
  <a href="./LICENSING.md">ライセンス</a>
</p>

<p align="center">
  <a href="README.md"><img alt="English" src="https://img.shields.io/badge/English-d0d7de?style=flat-square"></a>
  <a href="README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-24292f?style=flat-square"></a>
</p>

```text
$ nautra build -t upper:v1 .
upper:v1  sha256:802fb65f45b0acc4f189fca1e7d3deb22aadf4f363c035a0bbdbdcc1969c04b5

$ nautra run upper:v1
nautra: this image asks for grants this run does not give: -v <dir>:/data  --allow-net api.example.com:443  -e API_TOKEN
Error: fs.read_text("/data/in.txt"): Bad file descriptor (os error 9)

$ nautra run -v ./data:/data upper:v1
copied 5 characters from /data/in.txt to /data/out.txt
```

Nautra は、Linux コンテナに対するコンテナエンジンの役割を、WebAssembly に対して担います。`.wasm` バイナリをイメージにまとめ、OCI レジストリで配布し、実行します。

コンテナとの違いは出発点です。コンテナはファイルシステムとネットワークを丸ごと持った状態から始まり、そこから制限をかけます。Wasm のワークロードは**何も持たない**状態から始まります。ディレクトリもネットワークも環境変数もなく、標準入力は閉じています。使えるのは `run` の行に書いたものだけです。デーモンもゲストカーネルも root も要りません。

開発元は [Aid-On](https://github.com/Aid-On) です。[Almide](https://github.com/almide/almide) で書かれ、[wasmtime](https://wasmtime.dev) の上で動きます。Nautra は [Porta](https://github.com/almide/porta) から派生しました。Porta はホストのカーネルでネイティブプロセスを閉じ込めます。Nautra が扱う単位は Wasm バイナリです。

## クイックスタート

リリースバイナリはまだ公開していないので、ソースからビルドします。Rust 1.97 以降が必要です。

```bash
git clone https://github.com/Aid-On/nautra && cd nautra
bash scripts/install-almide.sh                       # 固定した Almide コンパイラを入れる
.tools/almide/almide build src/main.almd -o target/nautra
bash scripts/build-examples.sh                       # サンプルを target/wasm にビルドする
```

`.wasm` ファイルはそのまま実行できます。イメージにまとめてから実行することもできます。

```bash
target/nautra run target/wasm/hello.wasm tide        # hello, tide
target/nautra build -t hello:v1 target/wasm/hello.wasm
target/nautra run hello:v1 tide
target/nautra images
```

コアモジュール（WASI preview 1）とコンポーネント（WASI 0.2、0.3）のどれも実行できます。どれとして扱うかは、バイナリのヘッダで決まります。

## 権限

オプション1つが、1つのものを開きます。オプションはイメージ名の前に書きます。イメージ名より後ろはすべてワークロードの引数になります。

| オプション | 開くもの |
|---|---|
| `-v HOST:GUEST[:ro]` | ディレクトリ1つ。`GUEST` はワークロードから見える場所。`:ro` を付けると読み取りのみ |
| `-e NAME=VALUE`、`-e NAME` | 環境変数1つ。名前だけ書くとホストの値を渡す |
| `--allow-net HOST:PORT` | 接続先1つ。`HOST` だけ、`*:PORT`、すべてを許す `*` も書ける |
| `--secret NAME@HOST` | ホストの環境変数1つを、`HOST` への HTTP リクエストにだけ使わせる。ワークロードが持つのは身代わりの値で、本物は渡らない |
| `--audit FILE` | 何も開かない。この実行のネットワークに関する判定を、すべて `FILE` に追記する |
| `-p [ADDRESS:]PORT` | HTTP サービスが待ち受ける場所。アドレスを書かなければ、このマシンからの接続だけを受ける |
| `-i` | この端末の標準入力 |
| `-m 64m` | メモリの上限 |
| `--timeout 30s` | 制限時間（終了コード 124） |
| `--fuel N` | 実行できる命令数の上限（終了コード 137） |

```text
$ nautra run target/wasm/dial.wasm 127.0.0.1 8080
Error: tcp_connect(127.0.0.1:8080): permission denied

$ nautra run --allow-net 127.0.0.1:8080 target/wasm/dial.wasm 127.0.0.1 8080
connected to 127.0.0.1:8080
```

ネットワークの許可は、ソケットにも、外向きの HTTP（`wasi:http`）にも同じように効きます。ソケットの場合、ホスト名はワークロードが動き出す前に1度だけ名前解決します。そのため、許可された名前の指す先をワークロードが後から変えることはできません。HTTP の場合は、URL に書かれたホスト名を許可と照合し、許可のない宛先へのリクエストは接続する前に拒否します。マウントしたディレクトリの外へは、`..` でもシンボリックリンクでも出られません。

### 秘密の値

API を呼ぶワークロードには鍵が要ります。しかし鍵を持ったワークロードは、届く範囲のどこへでもそれを送れます。`--secret` は、この2つを切り離します。

```text
$ export API_TOKEN=real-token
$ nautra run --secret API_TOKEN@api.example.com app.wasm
```

ワークロードの中では、`API_TOKEN` は `nautra-secret-3f9a…` のような身代わりの値になります。ワークロードは、これを普段どおりのヘッダに入れて送ります。宛先が `api.example.com` なら、Nautra が身代わりを本物の値に差し替えます。身代わりをほかの宛先に送ろうとしたリクエストは拒否します。この指定は、そのホストへの HTTPS も開きます。開くのはそれだけです。なお、束縛先のホストは、その鍵を預ける相手として信頼することになります。

`--audit FILE` は、ネットワークに関する判定を1件につき1行の JSON で追記します。HTTP リクエストなら宛先・パス・ステータス・載せた秘密の名前を、拒否なら理由を、ソケットなら接続先を記録します。ヘッダの値、クエリ文字列、秘密の値は記録しません。

ワークロード自身の終了コードは、そのまま返します。`125` は、Nautra が実行を拒否したか、開始できなかったことを表します。

## サービス

HTTP ハンドラ（`wasi:http` の 0.2 か 0.3）を export するコンポーネントは、サービスとして動かせます。サービス自身はポートを持ちません。export するのは関数で、リクエストをどこから受けるかはホストが決めます。そのため `-p` に書くポートは1つで、対応づけは書きません。

```text
$ nautra run -p 8080 target/wasm/web.wasm
nautra: serving on http://127.0.0.1:8080

$ curl http://127.0.0.1:8080/tide/pool
GET /tide/pool from a wasm component
```

`-p 8080` は、このマシンからの接続だけを受けます。ほかのマシンからも受けるなら、`-p 0.0.0.0:8080` と明示します。リクエストは並行して処理します。`--timeout` はリクエスト1件ごとの制限時間です。サービスに渡る権限も通常の実行と同じで、既定では何も渡りません。Ctrl-C か SIGTERM で止まります。

## エージェントとツール

Nautra では、エージェントも普通のコンポーネントです。外の世界とのやり取りはすべて HTTP リクエストで、URL ごとの意味は Nautra が決めます。モデルは「秘密の値を束縛した、許可済みのホスト」です。ツールは `http://tools.nautra/` にある1つの [MCP](https://modelcontextprotocol.io) サーバとして見えます。この名前へのリクエストは Nautra の外に出ません。その先にある各ツールは別のイメージで、呼び出しごとに新しく起動し、それぞれ独自の権限で動きます。

ワークロードにどのツールを持たせるかは、ファイルに書きます。

```toml
# nautra.toml
[agent]
image = "ghcr.io/you/agent:v1"
env = ["MODEL_URL=https://api.example.com/v1/chat/completions"]
secrets = ["MODEL_KEY@api.example.com"]     # エージェントが持つのは身代わりの値
timeout = "2m"
max_tool_calls = 16

[tools.read_file]
image = "ghcr.io/you/fs-tools@sha256:…"     # ダイジェストで指定すれば固定になる
description = "Read a file from the workspace."
input = { type = "object", properties = { path = { type = "string" } }, required = ["path"] }
mounts = ["./workspace:/work:ro"]           # このツールだけの権限。エージェントには渡らない

[tools.upper]
image = "upper:v1"                           # 権限なし。計算しかできない
```

```text
$ nautra run -f nautra.toml "What does the note say?"
The note says: THE TIDE IS IN
```

このファイルに書けるのは、`run` の行に書けることだけです。各項目の `mounts`、`net`、`env`、`secrets`、`timeout`、`memory`、`fuel` は同名のオプションと同じ意味で、同じパーサが読みます。ファイル内のパスは、ファイルの場所からの相対パスです。コマンドラインに書いたオプションは、`[agent]` の項目に追加されます。

ツールにできるのは、コマンドとして動くイメージです。呼び出しの引数を標準入力から JSON で受け取り、標準出力に書いた内容が結果になります。そのため、手で試せます。

```text
$ echo '{"text": "tide"}' | nautra run -i upper:v1
TIDE
```

引数は、ツールを起動する前にホスト側で `input` と照合します。拒否するときは何が違うかを返すので、モデルは呼び出しを直せます。ツールの `max_calls` と、エージェントの `max_tool_calls` は上限です。上限を超えた呼び出しにはエラーの結果を返し、何も実行しません。`--audit` を付けると、ツールの呼び出しも、ツール自身の通信も、同じファイルに記録します。

[`examples/agent`](examples/agent) に、Almide で書いた100行ほどのエージェントと、ツール2つがあります。

### 検査

検査は、運用する側が「同意すること」を求めるイメージです。呼ばれるたびに新しく起動し、独自の権限で動きます。何について聞かれているかは、標準入力から JSON で受け取ります。終了コード 0 が同意で、それ以外は拒否です。拒否のときは、出力した内容が理由になります。

```toml
[checks.only_notes]
image = "checks/only-notes:v1"
before = ["write_file"]                  # これらのツールの前に聞く。"*" は全ツール

[checks.has_summary]
image = "checks/has-summary:v1"
finish = true                            # これが認めるまで、実行は受理されない
mounts = ["./workspace:/work:ro"]        # エージェント経由ではなく、自分の目で確かめる
```

`before` を指定した検査には、ツールの名前と引数が渡ります。検査が拒否すると、ツールは実行されません。呼び出した側には、理由がエラーの結果として返ります。

`finish` を指定した検査は、ワークロードが終了コード 0 で終わったときに走ります。検査が拒否すると、実行は終了コード 4 で終わり、理由を表示します。ワークロードの「終わりました」は申告にすぎず、受理するのは検査です。エージェントが手を打てるうちに結果を知れるよう、同じ検査を `verify` というツールとしても渡します。`verify` は、検査が何に不満かを返します。

### エージェントをツールにする

ツールには、別のエージェントも指定できます。モデルもツールも独自に持つ、構成ファイル1つ分です。引数は `task` の1つだけです。

```toml
[tools.reviewer]
agent = "reviewer.toml"
description = "Review a proposed change independently."
```

呼ばれる側のエージェントには、呼んだ側のものは何も渡りません。会話も、秘密の値も、マウントも、ツールも渡りません。仕事を次へ渡せる深さは4段までです。

### 外部の MCP サーバ

外部の MCP サーバは、「そのサーバを呼ぶツールイメージ」としてエージェントに渡します。[`examples/tools/mcp_call.almd`](examples/tools/mcp_call.almd) が60行ほどの実例です。こうすると、サーバはほかの宛先と同じく「許可したホストと、束縛した秘密の値」で表せます。Nautra 自身は、そのサーバについて何も知る必要がありません。

```toml
[tools.search]
image = "ghcr.io/you/mcp-call:v1"
description = "Search the knowledge base."
env = ["MCP_URL=https://mcp.example.com/mcp", "MCP_TOOL=search"]
secrets = ["MCP_TOKEN@mcp.example.com"]
```

### エージェントを公開する

エージェントは、仕事を終えたら終了するワークロードです。エージェントの実行基盤が求めるのはサーバです。`[agent]` の横に `[invoke]` を書くと、エージェントを書き換えずに公開できます。POST 1 回が実行 1 回になり、リクエストの本文が標準入力に、標準出力がそのまま応答になります。

```toml
[agent]
image = "agent.wasm"
secrets = ["MODEL_KEY@api.example.com"]

[invoke]
listen = "8080"            # PORT（このマシンのみ）か ADDRESS:PORT
token = "DOOR_TOKEN"       # 認証する基盤の後ろに置くなら open = true
```

```console
$ nautra run -f nautra.toml
nautra: each POST runs the workload: http://127.0.0.1:8080/invocations
$ curl -H "Authorization: Bearer $DOOR_TOKEN" -d '{"prompt": "What does the note say?"}' localhost:8080/invocations
The note says: THE TIDE IS IN
```

リクエストをまたいで残るものはありません。パスは何でも受け付け、`GET /ping` には `{"status": "Healthy"}` を返します。これは Amazon Bedrock AgentCore Runtime がコンテナに求める形です。コンテナに入れるものは [examples/platforms](examples/platforms/) にあります。詳しくは [ADR-0003](docs/adr/0003-platforms.md) を参照してください。

### 別の場所で動くエージェントにツールを公開する

これらのツールを使うエージェントは、Wasm である必要も、Nautra の下で動いている必要もありません。`[agent]` の代わりに `[mcp]` を書いた構成ファイルは、ツール群を MCP サーバとして公開します。

```toml
[mcp]
listen = "8000"            # PORT（このマシンのみ）か ADDRESS:PORT
token = "MCP_TOKEN"        # 呼び出し側のベアラートークンを持つホストの環境変数
audit = "calls.jsonl"

[tools.read_file]
image = "ghcr.io/you/read-file:v1"
mounts = ["./ws:/work:ro"]
```

```console
$ MCP_TOKEN=… nautra run -f tools.toml
nautra: tools published at http://127.0.0.1:8000/mcp
```

呼び出しの扱いは、実行の中から呼ばれたときと同じです。引数をスキーマで検査し、`before` の検査を先に通し、ツール自身の権限で新しいインスタンスを起動し、呼び出しを記録します。アドレスに届く相手は誰でもツールを呼べるので、このマシンの外に公開するときは、誰が呼べるかを必ず書きます。`token` を指定するか、Nautra の手前で基盤が認証する場合に限って `open = true` と書きます。リクエストは `POST /mcp` への JSON-RPC です。詳しくは [ADR-0003](docs/adr/0003-platforms.md) を参照してください。

### ネイティブコマンド

Nautra が閉じ込めるのは Wasm です。エージェントが git やコンパイラ、テストランナーのようなネイティブのプログラムを必要とするときは、ツールや検査の項目に `image` の代わりに `command` を書きます。Nautra はそれを、姉妹プロジェクト [Porta](https://github.com/almide/porta) のサンドボックスで閉じ込めます。このサンドボックスは Nautra に組み込まれているので、ほかに何もインストールする必要はありません。Porta は、ホストのカーネルでネイティブのプロセスを閉じ込めます。

```toml
[tools.git_log]
command = ["git", "log", "--oneline", "-5"]
description = "The last five commits."
mounts = ["./repo:ro"]

[checks.tests_pass]
command = ["cargo", "test", "--offline"]
finish = true
mounts = ["./repo"]
reads = "open"                           # ツールチェーンはマウントの外も読む
```

権限は Nautra の書き方で書き、Nautra がそれをサンドボックスへの依頼1つに変換します。既定の姿勢も Nautra のままです。ネットワークは書かなければ閉じています。読み取りは、`reads = "open"` と書かないかぎり、マウントとシステムのディレクトリに限ります。Wasm の項目と違う点が4つあります。ネイティブのプロセスは、閉じ込める対象として別物だからです。

- マウントは、ホストのディレクトリをその場所のまま見せます（`DIR` か `DIR:ro`）。ゲスト側のパスはありません。コマンドは、最初のマウント先を作業ディレクトリとして始まります。
- ネイティブのコマンドは、作業用の場所として `/tmp` にいつでも書き込めます。これはホスト自身の `/tmp` です。そこにあるディレクトリは読み取り専用にできないので、`/tmp` の下を `:ro` でマウントする項目は拒否します。書き換えられて困るものは `/tmp` に置かないでください。
- ネットワークは、ホスト名（HTTPS）かポート（`*:8080`）のどちらかで許可します。アドレスでは許可できず、両方を同時に書くこともできません。書かれたとおりにサンドボックスが守らせられない項目は、緩めて実行せずに拒否します。
- `memory` と `fuel` は使えません。サンドボックスにしかない設定は、`porta = ["--max-procs", "200"]` のように書きます。使えるのは `--max-cpu`、`--max-procs`、`--max-file-size`、`--max-memory-mb`、`--allow-bind`、`--allow-unix`、`--deny-unix`、`--deny-read`、`--protect`、`--preset`、`--allow-root` です。

引数を標準入力から JSON で受け取り、標準出力が結果になる点は、Wasm のツールと同じです。コマンドは Nautra がもう1つ起動した自分自身の下で動き、そちらがサンドボックスを適用して終了を待ちます。サンドボックスの実体は、Linux では Landlock と seccomp と名前空間、macOS では `sandbox-exec` です。

### 記録、再生、再開

`--record FILE` を付けると、実行の境界を越えたやり取りを順番どおりに書き残します。外向きの HTTP リクエストとその応答、ツールの呼び出しとその結果が対象です。

```text
$ nautra run --record run.record -f nautra.toml "What does the note say?"
The note says: THE TIDE IS IN

$ nautra inspect run.record
record    run.record
wasm      sha256:…
run       -f nautra.toml What does the note say?
exchanges 5 (3 http, 2 tool)
state     finished, exit code 0

$ nautra replay run.record
The note says: THE TIDE IS IN
nautra: the run matches its record: 5 exchange(s) replayed, nothing sent
```

再生では、ワークロードをもう一度動かし、すべてのやり取りに記録から答えます。モデルは呼ばず、ツールも起動せず、鍵も要りません。ワークロードが、記録の次の項目と違うことを求めた場合は、記録から外れたと判断して終了コード 3 で終わります。つまり再生は、同じバイナリが同じ判断をするかどうかの検査にもなります。

途中で止まった実行は、`nautra run --resume FILE` で続けられます。記録済みのやり取りを再生し、その先は実際に実行して、記録に追記していきます。やり取りは、送る前に要求を、返ってきた後に応答を、と2段階で書きます。そのため、その間で止まった実行は、そこで止まったと分かります。そのやり取りが相手に届いたかどうかは分からないので、繰り返しません。その先へは再開しません。

記録は、作ったときのバイナリそのものに結び付いています。再生するときは、記録したときと同じディレクトリで実行します。記録にはワークロードが受け取った内容が入るので、実行と同じだけ機密です。ただし、リクエストヘッダは記録しないので、秘密の値は入りません。`--record` を付けた実行では、ストリームで返る応答も、まとめて1度にワークロードへ届きます。時計と乱数は記録しません。リクエストの内容がそれらに左右されるワークロードは、再生で記録から外れます。

## バックグラウンド実行

`-d` を付けると、バックグラウンドで実行して ID を表示します。裏でデーモンが動くわけではありません。実体は、端末を持たない Nautra のプロセスがもう1つ動いているだけです。Nautra がその実行について知っていることは、`~/.nautra/runs` の下のディレクトリにすべて入っています。

```text
$ nautra run -d --name web -p 8080 target/wasm/web.wasm
5e834c6e4266

$ nautra ps
ID            NAME  IMAGE                 STATUS   PORT            STARTED
5e834c6e4266  web   target/wasm/web.wasm  running  127.0.0.1:8080  2026-11-15T07:27:31Z

$ nautra logs web
nautra: serving on http://127.0.0.1:8080

$ nautra stop web
web
```

| | |
|---|---|
| `nautra run -d [--name NAME] [--rm] …` | バックグラウンドで始める。`--rm` を付けると、終わったときに記録を消す |
| `nautra ps [-a]` | 実行中のものを一覧する。`-a` で、終わったものと終わり方も出す |
| `nautra logs [-f] RUN` | 標準出力と標準エラーに書かれた内容を出す。`-f` で、終わるまで追い続ける |
| `nautra stop [-t 10s] RUN…` | SIGTERM を送り、待っても終わらなければ SIGKILL を送る |
| `nautra rm [-f] RUN…` | 終わった実行の記録とログを消す。`-f` で、実行中なら止めてから消す |

`RUN` には、名前、ID、ID の先頭部分のどれでも書けます。始めた直後に失敗した場合（ポートが使用中、マウント先がない、など）は、`ps` で探すまでもなく、`run -d` 自身が終了コードとともに報告します。バックグラウンドの実行は、マシンを止めれば終わります。自動では再開しません。

## イメージ

イメージは、`.wasm` バイナリとその説明をまとめたものです。説明は `Nautrafile` に書きます。

```toml
wasm = "app.wasm"
description = "Uppercases /data/in.txt into /data/out.txt"

[run]
args = ["/data/in.txt", "/data/out.txt"]

[run.env]
LOG_LEVEL = "info"

[wants]                     # 実行時に渡してほしい権限
mounts = ["/data"]
net = ["api.example.com:443"]
env = ["API_TOKEN"]
```

`[wants]` はイメージの作者による申告で、許可ではありません。Nautra はこれを表示するだけです。表示するのは、`inspect` したときと、申告された権限を渡さずに `run` したときです。イメージが求めたからといって、何かが開くことはありません。

```text
$ nautra inspect upper:v1
name      upper:v1
digest    sha256:802fb65f45b0acc4f189fca1e7d3deb22aadf4f363c035a0bbdbdcc1969c04b5
wasm      sha256:e45630d64ca18182ead688724388cabc2a5958fb6fc1e835fd95ef2a8eb80d03
kind      module
size      19.3 kB
reach     files, environment
about     Uppercases /data/in.txt into /data/out.txt
args      /data/in.txt, /data/out.txt
env       LOG_LEVEL=info
asks for  -v <dir>:/data, --allow-net api.example.com:443, -e API_TOKEN
```

`reach` はバイナリの import から読み取った値で、そのバイナリが使いうるものの外枠を表します。ソケットを import していないバイナリは、どんな権限を渡されてもネットワークに届きません。

イメージは、標準の [OCI image layout](https://github.com/opencontainers/image-spec/blob/main/image-layout.md) として `~/.nautra/store` に保存します。場所は `$NAUTRA_HOME` で変えられます。形式は CNCF の [Wasm OCI Artifact](https://tag-runtime.cncf.io/wgs/wasm/deliverables/wasm-oci-artifact/) に合わせています。blob は読むたびにダイジェストと照合するので、ディスク上で書き換わったバイナリは実行されず、拒否されます。`SOURCE_DATE_EPOCH` を設定すると、同じ入力からは同じダイジェストができます。

| | |
|---|---|
| `nautra build [-t NAME] [-f Nautrafile] [PATH]` | イメージを作る。`PATH` は Nautrafile のあるディレクトリか、`.wasm` ファイル |
| `nautra images` | 一覧を出す |
| `nautra inspect IMAGE\|FILE.wasm` | 中身、求めている権限、届きうる範囲を表示する |
| `nautra tag SOURCE NAME` | 別の名前を付ける |
| `nautra rmi IMAGE…` | 名前を消す。どこからも参照されなくなった blob も消す |

## レジストリ

```bash
nautra build -t ghcr.io/you/app:v1 .
nautra push ghcr.io/you/app:v1
nautra run ghcr.io/you/app:v1          # 初回は自動で pull する
nautra pull ghcr.io/you/app@sha256:…   # ダイジェストで固定する
```

OCI Distribution API に対応したレジストリなら、どれでも使えます。レジストリ名のない名前（`app:v1`）はローカル専用です。Nautra には既定のレジストリがないので、書かれていないレジストリに接続することはありません。pull したものは、ストアに入れる前にすべてハッシュを計算し、指定されたダイジェストと照合します。Wasm のレイヤーを持たないイメージ（Linux コンテナのイメージ）は拒否します。

ログイン情報は、環境変数 `NAUTRA_USERNAME` と `NAUTRA_PASSWORD` から読みます。パスワードにはレジストリのトークンも使えます。

## 現状

**プレアルファです。** ここまでに書いた機能は動作し、macOS と CI 上の Linux で `scripts/integration.py` が検証しています。WASI 0.3 とその `wasi:http` は wasmtime でまだ実験的な扱いで、サービスの機能もその制約を受けます。push と pull はローカルの `registry:2` で検証済みですが、公開レジストリではまだ検証していません。

[ADR-0002](docs/adr/0002-agents.md) に書いたエージェント向けの機能は実装済みです。エージェントの実行基盤に載せる話は [ADR-0003](docs/adr/0003-platforms.md) にまとめています。Nautra 側の動作は Linux の x86-64 と arm64 で検証していますが、各基盤の上ではまだ動かしていません。

まだ無いもの：

- 再起動ポリシーと、マシン再起動後の自動再開
- 公開したポートの TLS と HTTP/2
- 複数のワークロードをまとめて動かす機能（compose）と、その間のネットワーク
- 署名、来歴の証明、pull してよいものを決めるポリシー
- 認証情報ヘルパー、コンパイル結果のキャッシュ、リリースバイナリ

## エディションとライセンス

Nautra はオープンコアです。

| | Community Edition | Nautra Enterprise |
|---|---|---|
| ソース | [`ee/`](ee/) 以外のすべて | [`ee/`](ee/) の内容を加えたもの |
| ライセンス | [Apache License 2.0](LICENSE) | [Nautra Enterprise License](ee/LICENSE)。Aid-On とのサブスクリプション契約が必要 |

この README に書いた機能は、すべて Community Edition のものです。2つの境界と、それを保つためのルールは [LICENSING.md](LICENSING.md) にあります。名前とロゴの扱いは [TRADEMARKS.md](TRADEMARKS.md) で定めています。

## コントリビューション

[CONTRIBUTING.md](CONTRIBUTING.md) を見てください。コントリビューションは [Contributor License Agreement](CLA.md) のもとで受け付けます。脆弱性は [SECURITY.md](SECURITY.md) の手順で非公開のまま報告してください。

## 名前

Nautra は、nautilus（オウムガイ）の naut- をもとにした造語です。ロゴのオウムガイは、殻の部屋を後ろから順に閉じていき、いちばん外側の部屋で暮らします。
