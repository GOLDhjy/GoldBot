# Pixel Shader / Vertex Shader 模板（UE5.7.1）

**本模板基于引擎官方代码提炼：**
- `Renderer/Private/PostProcess/PostProcessTonemap.cpp` — `FTonemapPS/FTonemapVS`
- `Renderer/Private/ScreenSpaceDenoise.cpp` — `FSSDInjestCS`（GBuffer 读取模式）
- `Renderer/Private/ScreenPass.h` — `AddDrawScreenPass`、`FScreenPassPipelineState`

---

## 1. 参数结构体

官方惯用 `SHADER_PARAMETER_STRUCT_INCLUDE` 拆分参数组，VS 和 PS 可共用同一个外部结构体。

```cpp
// 独立参数结构体（可被多个 Shader 共用）
BEGIN_SHADER_PARAMETER_STRUCT(FMyPassParameters, )
    // ---- 标准 View ----
    SHADER_PARAMETER_STRUCT_INCLUDE(FViewShaderParameters, View)

    // ---- 输入纹理 ----
    SHADER_PARAMETER_RDG_TEXTURE(Texture2D, ColorTexture)
    SHADER_PARAMETER_RDG_TEXTURE(Texture2D, DepthTexture)
    SHADER_PARAMETER_SAMPLER(SamplerState, ColorSampler)

    // ---- SceneTextures（两种写法任选其一）----
    // 方式 A：SceneTexture Uniform Buffer（Lumen/后处理常用）
    SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FSceneTextureUniformParameters, SceneTexturesStruct)
    // 方式 B：展开参数（旧式，仍可用）
    // SHADER_PARAMETER_STRUCT_INCLUDE(FSceneTextureParameters, SceneTextures)

    // ---- 自定义参数 ----
    SHADER_PARAMETER(FVector4f, ColorScale)
    SHADER_PARAMETER(float, Intensity)
    SHADER_PARAMETER(FMatrix44f, ViewProjectionMatrix)
END_SHADER_PARAMETER_STRUCT()
```

---

## 2. Pixel Shader 类

```cpp
class FMyPassPS : public FGlobalShader
{
public:
    DECLARE_GLOBAL_SHADER(FMyPassPS);
    SHADER_USE_PARAMETER_STRUCT(FMyPassPS, FGlobalShader);

    // 内嵌参数结构体（包含独立结构 + RT 绑定）
    BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
        SHADER_PARAMETER_STRUCT_INCLUDE(FMyPassParameters, Pass)
        RENDER_TARGET_BINDING_SLOTS()          // PS 输出，必须放在最后
    END_SHADER_PARAMETER_STRUCT()

    static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
    {
        return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::ES3_1);
    }

    static void ModifyCompilationEnvironment(
        const FGlobalShaderPermutationParameters& Parameters,
        FShaderCompilerEnvironment& OutEnvironment)
    {
        FGlobalShader::ModifyCompilationEnvironment(Parameters, OutEnvironment);
        // OutEnvironment.SetDefine(TEXT("MY_DEFINE"), 1);
    }
};
IMPLEMENT_GLOBAL_SHADER(FMyPassPS,
    "/Plugin/MyPlugin/Private/MyPass.usf", "MainPS", SF_Pixel);
```

---

## 3. （可选）Vertex Shader

全屏三角形通常用 `AddDrawScreenPass` 自动处理，**不需要自定义 VS**。
需要自定义顶点变换（Instanced Draw）时才添加：

```cpp
class FMyPassVS : public FGlobalShader
{
public:
    DECLARE_GLOBAL_SHADER(FMyPassVS);
    SHADER_USE_PARAMETER_STRUCT(FMyPassVS, FGlobalShader);

    // VS 只用 Pass 参数（不含 RT 绑定）
    using FParameters = FMyPassParameters;

    static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
    {
        return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::ES3_1);
    }
};
IMPLEMENT_GLOBAL_SHADER(FMyPassVS,
    "/Plugin/MyPlugin/Private/MyPass.usf", "MainVS", SF_Vertex);
```

---

