# Compute Shader 模板（UE5.7.1）

**本模板基于引擎官方代码提炼：**
- `Renderer/Private/ScreenSpaceDenoise.cpp` — `FSSDInjestCS`、`FSSDSpatialAccumulationCS`
- `Renderer/Private/Lumen/LumenScreenProbeGather.cpp` — `FScreenProbeIntegrateCS`
- `Renderer/Private/ComputeShaderUtils.h` — `FComputeShaderUtils::AddPass`、`kGolden2DGroupSize`

---

## 1. Compute Shader 类定义

官方惯例：
- 线程组大小作为**类静态常量**（不仅注入 USF 宏，C++ 侧也显式声明）
- `SHADER_PARAMETER_STRUCT_INCLUDE` 组合参数，避免参数结构体过长
- `SHADER_PARAMETER_RDG_UNIFORM_BUFFER` 绑定 SceneTextures 等全局 UB

```cpp
class FMyComputeCS : public FGlobalShader
{
public:
    DECLARE_GLOBAL_SHADER(FMyComputeCS);
    SHADER_USE_PARAMETER_STRUCT(FMyComputeCS, FGlobalShader);

    // 线程组大小：作为类常量，与 USF 保持同步
    static const uint32 kGroupSize = 8;   // 2D 处理
    // static const uint32 kGroupSize = 64;  // 1D/线性数据

    // Permutation（参考 SSD 的写法）
    class FEnableFeatureDim : SHADER_PERMUTATION_BOOL("ENABLE_MY_FEATURE");
    class FQualityDim       : SHADER_PERMUTATION_INT("QUALITY_LEVEL", 3);
    using FPermutationDomain = TShaderPermutationDomain<FEnableFeatureDim, FQualityDim>;

    // 在 ShouldCompilePermutation 中剔除无效组合
    static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
    {
        FPermutationDomain Domain(Parameters.PermutationId);
        // 示例：Quality=2 只在高质量排列下编译
        // if (Domain.Get<FQualityDim>() == 2 && !SomeCondition) return false;
        return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::SM5);
    }

    static void ModifyCompilationEnvironment(
        const FGlobalShaderPermutationParameters& Parameters,
        FShaderCompilerEnvironment& OutEnvironment)
    {
        FGlobalShader::ModifyCompilationEnvironment(Parameters, OutEnvironment);
        // 注入与 kGroupSize 对应的 USF 宏
        OutEnvironment.SetDefine(TEXT("THREADGROUP_SIZE"), kGroupSize);
    }

    BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
        // ---- 输入 ----
        SHADER_PARAMETER_RDG_TEXTURE(Texture2D, InputTexture)
        SHADER_PARAMETER_SAMPLER(SamplerState, InputSampler)
        SHADER_PARAMETER_RDG_BUFFER_SRV(StructuredBuffer<FMyGPUStruct>, InputBuffer)

        // ---- SceneTextures（官方 UB 方式）----
        SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FSceneTextureUniformParameters, SceneTexturesStruct)
        // 或 Substrate / Scene 等其他全局 UB：
        // SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FSubstrateGlobalUniformParameters, Substrate)

        // ---- 标准 View ----
        SHADER_PARAMETER_STRUCT_REF(FViewUniformShaderParameters, View)

        // ---- 自定义标量参数 ----
        SHADER_PARAMETER(FVector4f, ViewSizeAndInvSize)
        SHADER_PARAMETER(FMatrix44f, InvViewProjectionMatrix)
        SHADER_PARAMETER(uint32, SomeFlag)

        // ---- 输出 UAV ----
        SHADER_PARAMETER_RDG_BUFFER_UAV(RWStructuredBuffer<FMyGPUStruct>, OutputBuffer)
        SHADER_PARAMETER_RDG_TEXTURE_UAV(RWTexture2D<float4>, OutputTexture)
    END_SHADER_PARAMETER_STRUCT()
};
IMPLEMENT_GLOBAL_SHADER(FMyComputeCS,
    "/Plugin/MyPlugin/Private/MyCompute.usf", "MainCS", SF_Compute);
```

---

## 2. Dispatch（标准方式：FComputeShaderUtils::AddPass）

