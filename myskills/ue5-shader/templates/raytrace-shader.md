# Ray Tracing Shader 模板（UE5.7.1）

**本模板基于引擎官方代码提炼：**
- `Renderer/Private/RayTracingAmbientOcclusion.cpp` — `FRayTracingAmbientOcclusionRGS`
- `Renderer/Private/RayTracingShadows.cpp` — `FOcclusionRGS`
- `Renderer/Private/RayTracing/RayTracingBuiltInShaders.usf` — 内置简单示例
- `Renderer/Private/RayTracing/RayTracingCommon.ush` — Payload 和 TraceRay 封装

---

## 0. 平台前提

```cpp
// 所有光追相关声明和调用必须包裹在此宏内
#if RHI_RAYTRACING
// ...
#endif

// 运行时检测（渲染线程调用前）
if (!IsRayTracingEnabled(View.GetShaderPlatform()) || !View.IsRayTracingAllowedForView())
    return;
```

项目要求：DX12 后端 + 项目设置启用 Ray Tracing + 支持 DXR 1.0 的 GPU。

---

## 1. RayGen Shader 类（C++）

与 Compute/Pixel Shader 相比，有三处固定差异：
1. `SHADER_USE_ROOT_PARAMETER_STRUCT`（不是 `SHADER_USE_PARAMETER_STRUCT`）
2. 必须实现 `GetRayTracingPayloadType`
3. 必须实现 `GetShaderBindingLayout`

参数结构体中 Scene 和 NaniteRayTracing 用 `SHADER_PARAMETER_RDG_UNIFORM_BUFFER`（不是 `_STRUCT_REF`）。

```cpp
#if RHI_RAYTRACING

class FMyRayGenRGS : public FGlobalShader
{
    DECLARE_GLOBAL_SHADER(FMyRayGenRGS)
    SHADER_USE_ROOT_PARAMETER_STRUCT(FMyRayGenRGS, FGlobalShader)

    // Permutation 示例（参考 AO 的写法）
    class FEnableMaterialsDim : SHADER_PERMUTATION_BOOL("ENABLE_MATERIALS");
    using FPermutationDomain = TShaderPermutationDomain<FEnableMaterialsDim>;

    static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
    {
        return ShouldCompileRayTracingShadersForProject(Parameters.Platform);
    }

    static void ModifyCompilationEnvironment(
        const FGlobalShaderPermutationParameters& Parameters,
        FShaderCompilerEnvironment& OutEnvironment)
    {
        FGlobalShader::ModifyCompilationEnvironment(Parameters, OutEnvironment);
    }

    // Payload 类型：必须与 Hit/Miss Shader 一致
    static ERayTracingPayloadType GetRayTracingPayloadType(const int32 PermutationId)
    {
        return ERayTracingPayloadType::RayTracingMaterial;
    }

    // 绑定布局：UE5.7.1 必须提供
    static const FShaderBindingLayout* GetShaderBindingLayout(
        const FShaderPermutationParameters& Parameters)
    {
        return RayTracing::GetShaderBindingLayout(Parameters.Platform);
    }

    BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
        // 自定义参数
        SHADER_PARAMETER(int32, SamplesPerPixel)
        SHADER_PARAMETER(float, MaxRayDistance)
        SHADER_PARAMETER(float, MaxNormalBias)
        // TLAS
        SHADER_PARAMETER_RDG_BUFFER_SRV(RaytracingAccelerationStructure, TLAS)
        // 输出 UAV
        SHADER_PARAMETER_RDG_TEXTURE_UAV(RWTexture2D<float4>, RWOutputUAV)
        // 标准 View 参数
        SHADER_PARAMETER_STRUCT_REF(FViewUniformShaderParameters, ViewUniformBuffer)
        // Scene / Nanite（RDG Uniform Buffer，不是 _STRUCT_REF）
        SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FSceneUniformParameters, Scene)
        SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FNaniteRayTracingUniformParameters, NaniteRayTracing)
        // GBuffer 输入（可选）
        SHADER_PARAMETER_STRUCT_INCLUDE(FSceneTextureParameters, SceneTextures)
    END_SHADER_PARAMETER_STRUCT()
};
IMPLEMENT_GLOBAL_SHADER(FMyRayGenRGS,
    "/Plugin/MyPlugin/Private/MyRayTracing.usf", "MyRayGen", SF_RayGen);

#endif // RHI_RAYTRACING
```

