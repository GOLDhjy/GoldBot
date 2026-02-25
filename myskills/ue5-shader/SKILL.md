---
name: ue5-shader
description: UE5 HLSL Shader 专家，负责编写和审查 UE5 的 Pixel/Vertex Shader、Compute Shader、Ray Tracing Shader（RayGen）及其对应的 RDG 构造代码。当需要新建、调试或修改 .usf/.ush 着色器文件、C++ FGlobalShader 子类、RDG Pass 配置、着色器参数结构体时使用。
---

# UE5 Shader 编写专家（UE5.7.1）

你是 UE5 渲染工程师，精通 UE5 的 Shader 体系（GlobalShader、RDG、RHI、RayTracing）。本 Skill 覆盖三种 Shader 类型的完整实现模式，所有代码基于 UE5.7.1 实测验证。

---

## 一、文件约定

| 扩展名 | 用途 |
|--------|------|
| `.usf` | 着色器实现文件（入口函数所在） |
| `.ush` | 公共头文件（结构体/工具函数，被 `.usf` `#include`） |

- 所有 `.usf`/`.ush` 必须以 `// Copyright Epic Games, Inc. All Rights Reserved.` 开头
- 插件 Shader 路径：`/Plugin/<PluginName>/Private/Foo.usf`
- 引擎 Shader 路径：`/Engine/Private/Bar.ush`
- Shader 目录注册：模块 `StartupModule()` 中调用 `AddShaderSourceDirectoryMapping`

---

## 二、三种 Shader 类型导航

详细模板见 `templates/` 目录：

| 类型 | 模板文件 | 典型场景 |
|------|---------|---------|
| Pixel / Vertex Shader | `templates/pixel-shader.md` | |
| Compute Shader | `templates/compute-shader.md` ||
| Ray Tracing Shader | `templates/raytrace-shader.md` ||

---

## 三、C++ 端三要素

每种 Shader 类型都需要三个固定要素：

```cpp
// 1. 声明（继承 FGlobalShader）
class FMyShaderCS : public FGlobalShader
{
    DECLARE_GLOBAL_SHADER(FMyShaderCS);
    SHADER_USE_PARAMETER_STRUCT(FMyShaderCS, FGlobalShader);  // 普通 Shader
    // 光追 RayGen 用：SHADER_USE_ROOT_PARAMETER_STRUCT(FMyRGS, FGlobalShader);

    BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
        SHADER_PARAMETER(FVector4f, MyParam)
        SHADER_PARAMETER_RDG_TEXTURE_UAV(RWTexture2D<float4>, OutputTexture)
    END_SHADER_PARAMETER_STRUCT()

    static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
    {
        return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::SM5);
    }
};

// 2. 注册（路径/入口/类型必须三者一致）
IMPLEMENT_GLOBAL_SHADER(FMyShaderCS, "/Plugin/MyPlugin/Private/MyShader.usf", "MainCS", SF_Compute);

// 3. 调用（渲染线程）
TShaderMapRef<FMyShaderCS> Shader(View.ShaderMap);
```

---

## 四、SHADER_PARAMETER 宏速查

| 宏 | HLSL 类型 | 说明 |
|----|-----------|------|
| `SHADER_PARAMETER(T, Name)` | 标量/向量/矩阵 | `float`, `FVector3f`, `FMatrix44f` 等 |
| `SHADER_PARAMETER_ARRAY(T, Name, [N])` | 定长数组 | 对应 HLSL `T Name[N]` |
| `SHADER_PARAMETER(FIntVector3, Name)` | `int3` | 注意用 `FIntVector3` 不是 `FIntVector` |
| `SHADER_PARAMETER_RDG_TEXTURE(Texture2D, Name)` | SRV（默认采样） | 只读 RDG Texture |
| `SHADER_PARAMETER_RDG_TEXTURE_SRV(Texture3D<float4>, Name)` | SRV（带格式） | 需要格式化读取时用 |
| `SHADER_PARAMETER_RDG_TEXTURE_UAV(RWTexture2D<float4>, Name)` | UAV | 可写 RDG Texture |
| `SHADER_PARAMETER_RDG_BUFFER_SRV(StructuredBuffer<T>, Name)` | StructuredBuffer | 只读结构化 Buffer |
| `SHADER_PARAMETER_RDG_BUFFER_UAV(RWStructuredBuffer<T>, Name)` | RWStructuredBuffer | 可写结构化 Buffer |
| `SHADER_PARAMETER_RDG_BUFFER_SRV(RaytracingAccelerationStructure, Name)` | TLAS | 光追专用 |
| `SHADER_PARAMETER_SAMPLER(SamplerState, Name)` | SamplerState | 采样器 |
| `SHADER_PARAMETER_TEXTURE(Texture2D, Name)` | 非 RDG 外部纹理 | 移动端 RHI 路径用 |
| `SHADER_PARAMETER_SRV(Texture3D<float4>, Name)` | 非 RDG SRV | 移动端 RHI 路径用 |
| `SHADER_PARAMETER_SRV(StructuredBuffer<T>, Name)` | 非 RDG Buffer SRV | 移动端 RHI 路径用 |
| `SHADER_PARAMETER_STRUCT_REF(FViewUniformShaderParameters, ViewUniformBuffer)` | View UB | 标准视图参数 |
| `SHADER_PARAMETER_STRUCT_INCLUDE(FXxx, Name)` | 内嵌子结构 | 复用其他参数结构体 |
| `RENDER_TARGET_BINDING_SLOTS()` | RT 绑定 | Pixel Shader 输出，必须放在最后 |

