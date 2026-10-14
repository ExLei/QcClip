<p align="center">
  <img src="../assets/icon128.png" alt="QcClip" width="96" height="96">
</p>

<h1 align="center">QcClip</h1>

<p align="center">純 Rust · GPUI によるネイティブ描画 · 高性能クリップボードツール</p>

<p align="center">
  <a href="https://github.com/mosheng1/QcClip/stargazers">
    <img src="https://img.shields.io/github/stars/mosheng1/QcClip?style=for-the-badge&amp;logo=github&amp;color=yellow" alt="Stars">
  </a>
  <a href="https://github.com/mosheng1/QcClip/releases">
    <img src="https://img.shields.io/github/v/release/mosheng1/QcClip?style=for-the-badge&amp;label=Release&amp;color=brightgreen" alt="Release">
  </a>
  <a href="https://github.com/mosheng1/QcClip/releases">
    <img src="https://img.shields.io/github/downloads/mosheng1/QcClip/total?style=for-the-badge&amp;color=blueviolet" alt="Downloads">
  </a>
  <a href="https://github.com/mosheng1/QcClip">
    <img src="https://img.shields.io/github/license/mosheng1/QcClip?style=for-the-badge&amp;color=orange" alt="License">
  </a>
</p>

<p align="center">
  <a href="../README.md">简体中文</a>
  · <a href="README.en.md">English</a>
  · <a href="README.zh-TW.md">繁體中文</a>
  · <b>日本語</b>
  · <a href="README.ko.md">한국어</a>
</p>