---

## 2. RDG Dispatch 代码（C++）

### 路径 A：有材质（使用场景已有 Pipeline + SBT）

场景帧渲染路径的标准用法，不需要手动创建 PSO。

```cpp
#if RHI_RAYTRACING
void DispatchMyRayGen_RenderThread(
    FDeferredShadingSceneRenderer* Renderer,
    const FViewInfo& View,
    FRDGBuilder& GraphBuilder,
    FRDGTextureRef OutputTexture)
{
    // 1. 分配并填写参数
    FMyRayGenRGS::FParameters* PassParameters =
        GraphBuilder.AllocParameters<FMyRayGenRGS::FParameters>();

    PassParameters->SamplesPerPixel  = 1;
    PassParameters->MaxRayDistance   = View.FinalPostProcessSettings.AmbientOcclusionRadius;
    PassParameters->MaxNormalBias    = GetRaytracingMaxNormalBias();

    // TLAS：来自场景光追加速结构
    PassParameters->TLAS = View.GetRayTracingSceneLayerViewChecked(ERayTracingSceneLayer::Base);

    PassParameters->RWOutputUAV      = GraphBuilder.CreateUAV(OutputTexture);
    PassParameters->ViewUniformBuffer = View.ViewUniformBuffer;

    // Scene / Nanite Uniform Buffer（官方写法）
    PassParameters->Scene            = GetSceneUniformBufferRef(GraphBuilder);
    PassParameters->NaniteRayTracing = Nanite::GRayTracingManager.GetUniformBuffer();

    PassParameters->SceneTextures    = GetSceneTextureParameters(GraphBuilder, View);

    // 2. 选取排列
    FMyRayGenRGS::FPermutationDomain PermutationVector;
    PermutationVector.Set<FMyRayGenRGS::FEnableMaterialsDim>(true);
    TShaderMapRef<FMyRayGenRGS> RayGenShader(
        GetGlobalShaderMap(GMaxRHIFeatureLevel), PermutationVector);
    ClearUnusedGraphResources(RayGenShader, PassParameters);

    const FIntPoint Resolution = View.ViewRect.Size();

    // 3. 添加 Pass（光追走 Compute Queue）
    GraphBuilder.AddPass(
        RDG_EVENT_NAME("MyPlugin::RayGen %dx%d", Resolution.X, Resolution.Y),
        PassParameters,
        ERDGPassFlags::Compute,
        [PassParameters, RayGenShader, &View, Resolution]
        (FRHICommandList& RHICmdList)
        {
            // Shader 参数（光追必须用 BatchedShaderParameters）
            FRHIBatchedShaderParameters& GlobalResources =
                RHICmdList.GetScratchShaderParameters();
            SetShaderParameters(GlobalResources, RayGenShader, *PassParameters);

            // 绑定全局静态 Uniform Buffer
            FRHIUniformBuffer* SceneUB         = PassParameters->Scene->GetRHI();
            FRHIUniformBuffer* NaniteRayTracingUB = PassParameters->NaniteRayTracing->GetRHI();
            TOptional<FScopedUniformBufferStaticBindings> StaticUBScope =
                RayTracing::BindStaticUniformBufferBindings(
                    View, SceneUB, NaniteRayTracingUB, RHICmdList);

            // 使用场景已有 Pipeline 和 SBT（无需手动建 PSO）
            RHICmdList.RayTraceDispatch(
                View.MaterialRayTracingData.PipelineState,
                RayGenShader.GetRayTracingShader(),
                View.MaterialRayTracingData.ShaderBindingTable,
                GlobalResources,
                Resolution.X, Resolution.Y);
        });
}
#endif // RHI_RAYTRACING
```