## 4A. AddDrawScreenPass（官方标准全屏写法）

参考：`PostProcessTonemap.cpp` Tonemap Pass 的调用模式。
适用于：全屏后处理、GI 合成、叠加效果。

```cpp
#include "ScreenPass.h"  // AddDrawScreenPass, FScreenPassPipelineState

void RenderMyPass_RenderThread(
    const FViewInfo& View,
    FRDGBuilder& GraphBuilder,
    FScreenPassTexture SceneColor,     // 来自 FGlobalIlluminationPluginResources 或 PostProcessInput
    FRDGTextureRef DepthTexture,
    FScreenPassRenderTarget Output)    // 输出 RT
{
    // Viewport（输入和输出分辨率可以不同）
    const FScreenPassTextureViewport InputViewport(SceneColor);
    const FScreenPassTextureViewport OutputViewport(Output);

    // 1. 分配 PS 参数
    FMyPassPS::FParameters* PassParameters =
        GraphBuilder.AllocParameters<FMyPassPS::FParameters>();

    PassParameters->Pass.View           = GetShaderBinding(View);
    PassParameters->Pass.ColorTexture   = SceneColor.Texture;
    PassParameters->Pass.DepthTexture   = DepthTexture;
    PassParameters->Pass.ColorSampler   =
        TStaticSamplerState<SF_Point, AM_Clamp, AM_Clamp>::GetRHI();
    PassParameters->Pass.Intensity      = 1.0f;
    PassParameters->Pass.SceneTexturesStruct = SceneTextures.UniformBuffer;

    // 2. 绑定输出 RT
    PassParameters->RenderTargets[0] = Output.GetRenderTargetBinding();

    // 3. 获取 Shader
    TShaderMapRef<FMyPassVS> VertexShader(View.ShaderMap);
    TShaderMapRef<FMyPassPS> PixelShader(View.ShaderMap);

    // 4. 混合/深度状态
    FRHIBlendState* BlendState =
        // 加法叠加（GI 贡献）：
        TStaticBlendState<CW_RGBA, BO_Add, BF_One, BF_One>::GetRHI();
        // 覆盖写入：
        // FScreenPassPipelineState::FDefaultBlendState::GetRHI();
    FRHIDepthStencilState* DepthStencilState =
        FScreenPassPipelineState::FDefaultDepthStencilState::GetRHI();

    // 5. AddDrawScreenPass（内部自动设置 PSO + DrawFullscreenTriangle）
    AddDrawScreenPass(
        GraphBuilder,
        RDG_EVENT_NAME("MyPlugin::MyPass %dx%d",
            OutputViewport.Rect.Width(), OutputViewport.Rect.Height()),
        View,
        OutputViewport,
        InputViewport,
        FScreenPassPipelineState(VertexShader, PixelShader, BlendState, DepthStencilState),
        PassParameters,
        EScreenPassDrawFlags::None,
        // Lambda 内仅设 Shader 参数（PSO 已由 AddDrawScreenPass 处理）
        [VertexShader, PixelShader, PassParameters](FRHICommandList& RHICmdList)
        {
            SetShaderParameters(RHICmdList, VertexShader,
                VertexShader.GetVertexShader(), PassParameters->Pass);
            SetShaderParameters(RHICmdList, PixelShader,
                PixelShader.GetPixelShader(), *PassParameters);
        });
}
```

---

## 4B. FPixelShaderUtils::AddFullscreenPass（最简单写法）

无自定义 VS、无自定义混合状态时使用，参数直接挂在 PS 上：

```cpp
// PS 参数结构体中直接含 RENDER_TARGET_BINDING_SLOTS()
TShaderMapRef<FMyPassPS> PixelShader(View.ShaderMap);

FMyPassPS::FParameters* PassParameters =
    GraphBuilder.AllocParameters<FMyPassPS::FParameters>();
PassParameters->RenderTargets[0] =
    FRenderTargetBinding(OutputTexture, ERenderTargetLoadAction::ELoad);
// ... 填其他参数 ...

FPixelShaderUtils::AddFullscreenPass(
    GraphBuilder,
    View.ShaderMap,
    RDG_EVENT_NAME("MyPlugin::MySimplePass"),
    PixelShader,
    PassParameters,
    OutputViewport.Rect);          // FIntRect，输出裁剪区域
```

