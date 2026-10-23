#!/usr/bin/env python3
"""将 GitHub Releases 同步到 Gitee。"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


GITHUB_API = "https://api.github.com"
GITEE_API = "https://gitee.com/api/v5"
GITHUB_REPO = os.environ.get("GITHUB_REPOSITORY", "mosheng1/QcClip")
GITEE_REPO = os.environ["GITEE_REPO"]
GITHUB_TOKEN = os.environ["GH_TOKEN"]
GITEE_TOKEN = os.environ["GITEE_TOKEN"]
EVENT_NAME = os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch")
EVENT_PATH = Path(os.environ["GITHUB_EVENT_PATH"])
REORDER_RELEASES = os.environ.get("REORDER_RELEASES", "false").lower() == "true"
SYNC_TAG = os.environ.get("SYNC_TAG", "").strip()
API_RETRY_COUNT = 4
RETRYABLE_HTTP_STATUS = {408, 429, 500, 502, 503, 504}


def _retry_delay(attempt: int) -> int:
    """使用有限的指数退避，避免短时间内持续冲击 Gitee。"""
    return 2 ** attempt


def _is_retryable_error(error: Exception) -> bool:
    if isinstance(error, urllib.error.HTTPError):
        return error.code in RETRYABLE_HTTP_STATUS
    if isinstance(error, urllib.error.URLError):
        return True
    return isinstance(error, TimeoutError)


def request_json(
    base_url: str,
    path: str,
    token: str,
    method: str = "GET",
    payload: dict | None = None,
    gitee: bool = False,
):
    """调用 API，并在错误信息中隐藏令牌。"""
    url = f"{base_url}{path}"
    if gitee:
        separator = "&" if "?" in url else "?"
        url = f"{url}{separator}access_token={urllib.parse.quote(token)}"
    headers = {"Accept": "application/json", "User-Agent": "QcClip-release-sync"}
    if not gitee:
        headers["Authorization"] = f"Bearer {token}"
    data = None
    if payload is not None:
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    retryable = method in {"GET", "HEAD", "PATCH", "DELETE"}
    for attempt in range(API_RETRY_COUNT):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                content = response.read()
                return json.loads(content) if content else None
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
            if not retryable or not _is_retryable_error(error) or attempt == API_RETRY_COUNT - 1:
                if isinstance(error, urllib.error.HTTPError):
                    detail = error.read().decode("utf-8", errors="replace")[:500]
                    raise RuntimeError(
                        f"API 请求失败：{method} {path}，HTTP {error.code}，{detail}"
                    ) from error
                reason = error.reason if isinstance(error, urllib.error.URLError) else error
                raise RuntimeError(f"网络请求失败：{method} {path}，{reason}") from error
            delay = _retry_delay(attempt)
            print(
                f"API 请求暂时失败，将在 {delay} 秒后重试（{attempt + 1}/{API_RETRY_COUNT - 1}）："
                f"{method} {path}",
                flush=True,
            )
            time.sleep(delay)


def request_form(path: str, token: str, method: str, payload: dict) -> dict:
    """按 Gitee API 要求以 formData 提交 Release 参数。"""
    url = f"{GITEE_API}{path}?access_token={urllib.parse.quote(token)}"
    data = urllib.parse.urlencode(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        headers={
            "Accept": "application/json",
            "Content-Type": "application/x-www-form-urlencoded",
            "User-Agent": "QcClip-release-sync",
        },
        method=method,
    )
    retryable = method in {"HEAD", "PATCH", "DELETE"}
    for attempt in range(API_RETRY_COUNT):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                content = response.read()
                return json.loads(content) if content else {}
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
            if not retryable or not _is_retryable_error(error) or attempt == API_RETRY_COUNT - 1:
                if isinstance(error, urllib.error.HTTPError):
                    detail = error.read().decode("utf-8", errors="replace")[:500]
                    raise RuntimeError(
                        f"API 请求失败：{method} {path}，HTTP {error.code}，{detail}"
                    ) from error
                reason = error.reason if isinstance(error, urllib.error.URLError) else error
                raise RuntimeError(f"网络请求失败：{method} {path}，{reason}") from error
            delay = _retry_delay(attempt)
            print(
                f"API 请求暂时失败，将在 {delay} 秒后重试（{attempt + 1}/{API_RETRY_COUNT - 1}）："
                f"{method} {path}",
                flush=True,
            )
            time.sleep(delay)


def github_releases() -> list[dict]:
    """按本次检出的标签逐个读取 Release，规避列表接口返回空数组的问题。"""
    tag_output = subprocess.run(
        ["git", "tag", "--list"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    tags = sorted(filter(None, tag_output.splitlines()))
    if SYNC_TAG:
        if SYNC_TAG not in tags:
            raise RuntimeError(f"检出内容中没有指定标签：{SYNC_TAG}")
        tags = [SYNC_TAG]

    releases = []
    for tag in tags:
        encoded_tag = urllib.parse.quote(tag, safe="")
        try:
            release = request_json(
                GITHUB_API,
                f"/repos/{GITHUB_REPO}/releases/tags/{encoded_tag}",
                GITHUB_TOKEN,
            )
            if release.get("draft"):
                print(f"Release 仍是草稿，跳过同步：{tag}", flush=True)
                continue
            releases.append(release)
        except RuntimeError as error:
            if "HTTP 404" in str(error):
                print(f"标签没有对应 GitHub Release，跳过：{tag}", flush=True)
                continue
            raise
    return releases


def gitee_releases() -> list[dict]:
    releases = []
    page = 1
    while True:
        batch = request_json(
            GITEE_API,
            f"/repos/{GITEE_REPO}/releases?per_page=100&page={page}",
            GITEE_TOKEN,
            gitee=True,
        )
        if not batch:
            return releases
        releases.extend(batch)
        if len(batch) < 100:
            return releases
        page += 1


def event_release() -> dict | None:
    if EVENT_NAME == "workflow_dispatch":
        return None
    event = json.loads(EVENT_PATH.read_text(encoding="utf-8"))
    return event.get("release")


def release_assets(release_id: int) -> list[dict]:
    return request_json(
        GITEE_API,
        f"/repos/{GITEE_REPO}/releases/{release_id}/attach_files",
        GITEE_TOKEN,
        gitee=True,
    ) or []


def delete_asset(release_id: int, asset_id: int) -> None:
    request_json(
        GITEE_API,
        f"/repos/{GITEE_REPO}/releases/{release_id}/attach_files/{asset_id}",
        GITEE_TOKEN,
        method="DELETE",
        gitee=True,
    )


def download_asset(asset: dict, target: Path) -> None:
    download_url = asset.get("browser_download_url") or asset["url"]
    try:
        subprocess.run(
            [
                "curl",
                "--fail-with-body",
                "--silent",
                "--show-error",
                "--location",
                "--retry",
                "3",
                "--retry-all-errors",
                "--retry-delay",
                "5",
                "--http1.1",
                "--header",
                "Expect:",
                "--connect-timeout",
                "30",
                "--max-time",
                "300",
                "--user-agent",
                "QcClip-release-sync",
                "--output",
                str(target),
                download_url,
            ],
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()[-500:]
        raise RuntimeError(f"下载附件失败：{asset['name']}，{detail}") from error


def upload_asset(release_id: int, asset: dict, path: Path) -> None:
    url = (
        f"{GITEE_API}/repos/{GITEE_REPO}/releases/{release_id}/attach_files"
        f"?access_token={urllib.parse.quote(GITEE_TOKEN)}"
    )
    try:
        subprocess.run(
            [
                "curl",
                "--fail-with-body",
                "--silent",
                "--show-error",
                "--retry",
                "3",
                "--retry-all-errors",
                "--retry-delay",
                "5",
                "--http1.1",
                "--header",
                "Expect:",
                "--connect-timeout",
                "30",
                "--max-time",
                "300",
                "--user-agent",
                "QcClip-release-sync",
                "--form",
                f"access_token={GITEE_TOKEN}",
                "--form",
                f"file=@{path};filename={asset['name']}",
                url,
            ],
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()[-500:]
        raise RuntimeError(f"上传附件失败：{asset['name']}，{detail}") from error


def sync_assets(github_release: dict, gitee_release: dict) -> None:
    existing = {asset["name"]: asset for asset in release_assets(gitee_release["id"])}
    with tempfile.TemporaryDirectory(prefix="qcclip-release-") as directory:
        pending = []
        for asset in github_release.get("assets", []):
            old = existing.get(asset["name"])
            if old and old.get("size") == asset.get("size"):
                print(f"附件已存在，跳过：{asset['name']}", flush=True)
                continue
            if old:
                delete_asset(gitee_release["id"], old["id"])
                print(f"已删除旧附件：{asset['name']}", flush=True)
            path = Path(directory) / asset["name"]
            pending.append((asset, path))

        def sync_one(asset: dict, path: Path) -> None:
            download_asset(asset, path)
            upload_asset(gitee_release["id"], asset, path)
            print(f"已上传附件：{asset['name']}", flush=True)

        if not pending:
            return
        errors = []
        with ThreadPoolExecutor(max_workers=min(4, len(pending))) as executor:
            futures = [executor.submit(sync_one, asset, path) for asset, path in pending]
            for future in as_completed(futures):
                try:
                    future.result()
                except Exception as error:  # noqa: BLE001 - 汇总并继续其他附件
                    errors.append(str(error))
        if errors:
            raise RuntimeError("；".join(errors))


def sync_release(github_release: dict, existing: list[dict]) -> None:
    tag = github_release["tag_name"]
    current = next((release for release in existing if release.get("tag_name") == tag), None)
    payload = {
        "tag_name": tag,
        "name": github_release.get("name") or tag,
        "body": github_release.get("body") or "",
        "prerelease": bool(github_release.get("prerelease", False)),
    }
    if current:
        current = request_form(
            f"/repos/{GITEE_REPO}/releases/{current['id']}",
            GITEE_TOKEN,
            method="PATCH",
            payload=payload,
        )
        print(f"已更新 Release：{tag}", flush=True)
    else:
        current = request_form(
            f"/repos/{GITEE_REPO}/releases",
            GITEE_TOKEN,
            method="POST",
            payload={
                **payload,
                "target_commitish": github_release.get("target_commitish") or tag,
            },
        )
        print(f"已创建 Release：{tag}", flush=True)
    sync_assets(github_release, current)


def delete_stale_releases(github_releases_list: list[dict], gitee_releases_list: list[dict]) -> None:
    """手动全量同步时删除 Gitee 中已从 GitHub 删除的 Release。"""
    github_tags = {release["tag_name"] for release in github_releases_list}
    for release in gitee_releases_list:
        if release.get("tag_name") in github_tags:
            continue
        request_json(
            GITEE_API,
            f"/repos/{GITEE_REPO}/releases/{release['id']}",
            GITEE_TOKEN,
            method="DELETE",
            gitee=True,
        )
        print(f"已删除多余 Gitee Release：{release.get('tag_name', release['id'])}", flush=True)


def delete_all_releases(releases: list[dict]) -> None:
    """删除 Gitee Release 对象，供一次性重建排序使用。"""
    for release in releases:
        request_json(
            GITEE_API,
            f"/repos/{GITEE_REPO}/releases/{release['id']}",
            GITEE_TOKEN,
            method="DELETE",
            gitee=True,
        )
        print(f"已删除待重建 Release：{release.get('tag_name', release['id'])}", flush=True)


def main() -> None:
    if not GITEE_TOKEN:
        raise RuntimeError("环境变量 GITEE_TOKEN 为空")
    if SYNC_TAG and REORDER_RELEASES:
        raise RuntimeError("指定版本同步不能同时重建全部 Release 顺序")
    target_releases = gitee_releases()
    event = event_release()
    if EVENT_NAME == "release" and event and event.get("action") == "deleted":
        tag = event["tag_name"]
        current = next((release for release in target_releases if release.get("tag_name") == tag), None)
        if current:
            request_json(
                GITEE_API,
                f"/repos/{GITEE_REPO}/releases/{current['id']}",
                GITEE_TOKEN,
                method="DELETE",
                gitee=True,
            )
            print(f"已删除 Gitee Release：{tag}", flush=True)
        else:
            print(f"Gitee 没有对应 Release，无需删除：{tag}", flush=True)
        return

    if EVENT_NAME == "release" and event and event.get("draft"):
        print(f"Release 仍是草稿，跳过同步：{event.get('tag_name', '未知版本')}", flush=True)
        return

    if EVENT_NAME == "release" and event:
        release_id = event["id"]
        releases = [
            request_json(
                GITHUB_API,
                f"/repos/{GITHUB_REPO}/releases/{release_id}",
                GITHUB_TOKEN,
            )
        ]
    else:
        releases = github_releases()
        if REORDER_RELEASES:
            delete_all_releases(target_releases)
            target_releases = []
        elif not SYNC_TAG:
            delete_stale_releases(releases, target_releases)
    errors = []
    for release in releases:
        try:
            sync_release(release, target_releases)
        except Exception as error:  # noqa: BLE001 - 继续处理其他版本后统一失败
            errors.append(f"{release.get('tag_name', '未知版本')}：{error}")
            print(f"同步版本失败，继续处理后续版本：{errors[-1]}", flush=True)
    if errors:
        raise RuntimeError("；".join(errors))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:  # noqa: BLE001 - 工作流需要输出可读的失败原因
        print(f"同步失败：{error}", file=sys.stderr)
        sys.exit(1)