### 路径 B：无材质（Simplified Pipeline，手动建 PSO + SBT）

适用于遮挡/AO 关闭材质时的简化路径（或自定义场景）。
参考：`RayTracingAmbientOcclusion.cpp` 中 `CVarRayTracingAmbientOcclusionEnableMaterials == 0` 分支。

```cpp
// 在 Pass Lambda 内
FRayTracingPipelineStateInitializer Initializer;
Initializer.MaxPayloadSizeInBytes =
    GetRayTracingPayloadTypeMaxSize(ERayTracingPayloadType::RayTracingMaterial);

// 绑定布局（UE5.7.1 必须设置）
const FShaderBindingLayout* ShaderBindingLayout =
    RayTracing::GetShaderBindingLayout(ShaderPlatform);
if (ShaderBindingLayout)
    Initializer.ShaderBindingLayout = &ShaderBindingLayout->RHILayout;

// RayGen Table
FRHIRayTracingShader* RayGenTable[] = { RayGenShader.GetRayTracingShader() };
Initializer.SetRayGenShaderTable(RayGenTable);

// HitGroup（默认不透明 Shader）
FRHIRayTracingShader* HitGroupTable[] = { GetRayTracingDefaultOpaqueShader(View.ShaderMap) };
Initializer.SetHitGroupTable(HitGroupTable);

// Miss Shader
FRHIRayTracingShader* MissTable[] = { GetRayTracingDefaultMissShader(View.ShaderMap) };
Initializer.SetMissShaderTable(MissTable);

FRayTracingPipelineState* Pipeline =
    PipelineStateCache::GetAndOrCreateRayTracingPipelineState(RHICmdList, Initializer);

// 分配 Transient SBT
FShaderBindingTableRHIRef SBT = Scene->RayTracingSBT.AllocateTransientRHI(
    RHICmdList,
    ERayTracingShaderBindingMode::RTPSO,
    ERayTracingHitGroupIndexingMode::Disallow,
    Initializer.GetMaxLocalBindingDataSize());

// 写入 SBT 表项
RHICmdList.SetDefaultRayTracingHitGroup(SBT, Pipeline, 0);
RHICmdList.SetRayTracingMissShader(SBT, 0, Pipeline, 0, 0, nullptr, 0);
RHICmdList.CommitShaderBindingTable(SBT);

RHICmdList.RayTraceDispatch(
    Pipeline, RayGenShader.GetRayTracingShader(),
    SBT, GlobalResources,
    Resolution.X, Resolution.Y);
```

---

## 3. USF 模板（MyRayTracing.usf）

### 3.1 头文件 include

```hlsl
// Copyright Epic Games, Inc. All Rights Reserved.

#include "/Engine/Private/Common.ush"
#include "/Engine/Private/RayTracing/RayTracingCommon.ush"
#include "/Engine/Private/RayTracing/RayTracingDeferredShadingCommon.ush"
#include "/Engine/Private/RayTracing/RayTracingHitGroupCommon.ush"
// GBuffer 读取（若需要）：
#include "/Engine/Private/SceneTexturesCommon.ush"
#include "/Engine/Private/DeferredShadingCommon.ush"
```

### 3.2 绑定变量（与 C++ FParameters 一一对应）

```hlsl
// 自定义参数
int   SamplesPerPixel;
float MaxRayDistance;
float MaxNormalBias;

// TLAS（固定写法）
RaytracingAccelerationStructure TLAS;

// 输出
RWTexture2D<float4> RWOutputUAV;
```

### 3.3 RayGen 入口（用 UE 宏，不要裸写 [shader(...)]）

