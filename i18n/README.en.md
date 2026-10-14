<p align="center">
  <img src="../assets/icon128.png" alt="QcClip" width="96" height="96">
</p>

<h1 align="center">QcClip</h1>

<p align="center">Pure Rust · Native GPUI rendering · High-performance clipboard tool</p>

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
  · <b>English</b>
  · <a href="README.zh-TW.md">繁體中文</a>
  · <a href="README.ja.md">日本語</a>
  · <a href="README.ko.md">한국어</a>
</p>

**QcClip is a rewrite of the author's own [QuickClipboard](https://github.com/mosheng1/QuickClipboard), a high-performance clipboard tool written entirely in Rust with native rendering powered by GPUI.**

Copying something new no longer means losing what you copied before. QcClip keeps your clipboard history and puts frequently used replies, links, images, and files within reach, ready to find and paste whenever you need them.

This rewrite carries forward the QuickClipboard experience, with a focus on responsiveness, smooth browsing, and convenient everyday use.

Currently, **Windows** is the primary supported platform. Support for macOS and Linux is still in progress.

## Why QcClip

### Light and responsive, built for everyday use

A clipboard tool may be opened dozens of times a day, so responsiveness and scrolling matter. QcClip uses a native interface that **does not depend on WebView or a browser engine**, reducing extra runtime overhead. It also includes optimizations for long histories, image previews, and running in the background, helping you spend less time waiting.

- **Browse long histories comfortably**: Display and loading are optimized for long lists, reducing delays when looking through older content.
- **Keep image browsing efficient**: Content currently in view takes priority, while resources no longer needed are released to reduce memory use.
- **Avoid redundant storage**: Repeated copies of identical content share storage, keeping history tidier and saving disk space.

### Find what you copied more easily

Text copied while collecting information, a link you just saw, or an image you were about to send can all be found in your history, without reopening websites or digging through chats.

- **Search quickly**: Search by content, note, or source application name to narrow down the results.
- **See before you paste**: Filter by text, image, file, or link, then hover to preview an item before using it.
- **Paste the way you need**: Keep formatting when you need it, paste as plain text when you do not, or paste an image as an image file.

### Fewer clicks, less window switching

Bring up the window with a shortcut, select an item, and paste. Frequently used content can also be pasted directly with keyboard shortcuts, reducing repetitive steps.

- **Quick Paste window**: Pick items with the arrow keys in a compact window and switch between history and favorites.
- **Direct paste 1–9**: Configure number-key or F1–F9 combinations to paste the corresponding items directly.
- **Use several items at once**: Combine text snippets, text and images, or multiple files for copying or pasting. Files and text cannot be combined in the same operation.
- **Within reach, out of the way**: Hide the window at a screen edge, open it near the pointer, or keep it fixed. Customize single-click, double-click, and middle-click actions to suit your habits.

### Organize once, reuse often

Customer support replies, addresses, work links, and reference snippets can become your own reusable content collection. Retrieve what you need without finding the original every time.

- **Favorites and pinning**: Save content for long-term use, keep frequent items at the top, and drag to reorder them.
- **Tags**: Organize by work, personal use, or project. Give an item several tags for more flexible browsing.
- **Notes and editing**: Add a recognizable description, create or edit reusable text, and restore the captured original content when needed.

### Emoji, symbols, and images in one place

Find an emoji for a chat, a special symbol for a document, or an image for a message, all within QcClip.

- **Emoji and symbols**: Browse categories, search, and revisit recently used items. Select an item to paste it.
- **Your own gallery**: Drag images from local files or a browser into groups to keep images and reaction pictures together. PNG, JPG, GIF, and WebP are supported.
- **Easy organization**: Search by filename, rename images, move them between groups, and sort by name, type, or date.

### Your data, your preferences

No account is required. Clipboard history and image collections stay on your device. Choose how long to keep content, where to back it up, and how the window looks.

- **Control retention and cleanup**: Set history age and item limits. Items outside those limits are marked for cleanup; you can adjust the limits to bring them back before cleanup.
- **Back up or move devices**: Export and import your data. The portable version can move together with its data, and the installed version lets you change the data location.
- **Reduce interruptions**: Pause recording, shortcuts, or edge-triggered display for selected applications to suit your privacy needs and workflow.
- **Make it yours**: Customize light and dark themes, background images, opacity, list sizing, and animations, or follow your system appearance.
- **Everyday conveniences**: Launch at login, run in the system tray, use Chinese or English UI, enable action sounds, and check for updates.

## Preview

### Clipboard and tags

<table>
  <tr>
    <td align="center" width="50%">
      <a href="../readme-assets/display/剪贴板.png">
        <img src="../readme-assets/display/剪贴板.png" alt="Clipboard history with text, images, and files" width="360">
      </a>
      <br><strong>Clipboard history</strong><br>
      Find text, images, and files again
    </td>
    <td align="center" width="50%">
      <a href="../readme-assets/display/标签.png">
        <img src="../readme-assets/display/标签.png" alt="Color tags for organizing reusable content" width="360">
      </a>
      <br><strong>Color tags</strong><br>
      Organize by purpose, find things faster
    </td>
  </tr>