---

## 4C. 自定义 VS+PS（Instanced Draw，带顶点缓冲）

适用于探针球体、Debug 点绘制等有几何的情形：

```cpp
void RenderMyInstancedPass_RenderThread(
    const FViewInfo& View,
    FRDGBuilder& GraphBuilder,
    FRDGTextureRef SceneColorTexture,
    FRHIBuffer* VertexBuffer,
    int32 VertexCount,
    int32 InstanceCount)
{
    // VS 和 PS 共享 FMyPassParameters（VS 用 using，PS 用 INCLUDE）
    FMyPassParameters* PassParameters =
        GraphBuilder.AllocParameters<FMyPassParameters>();
    // ... 填参数 ...

    // 需要额外分配带 RT 绑定的 PS 参数（将上面的内嵌进去）
    FMyPassPS::FParameters* PSParams =
        GraphBuilder.AllocParameters<FMyPassPS::FParameters>();
    PSParams->Pass = *PassParameters;
    PSParams->RenderTargets[0] =
        FRenderTargetBinding(SceneColorTexture, ERenderTargetLoadAction::ELoad);

    TShaderMapRef<FMyPassVS> VertexShader(View.ShaderMap);
    TShaderMapRef<FMyPassPS> PixelShader(View.ShaderMap);

    GraphBuilder.AddPass(
        RDG_EVENT_NAME("MyPlugin::InstancedDraw x%d", InstanceCount),
        PSParams,
        ERDGPassFlags::Raster | ERDGPassFlags::NeverCull,
        [&View, VertexShader, PixelShader, PSParams,
         VertexBuffer, VertexCount, InstanceCount]
        (FRHICommandList& RHICmdList)
        {
            RHICmdList.SetViewport(
                (float)View.ViewRect.Min.X, (float)View.ViewRect.Min.Y, 0.f,
                (float)View.ViewRect.Max.X, (float)View.ViewRect.Max.Y, 1.f);

            FGraphicsPipelineStateInitializer GraphicsPSOInit;
            RHICmdList.ApplyCachedRenderTargets(GraphicsPSOInit);
            GraphicsPSOInit.RasterizerState  = TStaticRasterizerState<>::GetRHI();
            GraphicsPSOInit.DepthStencilState =
                TStaticDepthStencilState<false, CF_Always>::GetRHI();
            GraphicsPSOInit.BlendState       =
                TStaticBlendState<CW_RGBA, BO_Add, BF_One, BF_One>::GetRHI();
            GraphicsPSOInit.BoundShaderState.VertexDeclarationRHI =
                GEmptyVertexDeclaration.VertexDeclarationRHI;
            GraphicsPSOInit.BoundShaderState.VertexShaderRHI =
                VertexShader.GetVertexShader();
            GraphicsPSOInit.BoundShaderState.PixelShaderRHI =
                PixelShader.GetPixelShader();
            GraphicsPSOInit.PrimitiveType = PT_TriangleList;
            SetGraphicsPipelineState(RHICmdList, GraphicsPSOInit, 0);

            SetShaderParameters(RHICmdList, VertexShader,
                VertexShader.GetVertexShader(), PSParams->Pass);
            SetShaderParameters(RHICmdList, PixelShader,
                PixelShader.GetPixelShader(), *PSParams);

            RHICmdList.SetStreamSource(0, VertexBuffer, 0);
            RHICmdList.DrawPrimitive(0, VertexCount / 3, InstanceCount);
        });
}
```

---

## 5. USF 模板（MyPass.usf）