```hlsl
// RAY_TRACING_ENTRY_RAYGEN 是 UE 提供的宏，展开为 [shader("raygeneration")]
// 同时处理好 DispatchRaysIndex 等内置变量
RAY_TRACING_ENTRY_RAYGEN(MyRayGen)
{
    // 像素坐标（加上 ViewRectMin 偏移到屏幕绝对位置）
    const uint2 PixelCoord = DispatchRaysIndex().xy + View.ViewRectMin.xy;

    // --- 从 GBuffer 读取表面信息 ---
    float2 BufferUV = (float2(PixelCoord) + 0.5f) * View.BufferSizeAndInvSize.zw;

    FGBufferData GBuffer = GetGBufferDataFromSceneTexturesLoad(PixelCoord);
    float  DeviceZ    = SceneDepthTexture.Load(int3(PixelCoord, 0)).r;
    float  SceneDepth = ConvertFromDeviceZ(DeviceZ);
    float3 WorldNormal = GBuffer.WorldNormal;

    // 重建世界坐标
    float4 ClipPos = float4(
        (float2(PixelCoord) + 0.5f) * View.ViewSizeAndInvSize.zw * float2(2, -2) + float2(-1, 1),
        DeviceZ, 1);
    float4 WorldPos4 = mul(ClipPos, View.ScreenToTranslatedWorld);
    float3 TranslatedWorldPos = WorldPos4.xyz / WorldPos4.w;

    // --- 构造光线（以 AO 余弦分布为例）---
    FRayDesc Ray;
    Ray.Origin    = TranslatedWorldPos + WorldNormal * MaxNormalBias;
    Ray.TMin      = 0.01f;
    Ray.TMax      = MaxRayDistance;

    // 简单半球随机方向（实际项目用 RandomSequence）
    float3 RandDir = WorldNormal; // 替换为随机采样
    Ray.Direction = normalize(RandDir);

    // --- 追踪遮挡光线（使用 UE 封装的 TraceVisibilityRay）---
    // TraceVisibilityRay 返回 FMinimalPayload，HitT < 0 为 Miss
    uint RayFlags = RAY_FLAG_CULL_BACK_FACING_TRIANGLES
                  | RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH;

    FMinimalPayload Payload = TraceVisibilityRay(
        TLAS,
        RayFlags,
        RAY_TRACING_MASK_SHADOW,   // InstanceInclusionMask：仅检测投影体
        Ray);

    float Occlusion = Payload.IsHit() ? 1.0f : 0.0f;
    RWOutputUAV[PixelCoord] = float4(Occlusion, Payload.HitT, 0, 1);
}
```

### 3.4 更简单的 1D Dispatch 示例（无 GBuffer）

```hlsl
// 参考 RayTracingBuiltInShaders.usf: OcclusionMainRGS
RAY_TRACING_ENTRY_RAYGEN(SimpleOcclusionRGS)
{
    const uint RayIndex = DispatchRaysIndex().x;

    RayDesc Ray;
    Ray.Origin    = float3(0, 0, 0); // 从外部 Buffer 读取
    Ray.Direction = float3(0, 0, 1);
    Ray.TMin      = 0.0f;
    Ray.TMax      = 1000.0f;

    uint RayFlags = RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH
                  | RAY_FLAG_FORCE_OPAQUE
                  | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER;

    FDefaultPayload Payload = (FDefaultPayload)0;

    TraceRay(
        TLAS,
        RayFlags,
        RAY_TRACING_MASK_OPAQUE,            // InstanceInclusionMask
        RAY_TRACING_SHADER_SLOT_MATERIAL,   // RayContributionToHitGroupIndex
        RAY_TRACING_NUM_SHADER_SLOTS,       // MultiplierForGeometryContributionToShaderIndex
        0,                                  // MissShaderIndex
        Ray,
        Payload);

    // FMinimalPayload 的 IsHit() 判断
    RWOutputUAV[uint2(RayIndex, 0)] = ((FMinimalPayload)Payload).IsHit()
        ? float4(1, 0, 0, 1)
        : float4(0, 1, 0, 1);
}
```

