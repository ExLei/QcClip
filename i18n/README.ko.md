<p align="center">
  <img src="../assets/icon128.png" alt="QcClip" width="96" height="96">
</p>

<h1 align="center">QcClip</h1>

<p align="center">순수 Rust · GPUI 네이티브 렌더링 · 고성능 클립보드 도구</p>

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
  · <a href="README.ja.md">日本語</a>
  · <b>한국어</b>
</p>

**QcClip은 개발자가 직접 만든 [QuickClipboard](https://github.com/mosheng1/QuickClipboard)를 순수 Rust로 다시 작성한 고성능 클립보드 도구입니다. GPUI 프레임워크를 기반으로 네이티브 화면을 렌더링합니다.**

새 내용을 복사해도 이전 내용을 다시 찾아다닐 필요가 없습니다. QcClip은 클립보드 기록을 저장하고 자주 쓰는 답변, 링크, 이미지, 파일을 한곳에 모아 줍니다. 필요할 때 찾아 바로 붙여넣을 수 있습니다.

이번 재작성은 QuickClipboard의 사용 경험을 이어가면서 응답 속도, 부드러운 탐색, 일상적인 조작의 편리함에 중점을 두었습니다. 매일 가볍고 편하게 사용할 수 있는 클립보드 도구를 지향합니다.

현재 주로 **Windows**를 지원하며, macOS와 Linux 지원은 개발 중입니다.

## QcClip을 선택하는 이유

### 매일 켜 두기 좋은 가볍고 빠른 도구

클립보드 도구는 하루에도 수십 번 열 수 있어 응답 속도와 스크롤이 중요합니다. QcClip은 **화면 표시에 WebView나 브라우저 엔진을 사용하지 않는 네이티브 UI**로 불필요한 실행 부담을 줄입니다. 많은 기록, 이미지 미리보기, 백그라운드 실행도 최적화해 내용을 찾고 붙여넣을 때 기다리는 시간을 줄입니다.

- **기록이 많아도 편리한 탐색**: 긴 목록의 표시와 로딩을 최적화해 오래된 내용을 찾을 때의 대기를 줄입니다.
- **이미지가 많아도 자원 사용을 고려**: 보고 있는 내용을 우선 처리하고 필요 없어진 자원을 해제해 메모리 부담을 줄입니다.
- **같은 내용의 중복 저장 감소**: 동일한 내용을 반복해서 복사할 때 불필요한 저장을 줄여 기록을 정리하고 디스크 공간도 절약합니다.

### 복사한 내용을 더 쉽게 다시 찾기

자료를 정리하며 복사한 문장, 방금 본 링크, 보내려던 이미지까지 기록에서 찾을 수 있습니다. 웹페이지나 대화 내용을 다시 뒤질 필요가 없습니다.

- **빠른 검색**: 내용, 메모, 복사한 앱 이름으로 검색 범위를 좁힐 수 있습니다.
- **사용 전에 확인**: 텍스트, 이미지, 파일, 링크별로 필터링하고 마우스를 올려 미리 본 뒤 선택할 수 있습니다.
- **원하는 형태로 붙여넣기**: 서식이 필요하면 유지하고, 필요 없으면 일반 텍스트로 붙여넣습니다. 이미지는 이미지 파일로도 붙여넣을 수 있습니다.

### 클릭과 창 전환 줄이기

단축키로 창을 열고 내용을 골라 붙여넣으세요. 자주 쓰는 내용은 단축키로 바로 붙여넣을 수도 있어 반복 조작을 줄일 수 있습니다.

- **간편 붙여넣기 창**: 작은 창에서 방향키로 항목을 고르고 기록과 즐겨찾기를 전환할 수 있습니다.
- **단축키 붙여넣기 1~9**: 숫자 키나 F1~F9 조합을 설정해 해당 항목을 바로 붙여넣을 수 있습니다.
- **여러 항목을 한 번에**: 여러 문장, 텍스트와 이미지, 여러 파일을 합쳐 복사하거나 붙여넣을 수 있습니다. 파일과 텍스트를 섞어 합치는 기능은 지원하지 않습니다.
- **가까이 두고 작업은 방해하지 않게**: 화면 가장자리에 숨기기, 마우스 근처에서 열기, 창 고정을 지원합니다. 한 번 클릭, 두 번 클릭, 가운데 클릭의 동작도 설정할 수 있습니다.

### 한 번 정리하고 여러 번 활용

고객 응대 문구, 자주 쓰는 주소, 업무 링크, 참고 자료를 나만의 콘텐츠 모음으로 정리하세요. 필요할 때 바로 꺼내 쓸 수 있어 원문을 매번 찾지 않아도 됩니다.

- **즐겨찾기와 상단 고정**: 오래 쓸 내용을 저장하고 자주 쓰는 항목을 위에 둘 수 있습니다. 드래그로 순서를 바꿀 수 있습니다.
- **태그 분류**: 업무, 개인 용도, 프로젝트별로 정리할 수 있습니다. 한 항목에 여러 태그를 달아 유연하게 찾을 수 있습니다.
- **메모와 편집**: 알아보기 쉬운 설명을 붙이거나 자주 쓰는 텍스트를 추가·수정할 수 있습니다. 캡처한 원래 내용으로 되돌릴 수도 있습니다.

### 이모지, 기호, 이미지 소재를 한곳에서

채팅용 이모지, 문서에 넣을 특수 기호, 메시지에 첨부할 이미지를 QcClip에서 찾을 수 있습니다.

- **이모지와 기호**: 분류, 검색, 최근 사용 기록을 지원하며 선택하면 바로 붙여넣을 수 있습니다.
- **나만의 갤러리**: 로컬 파일이나 브라우저의 이미지를 그룹으로 끌어와 자주 쓰는 이미지와 반응 이미지를 모아 두세요. PNG, JPG, GIF, WebP를 지원합니다.
- **편리한 소재 정리**: 파일 이름으로 검색하고 이름을 바꾸거나 그룹 간에 이동할 수 있습니다. 이름, 종류, 날짜별 정렬도 지원합니다.

### 데이터도 모양도 내 방식대로

계정 로그인 없이 사용할 수 있으며, 클립보드 기록과 이미지 소재는 내 기기에 저장됩니다. 보관 기간, 백업 위치, 창 모양을 원하는 대로 설정할 수 있습니다.

- **보관과 정리를 직접 관리**: 기록 보관 기간과 개수를 설정하세요. 범위를 벗어난 내용은 정리 대기로 표시되며, 정리 전에 설정을 조정해 다시 표시할 수 있습니다.
- **편리한 백업과 기기 이동**: 데이터 내보내기와 가져오기를 지원합니다. 포터블 버전은 데이터와 함께 옮길 수 있고 설치 버전은 데이터 위치를 바꿀 수 있습니다.
- **간섭 줄이기**: 지정한 앱에서 내용 기록, 단축키, 화면 가장자리 표시를 일시 중지해 개인정보와 작업 습관을 고려할 수 있습니다.
- **취향에 맞는 화면**: 밝은 테마와 어두운 테마, 배경 이미지, 투명도, 목록 크기, 애니메이션을 조정하고 시스템 테마를 따르게 할 수 있습니다.
- **일상에 편리한 기능**: 자동 시작, 시스템 트레이 실행, 중국어·영어 UI, 조작 효과음, 업데이트 확인을 지원합니다.

## 화면 미리보기

### 클립보드와 태그

<table>
  <tr>
    <td align="center" width="50%">
      <a href="../readme-assets/display/剪贴板.png">
        <img src="../readme-assets/display/剪贴板.png" alt="텍스트, 이미지, 파일을 모아 보는 클립보드 기록" width="360">
      </a>
      <br><strong>클립보드 기록</strong><br>
      텍스트, 이미지, 파일을 언제든 다시 사용
    </td>
    <td align="center" width="50%">
      <a href="../readme-assets/display/标签.png">
        <img src="../readme-assets/display/标签.png" alt="용도별로 내용을 정리하는 색상 태그" width="360">
      </a>
      <br><strong>색상 태그</strong><br>
      용도별 분류로 더 쉽게 검색
    </td>
  </tr>
</table>

### 이모지, 기호, 갤러리

<table>
  <tr>
    <td align="center" width="33%">
      <a href="../readme-assets/display/Eomji.png">
        <img src="../readme-assets/display/Eomji.png" alt="분류와 검색을 지원하는 이모지 모음" width="240">
      </a>
      <br><strong>이모지</strong><br>
      자주 쓰는 이모지를 바로 입력
    </td>
    <td align="center" width="33%">
      <a href="../readme-assets/display/符号.png">
        <img src="../readme-assets/display/符号.png" alt="특수 기호를 찾고 입력할 수 있는 기호 모음" width="240">
      </a>
      <br><strong>기호</strong><br>
      특수 기호를 한곳에서
    </td>
    <td align="center" width="34%">
      <a href="../readme-assets/display/图库.png">
        <img src="../readme-assets/display/图库.png" alt="이미지 소재를 그룹별로 관리하는 갤러리" width="240">
      </a>
      <br><strong>갤러리</strong><br>
      이미지 소재를 그룹별로 보관
    </td>
  </tr>
</table>

### 마우스를 올려 내용 미리보기

항목에 마우스를 올리면 내용을 확인할 수 있어 따로 여는 수고를 줄여 줍니다.

<p align="center">
  <a href="../readme-assets/display/悬停内容预览.webp">
    <img src="../readme-assets/display/悬停内容预览.webp" alt="텍스트, 이미지, 파일의 마우스 오버 미리보기 애니메이션" width="960">
  </a>
</p>

### 다양한 테마 색상

밝은 색부터 어두운 색까지 원하는 테마를 골라 매일 켜 두는 도구를 편안하게 꾸밀 수 있습니다.

<p align="center">
  <a href="../readme-assets/display/丰富主题配色.webp">
    <img src="../readme-assets/display/丰富主题配色.webp" alt="테마 색상과 화면 설정을 바꾸는 애니메이션" width="960">
  </a>
</p>

## 시작하기

[다운로드 페이지](https://github.com/mosheng1/QcClip/releases)에서 원하는 버전을 선택하세요.

- **설치 버전**: 주로 쓰는 기기에서 일상적으로 사용하기 좋습니다.
- **포터블 버전**: 압축을 풀고 `QcClip/QcClip.exe`를 실행하세요. 데이터는 같은 위치의 `QcClipData` 폴더에 자동 저장됩니다. 이동하거나 업데이트할 때 이 폴더를 유지하세요.

기본 단축키 **`Ctrl + Shift + X`**로 기본 창을 표시하거나 숨길 수 있습니다. 내용을 복사한 뒤 창에서 검색·필터링하고 항목을 선택하면 다시 복사하거나 붙여넣을 수 있습니다. 간편 붙여넣기와 다른 단축키는 설정 → 단축키에서 지정할 수 있습니다.

### QuickClipboard에서 데이터 이전

설정 → 데이터 관리 → 이전 QuickClipboard에서 가져오기를 여세요. 로컬 설치 버전의 데이터를 감지하거나 이전 데이터 폴더·포터블 폴더를 선택할 수 있으며, 이전 버전에서 내보낸 ZIP도 읽을 수 있습니다.

즐겨찾기를 이전할 수 있고 클립보드 기록은 선택해서 가져올 수 있습니다. 기존 그룹은 태그로 바뀌고 즐겨찾기 제목은 메모에 추가됩니다. 저장된 텍스트, 이미지, 파일 목록도 함께 이전할 수 있습니다. 현재 QuickClipboard **v0.5.0** 데이터를 기준으로 지원합니다.

이전 전에 가져올 수 있는 내용을 확인할 수 있으며, 실행 시 QcClip의 현재 데이터를 자동 백업합니다. **이전 버전의 갤러리, 설정, 동기화 정보는 아직 이전하지 않습니다.** 파일 목록은 위치만 유지하며 실제 파일은 복사하지 않습니다. 자세한 내용은 [데이터 이전 안내](../docs/legacy-migration.md)(중국어 간체)를 참고하세요.

## 의견 및 제안

문제를 발견했거나 개선 아이디어가 있다면 [이슈 페이지](https://github.com/mosheng1/QcClip/issues)에 남겨 주세요.

## 커뮤니티

채널이나 그룹에 참여해 사용 경험과 제안을 나누고 개발 소식을 확인하세요. 아래 링크로 참여하거나 QQ로 QR 코드를 스캔할 수 있습니다.

<table>
  <tr>
    <td align="center" width="33%">
      <strong>QcClip 채널</strong><br>
      채널 ID: pd80680380<br><br>
      <a href="../assets/pD_1.png">
        <img src="../assets/pD_1.png" alt="QcClip 채널 QR 코드, ID pd80680380" width="200">
      </a>
      <br><a href="https://pd.qq.com/s/blp3j847c">채널 참여</a>
    </td>
    <td align="center" width="33%">
      <strong>QcClip 대화 그룹 1</strong><br>
      그룹 ID: 725313287<br><br>
      <a href="../assets/qG_1.png">
        <img src="../assets/qG_1.png" alt="QcClip 대화 그룹 1 QR 코드, ID 725313287" width="200">
      </a>
      <br><a href="https://qm.qq.com/q/nUCO76MX9C">그룹 1 참여</a>
    </td>
    <td align="center" width="34%">
      <strong>QcClip 대화 그룹 2</strong><br>
      그룹 ID: 1033556729<br><br>
      <a href="../assets/qG_2.png">
        <img src="../assets/qG_2.png" alt="QcClip 대화 그룹 2 QR 코드, ID 1033556729" width="200">
      </a>
      <br><a href="https://qm.qq.com/q/O5zOi3uTuy">그룹 2 참여</a>
    </td>
  </tr>
</table>

## 개발 후원

QcClip이 유용하다면 WeChat 후원으로 앞으로의 개발과 유지보수를 지원할 수 있습니다.

<p align="center">
  <a href="../assets/wxzsm.png">
    <img src="../assets/wxzsm.png" alt="WeChat 후원 코드" width="240">
  </a>
  <br><strong>WeChat 후원</strong>
</p>

## 개발 및 빌드

다음은 Windows용 개발 및 빌드 방법입니다. Rust(rustup 사용 권장), Visual Studio의 “C++를 사용한 데스크톱 개발” 워크로드, Windows SDK를 설치하세요. Rust 도구 모음 버전은 `rust-toolchain.toml`에 지정되어 있습니다.

프로젝트 루트에서 개발 버전을 실행합니다.

```powershell
cargo run --locked
```

릴리스 버전을 빌드합니다.

```powershell
cargo build --release --locked
```

기본 실행 파일은 `target/release/QcClip.exe`에 생성됩니다. 설치 패키지, 포터블 버전, 아키텍처별 패키징 방법은 [패키징 안내](../docs/packaging.md)(중국어 간체)를 참고하세요.