官方的**首选写法**，参考 `ScreenSpaceDenoise.cpp` 和 `LumenScreenProbeGather.cpp`。

```cpp
void DispatchMyCompute_RenderThread(
    FRDGBuilder& GraphBuilder,
    const FViewInfo& View,
    FRDGTextureRef InputTexture,
    FRDGBufferRef OutputBuffer,
    FIntPoint DispatchSize,
    bool bEnableFeature,
    int32 QualityLevel)
{
    // 1. 分配参数
    FMyComputeCS::FParameters* PassParameters =
        GraphBuilder.AllocParameters<FMyComputeCS::FParameters>();

    PassParameters->InputTexture    = InputTexture;
    PassParameters->InputSampler    =
        TStaticSamplerState<SF_Point, AM_Clamp, AM_Clamp>::GetRHI();
    PassParameters->SceneTexturesStruct = SceneTextures.UniformBuffer;
    PassParameters->View            = View.ViewUniformBuffer;
    PassParameters->ViewSizeAndInvSize = FVector4f(
        DispatchSize.X, DispatchSize.Y,
        1.f / DispatchSize.X, 1.f / DispatchSize.Y);
    PassParameters->InvViewProjectionMatrix =
        FMatrix44f(View.ViewMatrices.GetInvViewProjectionMatrix());

    PassParameters->OutputBuffer  = GraphBuilder.CreateUAV(
        FRDGBufferUAVDesc(OutputBuffer));
    PassParameters->OutputTexture = GraphBuilder.CreateUAV(
        FRDGTextureUAVDesc(/* 目标 RDGTexture */));

    // 2. 选取排列
    FMyComputeCS::FPermutationDomain PermutationVector;
    PermutationVector.Set<FMyComputeCS::FEnableFeatureDim>(bEnableFeature);
    PermutationVector.Set<FMyComputeCS::FQualityDim>(QualityLevel);
    TShaderMapRef<FMyComputeCS> ComputeShader(View.ShaderMap, PermutationVector);

    // 3. FComputeShaderUtils::AddPass（官方首选，线程组自动计算）
    FComputeShaderUtils::AddPass(
        GraphBuilder,
        RDG_EVENT_NAME("MyPlugin::MyCompute(Feature=%d Quality=%d) %dx%d",
            bEnableFeature, QualityLevel, DispatchSize.X, DispatchSize.Y),
        ComputeShader,
        PassParameters,
        // 2D：GetGroupCount(FIntPoint, GroupSize)
        FComputeShaderUtils::GetGroupCount(DispatchSize, FMyComputeCS::kGroupSize));
        // 或使用引擎推荐的 kGolden2DGroupSize（8x8）：
        // FComputeShaderUtils::GetGroupCount(DispatchSize, FComputeShaderUtils::kGolden2DGroupSize)
}
```

---

## 3. Dispatch（间接调度：Indirect Dispatch）

当线程组数量来自 GPU Buffer（Lumen Tile 分类后的做法）：

```cpp
// 参数结构体中声明 Indirect Args 访问
BEGIN_SHADER_PARAMETER_STRUCT(FMyIndirectCSParams, )
    SHADER_PARAMETER_RDG_BUFFER_SRV(StructuredBuffer<uint2>, TileData)
    SHADER_PARAMETER_RDG_TEXTURE_UAV(RWTexture2D<float4>, OutputTexture)
    RDG_BUFFER_ACCESS(IndirectArgs, ERHIAccess::IndirectArgs)  // 标记 Indirect Buffer
END_SHADER_PARAMETER_STRUCT()

// Dispatch 时传 IndirectArgs Buffer
FMyComputeCS::FParameters* PassParameters = ...;
PassParameters->IndirectArgs = IndirectArgsBuffer;

FComputeShaderUtils::AddPass(
    GraphBuilder,
    RDG_EVENT_NAME("MyPlugin::IndirectDispatch"),
    ComputePassFlags,       // ERDGPassFlags::Compute 或 AsyncCompute
    ComputeShader,
    PassParameters,
    IndirectArgsBuffer,     // FRDGBuffer*（Indirect Args）
    0);                     // ByteOffset in the buffer
```

---

## 4. Async Compute

与光栅化并行的后台计算，**仅在没有同帧资源依赖时使用**：