**QcClip は、作者自身が開発した [QuickClipboard](https://github.com/mosheng1/QuickClipboard) を純 Rust で書き直した、高性能なクリップボードツールです。GPUI フレームワークによるネイティブ描画を採用しています。**

新しい内容をコピーしても、前の内容を探し直す必要はありません。QcClip はクリップボード履歴を保存し、よく使う返信、リンク、画像、ファイルを手元にまとめます。必要なときに見つけて、そのまま貼り付けられます。

QuickClipboard の使い勝手を受け継ぎながら、応答速度、スムーズな閲覧、日常操作の便利さを重視して書き直しました。毎日気軽に使える、軽快なクリップボードツールを目指しています。

現在は **Windows** を主にサポートしています。macOS と Linux への対応は開発中です。

## QcClip を選ぶ理由

### 軽快な動作で、毎日の常駐に

クリップボードツールは一日に何十回も開くもの。応答やスクロールの快適さが使い心地を左右します。QcClip は、**WebView やブラウザーエンジンに依存しないネイティブ UI** を採用し、余分な実行負荷を抑えています。大量の履歴、画像プレビュー、バックグラウンド動作も最適化し、探すときや貼り付けるときの待ち時間を減らします。

- **履歴が多くても探しやすい**：長いリストの表示と読み込みを最適化し、古い内容を探す際の待ち時間を抑えます。
- **画像が多くてもリソースに配慮**：表示中の内容を優先し、不要になったリソースを解放してメモリの負担を減らします。
- **同じ内容の重複保存を削減**：繰り返しコピーした同一内容の冗長な保存を減らし、履歴を整理しながらディスク容量も節約します。

### コピーした内容を、もっと見つけやすく

資料をまとめる際にコピーした文章、さっき見たリンク、送ろうとしていた画像。ウェブページやチャットを開き直さなくても、履歴から探せます。

- **検索ですぐに絞り込み**：内容、メモ、コピー元のアプリ名で検索できます。
- **貼り付ける前に確認**：テキスト、画像、ファイル、リンクで絞り込み、マウスを重ねて内容をプレビューできます。
- **用途に合った貼り付け**：書式を残したいときはそのまま、不要なときはプレーンテキストで貼り付け。画像は画像ファイルとしても貼り付けられます。

### クリックとウィンドウ切り替えを減らす

ショートカットでウィンドウを開き、内容を選んで貼り付け。よく使う内容はショートカットから直接貼り付けることもでき、繰り返しの操作を減らせます。

- **クイック貼り付け**：コンパクトなウィンドウで方向キーを使って選択し、履歴とお気に入りを切り替えられます。
- **ショートカット貼り付け 1～9**：数字キーや F1～F9 のキー組み合わせを設定し、対応する項目を直接貼り付けられます。
- **複数の項目をまとめて使用**：複数の文章、文章と画像、複数のファイルをまとめてコピー・貼り付けできます。ファイルとテキストの混在結合には対応していません。
- **手元に置いて、作業を妨げない**：画面端に隠す、マウスポインター付近に表示する、ウィンドウを固定するなどの使い方に対応。シングルクリック、ダブルクリック、中クリックの動作も変更できます。

### 一度整理して、何度でも使う

問い合わせへの返信、よく使う住所、仕事のリンク、資料の抜粋などを、自分用の定型文・資料集としてまとめられます。毎回元の文章を探す必要はありません。

- **お気に入りとピン留め**：長く使う内容を保存し、頻繁に使う項目を上に配置。ドラッグで並べ替えられます。
- **タグで分類**：仕事、プライベート、プロジェクトごとに整理。一つの項目に複数のタグを付けられます。
- **メモと編集**：見分けやすい説明を付けたり、よく使う文章を追加・編集したりできます。取得済みの元の内容にも戻せます。

### 絵文字・記号・画像素材を一か所に

チャットの絵文字、文書で使う特殊記号、メッセージに添える画像を、QcClip から取り出せます。

- **絵文字と記号**：カテゴリ、検索、最近使った履歴から選んで、そのまま貼り付けられます。
- **自分だけのギャラリー**：ローカルやブラウザーの画像をグループにドラッグして、よく使う画像やリアクション画像を保存。PNG、JPG、GIF、WebP に対応しています。
- **素材を整理しやすい**：ファイル名で検索し、名前の変更やグループ間の移動ができます。名前、種類、日時による並べ替えにも対応しています。

### データも見た目も、自分の好みに

アカウントへのログインは不要です。クリップボード履歴と画像素材はローカルに保存されます。保存期間、バックアップ先、ウィンドウの見た目を自由に設定できます。

- **保存期間と整理を管理**：履歴の保存日数と件数を設定できます。範囲外の項目は削除待ちになり、削除前なら設定を見直して表示対象に戻せます。
- **バックアップや端末移行に便利**：データのエクスポート・インポートに対応。ポータブル版はデータごと移動でき、インストール版は保存先を変更できます。
- **干渉を減らす**：指定したアプリの使用中は、記録、ショートカット、画面端からの表示を一時停止でき、プライバシーや作業スタイルに合わせられます。
- **好みの外観に調整**：ライト・ダークテーマ、背景画像、透明度、リストの大きさ、アニメーションを変更できます。システムの外観にも追従します。
- **毎日に便利な設定**：自動起動、システムトレイでの常駐、中国語・英語 UI、操作音、更新チェックに対応しています。

## 画面プレビュー

### クリップボードとタグ

<table>
  <tr>
    <td align="center" width="50%">
      <a href="../readme-assets/display/剪贴板.png">
        <img src="../readme-assets/display/剪贴板.png" alt="テキスト・画像・ファイルをまとめたクリップボード履歴" width="360">
      </a>
      <br><strong>クリップボード履歴</strong><br>
      コピーした内容をいつでも再利用
    </td>
    <td align="center" width="50%">
      <a href="../readme-assets/display/标签.png">
        <img src="../readme-assets/display/标签.png" alt="用途ごとに内容を整理するカラータグ" width="360">
      </a>
      <br><strong>カラータグ</strong><br>
      用途別に分類して、探しやすく
    </td>
  </tr>
</table>

### 絵文字・記号・ギャラリー

<table>
  <tr>
    <td align="center" width="33%">
      <a href="../readme-assets/display/Eomji.png">
        <img src="../readme-assets/display/Eomji.png" alt="カテゴリから選んで検索できる絵文字" width="240">
      </a>
      <br><strong>絵文字</strong><br>
      よく使う絵文字をすぐに入力
    </td>
    <td align="center" width="33%">
      <a href="../readme-assets/display/符号.png">
        <img src="../readme-assets/display/符号.png" alt="特殊記号を探して入力できる記号一覧" width="240">
      </a>
      <br><strong>記号</strong><br>
      特殊記号を一か所から
    </td>
    <td align="center" width="34%">
      <a href="../readme-assets/display/图库.png">
        <img src="../readme-assets/display/图库.png" alt="画像素材をグループで管理するギャラリー" width="240">
      </a>
      <br><strong>ギャラリー</strong><br>
      画像素材をグループ別に保存
    </td>
  </tr>
</table>

### マウスを重ねて内容を確認

項目にマウスを重ねるだけで内容をプレビュー。開く手間を一つ減らせます。

<p align="center">
  <a href="../readme-assets/display/悬停内容预览.webp">
    <img src="../readme-assets/display/悬停内容预览.webp" alt="テキスト・画像・ファイルのホバープレビューのアニメーション" width="960">
  </a>
</p>

### 豊富なテーマカラー

ライトからダークまで、好みの配色を選んで、常駐ツールも見やすく快適に。

<p align="center">
  <a href="../readme-assets/display/丰富主题配色.webp">
    <img src="../readme-assets/display/丰富主题配色.webp" alt="テーマカラーと外観設定を切り替えるアニメーション" width="960">
  </a>
</p>

## 使い始める

[ダウンロードページ](https://github.com/mosheng1/QcClip/releases) から適したバージョンを選んでください。

- **インストール版**：普段使う端末での日常利用に。
- **ポータブル版**：展開後、`QcClip/QcClip.exe` を実行します。データは同じ場所の `QcClipData` フォルダーに保存されます。移動・更新の際はこのフォルダーを残してください。

標準では **`Ctrl + Shift + X`** でメインウィンドウを表示・非表示にできます。コピーした内容を検索・絞り込み、項目を選んで再度コピーまたは貼り付けます。クイック貼り付けなどのキー設定は「設定 → ショートカット」から変更できます。

### QuickClipboard からの移行

「設定 → データ管理 → 旧版 QuickClipboard から移行」を開きます。インストール済み旧版のデータ検出、旧版のデータフォルダーやポータブル版フォルダーの選択、旧版からエクスポートした ZIP の読み込みに対応しています。

お気に入りを移行でき、クリップボード履歴も任意で取り込めます。グループはタグに、お気に入りのタイトルはメモになります。保存済みのテキスト、画像、ファイル一覧も移行できます。現在は QuickClipboard **v0.5.0** のデータを基準に対応しています。

移行前に取り込める内容を確認でき、実行時には QcClip の現在のデータを自動でバックアップします。**旧版のギャラリー、設定、同期情報は現在移行されません。** ファイル一覧はパスのみを保持し、実際のファイルはコピーしません。詳しくは [移行ガイド](../docs/legacy-migration.md)（簡体字中国語）をご覧ください。

## フィードバック

不具合や改善の提案は、[Issue ページ](https://github.com/mosheng1/QcClip/issues) にお寄せください。

## コミュニティ

チャンネルやグループに参加して、使用感の共有、改善の提案、開発状況の確認ができます。リンクから参加するか、QQ で QR コードを読み取ってください。

<table>
  <tr>
    <td align="center" width="33%">
      <strong>QcClip チャンネル</strong><br>
      チャンネル ID：pd80680380<br><br>
      <a href="../assets/pD_1.png">
        <img src="../assets/pD_1.png" alt="QcClip チャンネルの QR コード、ID pd80680380" width="200">
      </a>
      <br><a href="https://pd.qq.com/s/blp3j847c">チャンネルに参加</a>
    </td>
    <td align="center" width="33%">
      <strong>QcClip 交流グループ 1</strong><br>
      グループ ID：725313287<br><br>
      <a href="../assets/qG_1.png">
        <img src="../assets/qG_1.png" alt="QcClip 交流グループ 1 の QR コード、ID 725313287" width="200">
      </a>
      <br><a href="https://qm.qq.com/q/nUCO76MX9C">グループ 1 に参加</a>
    </td>
    <td align="center" width="34%">
      <strong>QcClip 交流グループ 2</strong><br>
      グループ ID：1033556729<br><br>
      <a href="../assets/qG_2.png">
        <img src="../assets/qG_2.png" alt="QcClip 交流グループ 2 の QR コード、ID 1033556729" width="200">
      </a>
      <br><a href="https://qm.qq.com/q/O5zOi3uTuy">グループ 2 に参加</a>
    </td>
  </tr>
</table>

## 開発を支援

QcClip が役に立ったら、WeChat の投げ銭で今後の開発・保守を支援できます。

<p align="center">
  <a href="../assets/wxzsm.png">
    <img src="../assets/wxzsm.png" alt="WeChat 支援用コード" width="240">
  </a>
  <br><strong>WeChat で支援</strong>
</p>

## 開発とビルド

以下は Windows 向けの手順です。Rust（rustup 経由を推奨）、Visual Studio の「C++ によるデスクトップ開発」ワークロード、Windows SDK をインストールしてください。Rust ツールチェーンのバージョンは `rust-toolchain.toml` で指定されています。

プロジェクトのルートディレクトリで開発版を実行します。

```powershell
cargo run --locked
```

リリース版をビルドします。

```powershell
cargo build --release --locked
```

標準では `target/release/QcClip.exe` に実行ファイルが生成されます。インストーラー、ポータブル版、各アーキテクチャ向けのパッケージ作成は [パッケージ作成ガイド](../docs/packaging.md)（簡体字中国語）をご覧ください。