---

## 4. Payload 类型速查

```hlsl
// 仅遮挡/可见性检测（最小开销）
FMinimalPayload Payload = TraceVisibilityRay(TLAS, RayFlags, Mask, Ray);
bool bHit = Payload.IsHit();    // HitT >= 0 为命中
float HitT = Payload.HitT;

// 半透明投影（含 ShadowVisibility）
FTranslucentMinimalPayload Payload = TraceTranslucentVisibilityRay(TLAS, RayFlags, Mask, Ray);
float3 Visibility = Payload.ShadowVisibility; // Miss 时 = 1.0（全透明）

// 完整材质信息（用于 GI/Reflection）
FPackedMaterialClosestHitPayload Payload = (FPackedMaterialClosestHitPayload)0;
TraceRay(TLAS, RayFlags, Mask,
    RAY_TRACING_SHADER_SLOT_MATERIAL, RAY_TRACING_NUM_SHADER_SLOTS, 0,
    Ray, Payload);
```

---

## 5. 常用 RayFlags 和 InstanceInclusionMask

```hlsl
// RayFlags
RAY_FLAG_FORCE_OPAQUE                    // 跳过 AHS，所有几何体视为不透明
RAY_FLAG_CULL_BACK_FACING_TRIANGLES      // 剔除背面
RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH // 命中即停止（遮挡检测用）
RAY_FLAG_SKIP_CLOSEST_HIT_SHADER         // 不运行 CHS（配合 Accept First Hit 提速）

// InstanceInclusionMask（定义在 RayTracingDefinitions.ush）
RAY_TRACING_MASK_OPAQUE                  // 0x01 仅不透明
RAY_TRACING_MASK_TRANSLUCENT             // 0x02 半透明
RAY_TRACING_MASK_SHADOW                  // 0x04 投影体（AO/Shadow 用）
RAY_TRACING_MASK_THIN_SHADOW            // 0x08 薄投影体
0xFF                                     // 全部实例
```

---

## 6. 关键点汇总

| 项目 | 官方写法 | 常见错误 |
|------|---------|---------|
| 参数宏 | `SHADER_USE_ROOT_PARAMETER_STRUCT` | 误用 `SHADER_USE_PARAMETER_STRUCT` |
| Scene UB | `SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FSceneUniformParameters, Scene)` | 用 `_STRUCT_REF` 会导致绑定失败 |
| Scene UB 取 RHI | `PassParameters->Scene->GetRHI()` | 不能用 `View.GetSceneUniforms().GetBufferRHI()` |
| NaniteRT UB | `Nanite::GRayTracingManager.GetUniformBuffer()` | 忘记绑定导致 Nanite 几何体不参与光追 |
| 设置 Shader 参数 | `FRHIBatchedShaderParameters` + `SetShaderParameters` | 不能直接用 `SetShaderParameters(RHICmdList, ...)` |
| 静态 UB 绑定 | `RayTracing::BindStaticUniformBufferBindings(View, SceneUB, NaniteUB, RHICmdList)` | 漏绑导致 View 参数错误 |
| 有材质路径 | `View.MaterialRayTracingData.PipelineState/ShaderBindingTable` | 不需要手动建 PSO |
| 无材质路径 SBT | `Scene->RayTracingSBT.AllocateTransientRHI` + Commit | 忘记 `CommitShaderBindingTable` 导致崩溃 |
| USF 入口宏 | `RAY_TRACING_ENTRY_RAYGEN(FuncName)` | 直接写 `[shader("raygeneration")]` 可能不兼容 UE 宏体系 |
| TraceRay 封装 | 优先用 `TraceVisibilityRay` / `TraceTranslucentVisibilityRay` | 裸写 `TraceRay` 需要手动设正确的 ShaderSlot 参数 |
| 平台保护 | `#if RHI_RAYTRACING` 包裹所有声明和调用 | 非光追平台编译失败 |