```hlsl
// Copyright Epic Games, Inc. All Rights Reserved.

#include "/Engine/Public/Platform.ush"
#include "/Engine/Private/Common.ush"
#include "/Engine/Private/ScreenPass.ush"        // FScreenTransform, ViewportUVToBufferUV 等
#include "/Engine/Private/DeferredShadingCommon.ush"
#include "/Engine/Private/PositionReconstructionCommon.ush"

// 绑定变量（名称与 C++ FMyPassParameters 成员完全一致）
Texture2D<float4>  ColorTexture;
Texture2D          DepthTexture;
SamplerState       ColorSampler;
float4             ColorScale;
float              Intensity;

// ---- 全屏 Pixel Shader ----
void MainPS(
    noperspective float4 UVAndScreenPos : TEXCOORD0,   // AddDrawScreenPass 自动传入
    float4 SvPosition : SV_POSITION,
    out float4 OutColor : SV_Target0)
{
    float2 UV = UVAndScreenPos.xy;

    // 采样输入颜色
    float4 Color = ColorTexture.SampleLevel(ColorSampler, UV, 0);

    // 读取深度并重建世界坐标
    float DeviceZ    = DepthTexture.SampleLevel(ColorSampler, UV, 0).r;
    float SceneDepth = ConvertFromDeviceZ(DeviceZ);

    float2 PixelPos     = SvPosition.xy;
    float2 GBufferUV    = (PixelPos + 0.5f) * View.ViewSizeAndInvSize.zw;
    float4 ClipPos      = float4(GBufferUV * float2(2, -2) + float2(-1, 1), 0, 1);
    float3 ScreenVector = mul(float4(ClipPos.xy, 1, 0), View.ScreenToTranslatedWorld).xyz;
    float3 WorldPos     = ScreenVector * SceneDepth
                        + LWCHackToFloat(PrimaryView.WorldCameraOrigin);

    // 自定义逻辑
    OutColor = Color * ColorScale * Intensity;
    OutColor.rgb *= View.PreExposure;   // 叠加到 SceneColor 时需要乘 PreExposure
}

// ---- Instanced Vertex Shader ----
struct FVSOutput
{
    float4 Position : SV_POSITION;
    float2 UV       : TEXCOORD0;
};

float4x4 ViewProjectionMatrix;

FVSOutput MainVS(uint VertexId : SV_VertexID, uint InstanceId : SV_InstanceID)
{
    FVSOutput Out = (FVSOutput)0;
    // 示例：从 StructuredBuffer 读取每个实例位置
    // float3 WorldPos = InstanceDataBuffer[InstanceId].Position;
    Out.Position = mul(float4(0, 0, 0, 1), ViewProjectionMatrix);
    Out.UV       = float2(0, 0);
    return Out;
}
```

---

## 6. 关键点

| 项目 | 官方做法 | 说明 |
|------|---------|------|
| 全屏 Pass | `AddDrawScreenPass` | 自动建 PSO、设 Viewport、DrawFullscreenTriangle |
| 最简全屏 | `FPixelShaderUtils::AddFullscreenPass` | 无自定义 VS/BlendState 时最简洁 |
| PS 参数结构 | 内嵌结构 + `RENDER_TARGET_BINDING_SLOTS()` | RT 绑定必须在参数结构体里，不能单独设 |
| 参数组复用 | `SHADER_PARAMETER_STRUCT_INCLUDE` | 把公共参数拆成子结构，VS/PS 共用 |
| SceneTextures | `SHADER_PARAMETER_RDG_UNIFORM_BUFFER(FSceneTextureUniformParameters, SceneTexturesStruct)` | 官方后处理用 UB 方式，不是逐贴图绑定 |
| 混合状态 | `FScreenPassPipelineState::FDefaultBlendState` / `TStaticBlendState<...>` | 覆盖写用 Default；叠加 GI 用 `BO_Add, BF_One, BF_One` |
| PS Lambda | 只调 `SetShaderParameters`，PSO 已由外层处理 | 不在 Lambda 里调 `SetGraphicsPipelineState` |
| USF UV 来源 | `noperspective float4 UVAndScreenPos : TEXCOORD0` | `AddDrawScreenPass` 自动传入，直接用 `.xy` |
| PreExposure | 叠加到 SceneColor 前乘 `View.PreExposure` | 否则亮度尺度错误 |
