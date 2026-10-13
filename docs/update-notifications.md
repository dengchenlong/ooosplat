# 更新提醒 JSON / Update notification manifest

客户端读取 `https://ooolab.cn/updates/latest.json`。本仓库只实现客户端和格式说明，不部署网站或安装包。

The client reads `https://ooolab.cn/updates/latest.json`. This repository implements the client and documents the format; it does not deploy the website or installers.

## 格式 / Format

以下 `0.6.1` 仅为示例，不代表已经发布。JSON 必须使用 UTF-8，响应不超过 64 KiB。

The version `0.6.1` below is illustrative, not an announcement of an existing release. Serve UTF-8 JSON with a response body of at most 64 KiB.

```json
{
  "schemaVersion": 1,
  "releases": {
    "windows-x86_64": {
      "version": "0.6.1",
      "publishedAt": "2026-10-11T08:00:00Z",
      "notes": {
        "zh-CN": ["新增版本更新提醒。", "修复部分预览问题。"],
        "en": ["Added new-version notifications.", "Fixed several preview issues."]
      },
      "downloads": {
        "github": "https://github.com/ooolabdev/ooosplat/releases/tag/0.6.1",
        "quark": null,
        "quarkCode": null
      }
    }
  }
}
```

- 平台键为 `windows-x86_64`、`macos-aarch64`、`linux-x86_64`，各项采用相同结构，可以使用不同版本。尚未准备好安装包的平台不要添加新版本项。没有当前平台时客户端显示“暂未提供当前平台的版本信息”。
- Platform keys are `windows-x86_64`, `macos-aarch64`, and `linux-x86_64`. Each uses the same structure and can have a different version. Do not advertise a platform update before its installer is ready. Missing platforms are reported as unavailable, not as up to date.
- `version` 使用 SemVer（如 `0.6.10`），也接受前缀 `v`。客户端仅提醒比当前版本新的正式版本；不推荐预发布版本或降级。
- `version` uses SemVer, such as `0.6.10`; an optional `v` prefix is accepted. Only newer stable releases trigger a notification; prereleases and downgrades do not.
- `publishedAt` 为含时区的 ISO 8601 时间；中英文 `notes` 为纯文本数组，不支持 HTML。当前语言没有说明时使用另一语言，两个数组都为空时显示默认提示。
- `publishedAt` is an ISO 8601 timestamp with a timezone. Both `notes` arrays contain plain text, not HTML. Empty notes fall back to the other language or a default message.
- `github` 必填，仅接受 `https://github.com/ooolabdev/ooosplat/releases/tag/…` 或该仓库的 `releases/download/…` 链接。
- `github` is required and must point to an HTTPS release page or release asset in `ooolabdev/ooosplat`.
- `quark` 和 `quarkCode` 可省略或为 `null`。启用夸克时填写真实安装包分享链接，例如 `https://pan.quark.cn/s/<分享ID>`；没有提取码就保留 `null`。**README 中的测试素材分享不是安装包链接，不可用于更新。**
- `quark` and `quarkCode` may be omitted or null. Use a real installer share at `https://pan.quark.cn/s/<share-id>` and leave the code null if none is required. **The sample-media share in README is not an installer link.**

## 发布顺序 / Publishing order

1. 构建并发布各平台安装包，在浏览器中确认 GitHub／夸克下载可用。
2. 更新对应平台的版本、说明和链接，再发布 JSON；建议网站静态 CDN 缓存 5–15 分钟，不使用长期 immutable 缓存。
3. 使用客户端“设置 → 检查更新”验证版本和复制链接。网站尚未部署时，客户端启动检查静默失败；手动检查显示可重试提示。

1. Publish installers and verify the GitHub / Quark downloads in a browser.
2. Publish the corresponding JSON entries. A 5–15 minute CDN cache is suitable; do not mark this mutable manifest as permanently immutable.
3. Verify the result using Settings → Check for updates. Before the endpoint is deployed, startup checks fail silently and manual checks show a retryable message.

## 客户端行为 / Client behavior

- 每次启动检查；24 小时内的成功结果可以使用本地缓存，不轮询。发现新版本自动弹窗，“忽略”仅关闭本次提醒；下次启动仍会提醒同一版本。主界面“设置”旁保留可点击入口。
- Startup checks reuse successful results for 24 hours without polling. A newer release opens a dialog. Ignore dismisses it only for this app session; the next launch can remind again. The main toolbar retains a clickable update badge.
- 手动检查绕过24小时缓存，但有30秒冷却并合并正在执行的请求。请求连接超时3秒、总超时10秒；只允许 HTTPS，最多3次同站点重定向（`ooolab.cn`／`www.ooolab.cn`）。
- Manual checks bypass the 24-hour cache with a 30-second cooldown and shared in-flight requests. Connection timeout is 3 seconds and total request timeout is 10 seconds. Only HTTPS and up to three redirects within `ooolab.cn` / `www.ooolab.cn` are allowed.
- 下载入口只复制文本，不打开浏览器、不下载、不安装。失败、隐私设置等弹窗优先；预览期间延后提醒，生成任务继续运行。
- Download actions only copy text; they do not open a browser, download, or install anything. Other modal dialogs take priority; notifications wait while the preview is open and do not interrupt generation.
- 更新检查独立于匿名统计，不携带项目、素材、安装标识或硬件信息；网站仍会自然收到请求 IP 等普通网络信息。
- Update checks are separate from anonymous statistics and contain no project, media, installation ID, or hardware details. The website still receives normal network information such as the request IP.