```cpp
// ERDGPassFlags::AsyncCompute（替换 Compute）
FComputeShaderUtils::AddPass(
    GraphBuilder,
    RDG_EVENT_NAME("MyPlugin::AsyncCompute"),
    ERDGPassFlags::AsyncCompute,   // ← 与光栅化并行
    ComputeShader,
    PassParameters,
    FComputeShaderUtils::GetGroupCount(DispatchSize, FMyComputeCS::kGroupSize));
```

---

## 5. RDG Buffer / Texture 创建速查

```cpp
// ---- 上传 CPU 数组到 GPU（只读 SRV）----
FRDGBufferRef UploadBuf = CreateUploadBuffer(
    GraphBuilder,
    TEXT("MyPlugin.Upload"),
    sizeof(FMyGPUStruct),       // stride
    CpuArray.Num(),
    CpuArray.GetData(),
    CpuArray.Num() * sizeof(FMyGPUStruct));
PassParameters->InputBuffer = GraphBuilder.CreateSRV(UploadBuf);

// ---- 创建 GPU 输出 Buffer（UAV）----
FRDGBufferRef OutputBuf = GraphBuilder.CreateBuffer(
    FRDGBufferDesc::CreateStructuredDesc(sizeof(FMyGPUStruct), ElementCount),
    TEXT("MyPlugin.Output"));
PassParameters->OutputBuffer = GraphBuilder.CreateUAV(FRDGBufferUAVDesc(OutputBuf));

// ---- 注册外部持久 Buffer（跨帧 Pooled Buffer）----
FRDGBufferRef ExternalBuf = GraphBuilder.RegisterExternalBuffer(MyPooledBuffer);

// ---- 创建 2D Output Texture ----
FRDGTextureRef OutputTex = GraphBuilder.CreateTexture(
    FRDGTextureDesc::Create2D(
        FIntPoint(SizeX, SizeY),
        PF_FloatRGBA,
        FClearValueBinding::Black,
        TexCreate_ShaderResource | TexCreate_UAV),
    TEXT("MyPlugin.OutputTexture"));
PassParameters->OutputTexture = GraphBuilder.CreateUAV(FRDGTextureUAVDesc(OutputTex));
```

---

## 6. GPU Readback

### Buffer Readback（轮询方式）

```cpp
FRHIGPUBufferReadback* Readback = new FRHIGPUBufferReadback(TEXT("MyReadback"));
AddEnqueueCopyPass(GraphBuilder, Readback, OutputBuffer, 0u);   // 0 = 全量拷贝

// 渲染线程上递归轮询
auto PollFunc = [Readback, ElementCount, Callback](auto&& Self) -> void
{
    if (Readback->IsReady())
    {
        FMyGPUStruct* Data =
            (FMyGPUStruct*)Readback->Lock(ElementCount * sizeof(FMyGPUStruct));
        TArray<FMyGPUStruct> Result(Data, ElementCount);
        Readback->Unlock();
        delete Readback;
        AsyncTask(ENamedThreads::GameThread,
            [Callback, Result = MoveTemp(Result)]() { Callback(Result); });
    }
    else
    {
        AsyncTask(ENamedThreads::ActualRenderingThread,
            [Self]() { Self(Self); });
    }
};
AsyncTask(ENamedThreads::ActualRenderingThread,
    [PollFunc]() { PollFunc(PollFunc); });
```

### Texture Readback（跨帧持有）

```cpp
// 成员变量，跨帧持有
FRHIGPUTextureReadback* ReadbackTex =
    new FRHIGPUTextureReadback(TEXT("MyTexReadback"));

// 在 GraphBuilder 中发起异步拷贝
AddEnqueueCopyPass(GraphBuilder, ReadbackTex, RDGOutputTexture);

// 后续帧 Tick 中轮询
if (ReadbackTex->IsReady())
{
    int32 RowPitchInPixels, BufferHeight;
    uint8* Data = (uint8*)ReadbackTex->Lock(RowPitchInPixels, &BufferHeight);
    FMemory::Memcpy(DestBuffer, Data, ByteSize);
    ReadbackTex->Unlock();
}
```

---

## 7. USF 模板（MyCompute.usf）

