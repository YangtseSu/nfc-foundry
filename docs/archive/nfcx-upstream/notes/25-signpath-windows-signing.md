# SignPath Windows 发布签名

## 实际完成

- Windows release job 改为通过 SignPath GitHub Action 提交签名请求，不再向 CI 导入 Windows PFX 私钥；
- 使用项目 `NFCX`、测试签名策略 `test-signing` 和 Artifact Configuration `nfcx-windows-test-sign`；API token 继续仅从 `SIGNPATH_API_TOKEN` repository secret 读取；
- 先生成临时 unsigned ZIP，SignPath 仅对其明确配置的 EXE/DLL 签名，再将取回的已签名文件放回 Windows staging 目录；
- 签名后才生成 runtime manifest，因此 manifest 的 SHA-256 与最终发布的 Authenticode-signed 文件一致；
- 最终 GitHub Release 仅上传签名后的 `NFCX-<version>-windows-amd64.zip`，临时 `-unsigned` ZIP 不会作为发布 asset 保留；
- workflow 在打包前检查每个分发的 EXE/DLL 是否含嵌入式签名。
- publish job 仅上传 `SHA256SUMS` 与四个预期发布 archive（DMG、AppImage、tar.gz、Windows ZIP）；不再使用会匹配目录的 `dist/*`。
- 发布改为两个 workflow：准备 workflow 只构建、保存跨平台 artifact 并提交 SignPath 请求；批准后的发布 workflow 以 request ID 下载同一份已签名 Windows artifact，再与指定 build run 的 macOS/Linux artifact 合并发布。

## 实现中遇到的问题

- 原 workflow 仅支持从 `WINDOWS_CERTIFICATE` 和 `WINDOWS_CERTIFICATE_PASSWORD` 导入 PFX。SignPath 托管证书不应导出私钥到 GitHub runner，因此该路径已由 SignPath signing request 取代。
- runtime manifest 必须在 Authenticode 写入 PE 文件之后生成；若提前生成，签名会改变文件哈希，应用的 `--self-check` 会错误报告完整性失败。
- SignPath action 的 `skip-decompress` 会将签名后的 ZIP 原样写入输出目录，而 staging 步骤需要 ZIP 内已解压的文件；因此保留默认值 `false`，让 action 在下载后解压输出。
- 初次 SignPath tag 发布中，`dist/*` 还匹配到了一个目录，GitHub CLI 因而上传了目录内的 `NFCX Updater.exe`，又在读取 `LICENSES/` 目录时失败。publish job 现以严格的文件数组和数量断言处理，并清理这一个错误遗留 asset。
- 原 Windows 打包脚本每次打包都会重新编译 `NFCX Updater.exe`。这会在第二次打包时覆盖已由 SignPath 签名的 Updater；现仅在文件尚不存在时构建它。

## 阻塞或未完成

- 无。下一次带 tag 的 GitHub Actions release 将使用 SignPath 提供的 self-signed 测试证书验证完整端到端流程；获得生产证书后，只需在 SignPath 中切换 signing policy slug。
