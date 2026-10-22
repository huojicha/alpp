# ALPP — Any Language PreProcessor

用注释内嵌的 `#ifdef` 风格指令，为**任意语言**提供 C 宏式的条件编译：
在发布构建时按宏定义裁剪代码块（典型场景：剥离日志与调试代码），而开发树无需任何工具即可通过编译、lint 与 IDE 检查。

```java
//#ifndef RELEASE
log.debug("user {} did {}", userID, action);
//#endif
```

```bash
alpp --write -DRELEASE ./src
```

- 语言：Rust（多平台：linux/darwin/windows × x86_64/aarch64）
- 并行：文件级数据并行（rayon，无共享状态）
- 加速：SIMD 行扫描（memchr，SSE2/AVX2/NEON 运行时派发）
- 安全默认：dry-run diff，`--write` 才落盘，`--check` 供 CI 门禁
- 项目配置：`.alpp` 提供默认宏（优先级低于命令行）并限定允许的指令集（如只许 ifdef/endif），管控业务仓库的宏滥用

## 文档

- [AGENTS.md](AGENTS.md) — AI agent 维护规范（约定、同步链路、已知陷阱）
- [设计文档](docs/DESIGN.md) — 架构、SIMD/并行/跨平台策略、里程碑
- [宏规范 v1](docs/MACRO_SPEC.md) — 指令语法、表达式文法、错误码、与 cpp 差异
- [AI Agent 技能](skills/alpp-macros/SKILL.md) — 宏语法速查与批量改代码工作流，供 AI agent 按规范书写/修改条件编译指令

## 状态

M1 已完成（单线程闭环 + 多线程、完善测试与性能基准，见设计文档 §6 里程碑）；后续 M2–M4 进行中。

## License

[MIT](LICENSE)