```hlsl
// Copyright Epic Games, Inc. All Rights Reserved.

#include "/Engine/Public/Platform.ush"
// 需要 View 内置变量时：
// #include "/Engine/Private/Common.ush"
// 需要 SceneTextures 时（配合 FSceneTextureUniformParameters）：
// #include "/Engine/Private/SceneTexturesCommon.ush"

// 绑定变量（名称与 C++ FParameters 成员完全一致）
Texture2D<float4>               InputTexture;
SamplerState                    InputSampler;
StructuredBuffer<FMyGPUStruct>  InputBuffer;
float4                          ViewSizeAndInvSize;
float4x4                        InvViewProjectionMatrix;
uint                            SomeFlag;

RWStructuredBuffer<FMyGPUStruct> OutputBuffer;
RWTexture2D<float4>              OutputTexture;

// USF 中线程组大小由 C++ ModifyCompilationEnvironment 注入
// 同时提供回退默认值，防止单独编译时报错
#ifndef THREADGROUP_SIZE
#define THREADGROUP_SIZE 8
#endif

[numthreads(THREADGROUP_SIZE, THREADGROUP_SIZE, 1)]
void MainCS(uint3 DispatchThreadID : SV_DispatchThreadID,
            uint3 GroupID          : SV_GroupID,
            uint3 GroupThreadID    : SV_GroupThreadID)
{
    // 越界保护（Dispatch 向上取整，边缘线程可能超出实际范围）
    uint2 Resolution = uint2(ViewSizeAndInvSize.xy);
    if (any(DispatchThreadID.xy >= Resolution))
        return;

    // UV（像素中心对齐）
    float2 UV = (float2(DispatchThreadID.xy) + 0.5f) * ViewSizeAndInvSize.zw;

    // 采样
    float4 Color = InputTexture.SampleLevel(InputSampler, UV, 0);

    // 重建世界坐标
    float4 ClipPos   = float4(UV * float2(2, -2) + float2(-1, 1), 0, 1);
    float4 WorldPos4 = mul(ClipPos, InvViewProjectionMatrix);
    float3 WorldPos  = WorldPos4.xyz / WorldPos4.w;

    // 线性写入输出 Buffer
    uint LinearIdx = DispatchThreadID.x + DispatchThreadID.y * Resolution.x;
    FMyGPUStruct Out = (FMyGPUStruct)0;
    Out.Position = WorldPos;
    Out.Color    = Color.rgb;
    OutputBuffer[LinearIdx] = Out;

    // 写入输出 Texture
    OutputTexture[DispatchThreadID.xy] = Color;
}
```

---

## 8. 关键点

| 项目 | 官方做法 | 说明 |
|------|---------|------|
| 线程组大小 | 类静态常量 `static const uint32 kGroupSize = 8` | 同时注入 USF 宏 + C++ 侧 `GetGroupCount` 使用 |
| 2D Group Count | `FComputeShaderUtils::GetGroupCount(ViewSize, kGroupSize)` | 自动向上取整 |
| 2D 推荐尺寸 | `FComputeShaderUtils::kGolden2DGroupSize`（=8） | SSD 等系统使用的黄金分割尺寸 |
| 1D Group Count | `FComputeShaderUtils::GetGroupCount(ElementCount, 64)` | 线性数据处理 |
| SceneTextures | `SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FSceneTextureUniformParameters, SceneTexturesStruct)` | 官方后处理/Lumen 用 UB 方式 |
| Dispatch 首选 | `FComputeShaderUtils::AddPass` | 比手写 `GraphBuilder.AddPass` 简洁 |
| Async Compute | `ERDGPassFlags::AsyncCompute` | 与光栅化并行；有同帧资源依赖必须用 `Compute` |
| Indirect Dispatch | `RDG_BUFFER_ACCESS(IndirectArgs, ERHIAccess::IndirectArgs)` + `AddPass(..., IndirectBuffer, Offset)` | GPU 驱动的线程组数量 |
| 越界保护 | USF 中 `if (any(id >= size)) return;` | Dispatch 向上取整，边缘必须保护 |
| Readback 生命周期 | Texture Readback 成员变量跨帧持有；Buffer Readback Lambda 内 new/delete | 不能在 GraphBuilder 外保存 RDG 资源引用 |