</table>

### Emoji, symbols, and gallery

<table>
  <tr>
    <td align="center" width="33%">
      <a href="../readme-assets/display/Eomji.png">
        <img src="../readme-assets/display/Eomji.png" alt="Emoji categories and search" width="240">
      </a>
      <br><strong>Emoji</strong><br>
      Favorite expressions within reach
    </td>
    <td align="center" width="33%">
      <a href="../readme-assets/display/符号.png">
        <img src="../readme-assets/display/符号.png" alt="Special symbols to find and paste" width="240">
      </a>
      <br><strong>Symbols</strong><br>
      Special characters in one place
    </td>
    <td align="center" width="34%">
      <a href="../readme-assets/display/图库.png">
        <img src="../readme-assets/display/图库.png" alt="Image gallery organized into groups" width="240">
      </a>
      <br><strong>Gallery</strong><br>
      Keep image collections organized
    </td>
  </tr>
</table>

### Hover previews

Hover over an item to see its content, saving you an extra step.

<p align="center">
  <a href="../readme-assets/display/悬停内容预览.webp">
    <img src="../readme-assets/display/悬停内容预览.webp" alt="Animated demonstration of text, image, and file hover previews" width="960">
  </a>
</p>

### A choice of themes

Choose a light or dark color scheme that makes an everyday tool comfortable to look at.

<p align="center">
  <a href="../readme-assets/display/丰富主题配色.webp">
    <img src="../readme-assets/display/丰富主题配色.webp" alt="Animated demonstration of themes and appearance settings" width="960">
  </a>
</p>

## Get started

Visit the [download page](https://github.com/mosheng1/QcClip/releases) and choose a version:

- **Installer**: For everyday use on a regular device.
- **Portable**: Extract the archive and run `QcClip/QcClip.exe`. Data is saved in the adjacent `QcClipData` folder. Keep that folder when moving or upgrading the app.

By default, **`Ctrl + Shift + X`** shows or hides the main window. After copying content, search or filter the history and select an item to copy or paste it again. Configure Quick Paste and other shortcuts in Settings → Shortcuts.

### Migrate from QuickClipboard

Open Settings → Data Management → Migrate from legacy QuickClipboard. You can detect locally installed data, choose an old data or portable folder, or select a ZIP exported by the old version.

Favorites can be migrated, with clipboard history included optionally. Existing groups become tags, favorite titles are added to notes, and saved text, images, and file lists can be brought over. Migration currently targets data from QuickClipboard **v0.5.0**.

You can review the migration results before proceeding, and QcClip backs up its current data before importing. **The old gallery, settings, and sync information are not migrated yet.** File lists retain file locations without copying the actual files. See the [migration guide](../docs/legacy-migration.md) for details (in Simplified Chinese).

## Feedback and suggestions

Found a problem or have an idea for improvement? Share it on the [issues page](https://github.com/mosheng1/QcClip/issues).

## Community

Join the channel or a chat group to discuss your experience, suggest improvements, and follow development. Use the links below or scan a QR code with QQ.

<table>
  <tr>
    <td align="center" width="33%">
      <strong>QcClip channel</strong><br>
      Channel ID: pd80680380<br><br>
      <a href="../assets/pD_1.png">
        <img src="../assets/pD_1.png" alt="QcClip channel QR code, ID pd80680380" width="200">
      </a>
      <br><a href="https://pd.qq.com/s/blp3j847c">Join channel</a>
    </td>
    <td align="center" width="33%">
      <strong>QcClip group 1</strong><br>
      Group ID: 725313287<br><br>
      <a href="../assets/qG_1.png">
        <img src="../assets/qG_1.png" alt="QcClip QQ group 1 QR code, ID 725313287" width="200">
      </a>
      <br><a href="https://qm.qq.com/q/nUCO76MX9C">Join group 1</a>
    </td>
    <td align="center" width="34%">
      <strong>QcClip group 2</strong><br>
      Group ID: 1033556729<br><br>
      <a href="../assets/qG_2.png">
        <img src="../assets/qG_2.png" alt="QcClip QQ group 2 QR code, ID 1033556729" width="200">
      </a>
      <br><a href="https://qm.qq.com/q/O5zOi3uTuy">Join group 2</a>
    </td>
  </tr>
</table>

## Support development

Enjoying QcClip? You can support continued development and maintenance with a WeChat donation.

<p align="center">
  <a href="../assets/wxzsm.png">
    <img src="../assets/wxzsm.png" alt="WeChat donation code" width="240">
  </a>
  <br><strong>WeChat donation</strong>
</p>

## Development and build

The following instructions are for Windows. Install Rust (preferably through rustup), the Visual Studio “Desktop development with C++” workload, and the Windows SDK. The Rust toolchain is specified in `rust-toolchain.toml`.

Run the development version from the project root:

```powershell
cargo run --locked
```

Build a release version:

```powershell
cargo build --release --locked
```

By default, the executable is generated at `target/release/QcClip.exe`. See the [packaging guide](../docs/packaging.md) for installers, portable builds, and other architectures (in Simplified Chinese).