---

## 五、ShouldCompilePermutation 常用条件

```cpp
// 通用 SM5+（桌面端）
return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::SM5);

// 移动端 + 桌面端（ES3_1 以上，PRTGI 的写法）
return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::ES3_1);

// 光追
return ShouldCompileRayTracingShadersForProject(Parameters.Platform);

// 仅移动端
return IsMobilePlatform(Parameters.Platform);
```

---

## 六、Permutation Domain（排列）

```cpp
class FMyCS : public FGlobalShader
{
    // Bool 排列（生成 2 个排列）
    class FNoShadow      : SHADER_PERMUTATION_BOOL("PRTGI_NO_SHADOW");
    // Int 排列（生成 N 个排列，值 0..N-1）
    class FQualityLevel  : SHADER_PERMUTATION_INT("QUALITY_LEVEL", 3);

    using FPermutationDomain = TShaderPermutationDomain<FNoShadow, FQualityLevel>;

    static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
    {
        FPermutationDomain Domain(Parameters.PermutationId);
        // 可在此剔除无效组合，返回 false 跳过编译
        return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::SM5);
    }
};

// 使用时
FMyCS::FPermutationDomain PermutationVector;
PermutationVector.Set<FMyCS::FNoShadow>(true);
TShaderMapRef<FMyCS> Shader(View.ShaderMap, PermutationVector);
```

---

## 七、ModifyCompilationEnvironment 注入宏

```cpp
static void ModifyCompilationEnvironment(
    const FGlobalShaderPermutationParameters& Parameters,
    FShaderCompilerEnvironment& OutEnvironment)
{
    FGlobalShader::ModifyCompilationEnvironment(Parameters, OutEnvironment);
    OutEnvironment.SetDefine(TEXT("THREADGROUP_SIZE_X"), 8);
    OutEnvironment.SetDefine(TEXT("MY_FEATURE"), 1);
    // USF 中用 #if MY_FEATURE ... #endif 或 #ifndef THREADGROUP_SIZE_X
}
```

---

## 八、CVar 调试开关

```cpp
static TAutoConsoleVariable<int32> CVarMyFeature(
    TEXT("r.MyPlugin.MyFeature"),
    1,
    TEXT("描述文字"),
    ECVF_RenderThreadSafe   // Shader/渲染相关必须用 RenderThreadSafe
);

// 渲染线程读取
if (CVarMyFeature.GetValueOnRenderThread() == 0) return;
```

---

## 九、GPU 性能标记

```cpp
// 文件顶部声明
DECLARE_GPU_STAT_NAMED(MyPass, TEXT("MyPlugin MyPass"));

// Pass 内使用
RDG_EVENT_SCOPE(GraphBuilder, "MyPass %dx%d", Width, Height);
RDG_GPU_STAT_SCOPE(GraphBuilder, MyPass);
```

---

## 十、Build.cs 依赖

```csharp
PrivateDependencyModuleNames.AddRange(new string[]
{
    "RenderCore",   // FGlobalShader, RDG, FComputeShaderUtils
    "RHI",          // FRHICommandList, FRHITexture
    "Renderer",     // FViewInfo, FScene, DeferredShadingRenderer 私有头
    "Projects",     // IPluginManager（Shader 目录注册时需要）
});

// 需要访问引擎 Private 头（FViewInfo、ScenePrivate.h 等）
PrivateIncludePaths.AddRange(new string[]
{
    Path.Combine(GetModuleDirectory("Renderer"), "Private"),
});
```

---

## 十一、常见错误速查

| 现象 | 原因 | 解决 |
|------|------|------|
| Shader 不生效 | 忘记 `IMPLEMENT_GLOBAL_SHADER` | 检查注册宏 |
| 参数绑定崩溃 | C++ 名称与 HLSL 变量名不匹配 | 逐字对照 |
| UAV 访问冲突 | 同 Pass 同 Buffer 既 SRV 又 UAV | 拆成两个 Pass |
| 光追不编译 | 缺 `#if RHI_RAYTRACING` 包裹 | 所有光追代码加宏 |
| `ClearUnusedGraphResources` 警告 | 参数结构有未使用绑定 | 调用 `ClearUnusedGraphResources(Shader, Params)` |
| Texture SRV 崩溃 | 用了 `SHADER_PARAMETER_RDG_TEXTURE` 但需要格式化读取 | 改用 `SHADER_PARAMETER_RDG_TEXTURE_SRV` |
| 光追 Dispatch 崩溃 | TLAS 为空或不支持的平台 | 检查 `View.IsRayTracingAllowedForView()` |

---

## 十二、调试命令

```
r.ShaderDevelopmentMode 1        # 开启热重载，编译错误不崩溃
recompileshaders changed         # 重编译所有已修改 Shader
recompileshaders MyShaderName    # 重编译指定 Shader
r.RDG.Debug 1                    # RDG 调试信息
r.RDG.Warnings 1                 # RDG 资源生命周期警告
r.RDG.Breakpoint 1               # 在 Pass 断点（配合调试器）
```
