---
name: ue5-shader-plugin
description: 用于创建和适配 Unreal Engine 5 插件中的 Shader 代码，覆盖 Global Shader（VS/PS）、Compute Shader、Ray Tracing Shader，包括 .usf 文件、C++ Shader 注册、参数结构体、模块初始化和版本差异适配说明。适用于需要脚手架、移植、排错或评审 UE5 插件 Shader 实现（如 IMPLEMENT_GLOBAL_SHADER、SHADER_USE_PARAMETER_STRUCT、AddShaderSourceDirectoryMapping、RDG Compute Dispatch、光追插件集成）时。
---

# UE5 Shader 插件 Skill

用这份 skill 生成或适配 UE5 插件 Shader 代码，覆盖三类常见场景：

1. Global Shader（普通 VS/PS）
2. Compute Shader（优先 RDG，也可 RHI）
3. Ray Tracing Shader（先做 RayGen，再逐步扩展）

优先参考 Epic 官方文档和目标仓库中已能工作的本地示例，不要优先套用泛化教程片段。

## 快速流程

1. Confirm target UE version and module name.
2. Confirm shader type: Global VS/PS, Compute, or Ray Tracing.
3. Inspect the plugin's existing shader patterns before writing code.
4. Add or verify shader source directory mapping in module startup.
5. Create `.usf` shader file in plugin `Shaders/Private/`.
6. Create C++ shader class with parameter struct and `IMPLEMENT_GLOBAL_SHADER`.
7. Add call site (RDG pass / render pass / ray tracing dispatch).
8. Build target module and fix version-specific API mismatches.

建议执行顺序：

1. 确认目标 UE 版本和模块名
2. 确认 Shader 类型（Global / Compute / RayTracing）
3. 先搜插件内现有写法，再动手写
4. 检查或补 `AddShaderSourceDirectoryMapping(...)`
5. 在 `Shaders/Private/` 下创建 `.usf`
6. 写 C++ Shader 类 + 参数结构体 + `IMPLEMENT_GLOBAL_SHADER`
7. 接入调用点（RDG Pass / Render Pass / Ray Tracing Dispatch）
8. 编译目标模块并修正版本 API 差异

## 官方文档锚点

联网时优先看这些（作为主参考）：

1. Epic docs: `Shaders in Plugins`
2. Epic docs: `Creating a New Global Shader as a Plugin`
3. Epic community tutorial: `Creating a Compute Shader - C++`
4. Epic community tutorial: `How to Create a Custom Ray Tracing Shader as a Plugin`

对社区光追示例要默认按“版本相关”处理，几乎一定需要改。

## 先搜本地已有模式

在生成新代码前，先搜索目标插件里已有的 Shader 写法。

可用搜索命令：

```powershell
rg -n "AddShaderSourceDirectoryMapping|IMPLEMENT_GLOBAL_SHADER|DECLARE_GLOBAL_SHADER|SHADER_USE_PARAMETER_STRUCT|SF_RayGen" Source -g "*.cpp" -g "*.h"
```

## 按类型加载参考文件

只读当前任务需要的参考文件：

1. `references/global-shader.md` for regular VS/PS Global Shader
2. `references/compute-shader.md` for Compute Shader (RDG-first)
3. `references/ray-tracing-shader.md` for Ray Tracing Shader (RayGen-first)

## 必查集成项

在说“写完了”之前，必须逐项检查：

1. `*.Build.cs` dependencies include required modules (`RenderCore`, `RHI`, `Renderer`, `Projects`, etc.) for the chosen path.
2. `StartupModule()` registers plugin shader directory mapping with `AddShaderSourceDirectoryMapping(...)`.
3. Shader path matches the plugin mapping (example: `"/Plugin/MyPlugin/Private/MyShader.usf"`).
4. Entry point name in `IMPLEMENT_GLOBAL_SHADER(...)` matches the `.usf` function exactly.
5. `ShouldCompilePermutation(...)` gates correctly for platform and feature support.
6. Ray tracing code is guarded with `#if RHI_RAYTRACING` (or project-specific equivalent).

## 版本适配规则（重要）

UE5 不同小版本的 Shader API 变化较多，按下面规则处理：

1. Prefer local engine/plugin examples from the same branch over online snippets.
2. Keep the high-level structure from docs, but adapt types and dispatch APIs to local code.
3. For ray tracing, do not assume a tutorial for UE5.2/5.3 compiles on UE5.7.1 unchanged.
4. If a symbol is missing, search the engine source for the latest equivalent rather than forcing an old API.

翻译后的执行原则：

1. 同分支本地引擎/插件示例优先于网上片段
2. 文档保留高层结构，类型和 dispatch API 以本地代码为准
3. 光追示例不要假设 UE5.2/5.3 能直接在 UE5.7.1 编译
4. 符号找不到时先搜引擎源码中的新替代接口，不要强行套旧 API

## 使用本 Skill 时的输出要求

当用户让你“写一份 Shader 代码”时，输出至少包含：

1. File list to add or modify (`.cpp`, `.h`, `.usf`, and sometimes `*.Build.cs`)
2. Complete shader class declaration/implementation
3. Minimal shader source (`.usf`) with matching entry point names
4. Call-site integration snippet (RDG pass or render path)
5. Build and validation notes

## 验证清单

1. Build only the affected module first (faster feedback).
2. Check shader compile log for path/entry point mismatches.
3. If compute shader writes output, validate dimensions and thread group count.
4. If ray tracing shader dispatches, validate DX12 + hardware ray tracing project settings.
5. If adapting PRTGI, preserve existing style and avoid breaking current shader mapping.

## 常见失败点

1. Wrong shader virtual path (`/Plugin/...`) after moving `.usf` files
2. Missing `AddShaderSourceDirectoryMapping(...)`
3. `IMPLEMENT_GLOBAL_SHADER` entry point name mismatch
4. Missing module dependencies in `*.Build.cs`
5. Ray tracing code compiled on non-ray-tracing platforms without guards
6. Using tutorial API signatures from a different UE minor version
